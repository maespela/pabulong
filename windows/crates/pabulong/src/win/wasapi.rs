//! WASAPI capture threads: the physical mic (event-driven, MMCSS "Pro
//! Audio") and loopback of the default speakers for the echo reference and
//! the remote side of recordings.
//!
//! Both deliver 48 kHz mono float with the QPC time (ns) of the first sample.
//! WASAPI converts the format for us (AUTOCONVERTPCM); if a driver refuses,
//! we open the mix format and convert in `crate::convert`.

use super::devices;
use crate::convert::{downmix, InputFormat, Resampler, SampleFormat};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::{GUID, HRESULT};
use windows::Win32::Foundation::{CloseHandle, E_ACCESSDENIED, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDevice, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR,
    AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    /// Settings › Privacy & security › Microphone blocks desktop apps.
    AccessDenied,
    /// Unplugged, disabled, or (loopback) the default speakers changed.
    DeviceGone,
    Other(i32),
}

impl From<windows::core::Error> for StreamError {
    fn from(e: windows::core::Error) -> Self {
        classify(e.code())
    }
}

fn classify(code: HRESULT) -> StreamError {
    if code == E_ACCESSDENIED {
        StreamError::AccessDenied
    } else if code == AUDCLNT_E_DEVICE_INVALIDATED {
        StreamError::DeviceGone
    } else {
        StreamError::Other(code.0)
    }
}

pub type OnAudio = Box<dyn FnMut(&[f32], u64) + Send>;

#[derive(Clone)]
enum Source {
    /// Endpoint id of a capture device.
    Capture(String),
    /// Loopback of the default render device.
    Loopback,
}

/// A running capture thread. Dropping it stops the thread.
pub struct Stream {
    stop: SendHandle,
    alive: Arc<AtomicBool>,
    thread: Option<JoinHandle<Option<StreamError>>>,
}

struct SendHandle(HANDLE);
// SAFETY: event handles may be signalled and closed from any thread.
unsafe impl Send for SendHandle {}
unsafe impl Sync for SendHandle {}

impl Stream {
    pub fn capture(device_id: String, on_audio: OnAudio) -> Result<Stream, StreamError> {
        Self::start(Source::Capture(device_id), on_audio)
    }

    pub fn loopback(on_audio: OnAudio) -> Result<Stream, StreamError> {
        Self::start(Source::Loopback, on_audio)
    }

    /// False once the thread has ended on its own (device lost etc.).
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn start(source: Source, on_audio: OnAudio) -> Result<Stream, StreamError> {
        let stop = unsafe { CreateEventW(None, true, false, None) }.map_err(StreamError::from)?;
        let stop = SendHandle(stop);
        let stop_for_thread = SendHandle(stop.0);
        let alive = Arc::new(AtomicBool::new(true));
        let alive_for_thread = alive.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let name = match source {
            Source::Capture(_) => "pabulong-mic",
            Source::Loopback => "pabulong-loopback",
        };
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                // Move the whole Send wrapper in (not just its HANDLE field).
                let stop = stop_for_thread;
                let result = run(source, on_audio, stop.0, ready_tx);
                alive_for_thread.store(false, Ordering::Release);
                result.err()
            })
            .map_err(|_| StreamError::Other(0))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Stream { stop, alive, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                unsafe {
                    let _ = CloseHandle(stop.0);
                }
                Err(e)
            }
            Err(_) => Err(StreamError::Other(0)),
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.stop.0);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        unsafe {
            let _ = CloseHandle(self.stop.0);
        }
    }
}

struct ComGuard;
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// What the stream delivers, and how to turn it into 48 kHz mono.
enum Decode {
    /// AUTOCONVERTPCM gave us exactly 48 kHz mono float.
    Native,
    Convert { format: InputFormat, resampler: Resampler, mono: Vec<f32> },
}

fn desired_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: 1,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 48_000 * 4,
        nBlockAlign: 4,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

unsafe fn input_format(mix: *const WAVEFORMATEX) -> InputFormat {
    let f = *mix;
    let tag = u32::from(f.wFormatTag);
    let bits = f.wBitsPerSample;
    let is_float = if tag == WAVE_FORMAT_EXTENSIBLE {
        let ext = *(mix as *const WAVEFORMATEXTENSIBLE);
        let sub: GUID = ext.SubFormat;
        sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
    } else {
        tag == WAVE_FORMAT_IEEE_FLOAT
    };
    let format = match (is_float, bits) {
        (true, _) => SampleFormat::F32,
        (false, 16) => SampleFormat::I16,
        (false, 24) => SampleFormat::I24,
        _ => SampleFormat::I32,
    };
    InputFormat { sample_rate: f.nSamplesPerSec, channels: f.nChannels, format }
}

/// Initializes `client` for shared-mode capture, preferring 48 kHz mono float.
unsafe fn initialize(client: &IAudioClient, flags: u32, buffer_hns: i64) -> windows::core::Result<Decode> {
    let wanted = desired_format();
    let auto = flags | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    if client.Initialize(AUDCLNT_SHAREMODE_SHARED, auto, buffer_hns, 0, &wanted, None).is_ok() {
        return Ok(Decode::Native);
    }
    let mix = client.GetMixFormat()?;
    let result = client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, buffer_hns, 0, mix, None);
    let format = input_format(mix);
    CoTaskMemFree(Some(mix as *const _));
    result?;
    Ok(Decode::Convert { format, resampler: Resampler::new(format.sample_rate), mono: Vec::new() })
}

/// Keeps the default speakers' audio engine running by playing silence, so
/// loopback delivers a continuous, evenly timestamped stream even while
/// nothing is playing (WASAPI loopback otherwise goes quiet, and gaps would
/// confuse the echo canceller's alignment).
struct SilentKeepAlive {
    client: IAudioClient,
    render: IAudioRenderClient,
    frames: u32,
}

impl SilentKeepAlive {
    unsafe fn start(device: &IMMDevice) -> windows::core::Result<Self> {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let mix = client.GetMixFormat()?;
        let result = client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 1_000_000, 0, mix, None);
        CoTaskMemFree(Some(mix as *const _));
        result?;
        let frames = client.GetBufferSize()?;
        let render: IAudioRenderClient = client.GetService()?;
        let keep = Self { client, render, frames };
        keep.fill();
        keep.client.Start()?;
        Ok(keep)
    }

    unsafe fn fill(&self) {
        let padding = self.client.GetCurrentPadding().unwrap_or(self.frames);
        let free = self.frames.saturating_sub(padding);
        if free > 0 && self.render.GetBuffer(free).is_ok() {
            let _ = self.render.ReleaseBuffer(free, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
        }
    }
}

impl Drop for SilentKeepAlive {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

fn run(source: Source, mut on_audio: OnAudio, stop: HANDLE, ready: mpsc::SyncSender<Result<(), StreamError>>) -> Result<(), StreamError> {
    let setup = || -> Result<_, StreamError> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        let com = ComGuard;
        let e = devices::enumerator()?;
        let (device, loopback) = unsafe {
            match &source {
                Source::Capture(id) => (e.GetDevice(&super::h(id))?, false),
                Source::Loopback => (
                    e.GetDefaultAudioEndpoint(windows::Win32::Media::Audio::eRender, windows::Win32::Media::Audio::eConsole)?,
                    true,
                ),
            }
        };
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let (flags, buffer) = if loopback { (AUDCLNT_STREAMFLAGS_LOOPBACK, 2_000_000) } else { (AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 1_000_000) };
        let decode = unsafe { initialize(&client, flags, buffer)? };
        let event = if loopback {
            None
        } else {
            let ev = unsafe { CreateEventW(None, false, false, None)? };
            unsafe { client.SetEventHandle(ev)? };
            Some(ev)
        };
        let capture: IAudioCaptureClient = unsafe { client.GetService()? };
        let keep_alive = if loopback { unsafe { SilentKeepAlive::start(&device).ok() } } else { None };
        let render_id = if loopback { devices::default_render_id(&e) } else { None };
        unsafe { client.Start()? };
        Ok((com, e, client, capture, decode, event, keep_alive, render_id, loopback))
    };
    let (_com, enumerator, client, capture, mut decode, event, keep_alive, render_id, loopback) = match setup() {
        Ok(v) => {
            let _ = ready.send(Ok(()));
            v
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return Err(e);
        }
    };

    // Low-latency scheduling for the mic thread.
    let mut task_index = 0u32;
    let mmcss = if loopback { None } else { unsafe { AvSetMmThreadCharacteristicsW(&super::h("Pro Audio"), &mut task_index).ok() } };

    let mut native = Vec::<f32>::with_capacity(4800);
    let mut out = Vec::<f32>::with_capacity(4800);
    let mut last_device_check = Instant::now();
    let result = loop {
        let woke_for_stop = unsafe {
            match event {
                Some(ev) => WaitForMultipleObjects(&[stop, ev], false, 200) == WAIT_OBJECT_0,
                None => WaitForSingleObject(stop, 10) == WAIT_OBJECT_0,
            }
        };
        if woke_for_stop {
            break Ok(());
        }
        if let Some(k) = &keep_alive {
            unsafe { k.fill() };
        }
        if let Err(e) = drain(&capture, &mut decode, &mut native, &mut out, &mut on_audio) {
            break Err(e);
        }
        // Follow the default speakers: if they change, end so the owner
        // restarts loopback on the new device.
        if loopback && last_device_check.elapsed() > Duration::from_secs(2) {
            last_device_check = Instant::now();
            if devices::default_render_id(&enumerator) != render_id {
                break Err(StreamError::DeviceGone);
            }
        }
    };

    unsafe {
        let _ = client.Stop();
        if let Some(ev) = event {
            let _ = CloseHandle(ev);
        }
        if let Some(h) = mmcss {
            let _ = AvRevertMmThreadCharacteristics(h);
        }
    }
    drop(keep_alive);
    result
}

fn drain(capture: &IAudioCaptureClient, decode: &mut Decode, native: &mut Vec<f32>, out: &mut Vec<f32>, on_audio: &mut OnAudio) -> Result<(), StreamError> {
    loop {
        let packet = unsafe { capture.GetNextPacketSize()? };
        if packet == 0 {
            return Ok(());
        }
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut frames = 0u32;
        let mut flags = 0u32;
        let mut qpc_100ns = 0u64;
        unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc_100ns))? };
        let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
        let n = frames as usize;
        // Time of the first frame on the QPC clock, in ns.
        let first_ns = if qpc_100ns == 0 || flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
            super::qpc_now_ns().saturating_sub(n as u64 * 1_000_000_000 / 48_000)
        } else {
            qpc_100ns * 100
        };
        out.clear();
        match decode {
            Decode::Native => {
                if silent || data.is_null() {
                    out.resize(n, 0.0);
                } else {
                    out.extend_from_slice(unsafe { std::slice::from_raw_parts(data as *const f32, n) });
                }
            }
            Decode::Convert { format, resampler, mono } => {
                let bytes = n * usize::from(format.channels.max(1)) * format.format.bytes();
                if silent || data.is_null() {
                    mono.clear();
                    mono.resize(n, 0.0);
                } else {
                    downmix(unsafe { std::slice::from_raw_parts(data, bytes) }, *format, mono);
                }
                native.clear();
                native.extend_from_slice(mono);
                resampler.process(native.as_slice(), out);
            }
        }
        unsafe { capture.ReleaseBuffer(frames)? };
        if !out.is_empty() {
            on_audio(out.as_slice(), first_ns);
        }
    }
}
