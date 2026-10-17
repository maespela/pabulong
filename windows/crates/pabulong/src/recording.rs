//! Local call recording, the Windows port of `CallRecorder`,
//! `CallSessionManager` and `CallMetadataStore`.
//!
//! Each call gets a folder `Calls\<id>\` holding `local-mic.wav` (your
//! processed mic), `remote-system.wav` (what the speakers played, from WASAPI
//! loopback) and, once the call ends, `mixed-preview.wav`. All three are
//! 16 kHz 16-bit mono PCM, the same files the macOS app writes, plus a
//! `call.json` with the session metadata. Nothing ever leaves the PC.

use crate::json::Value;
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

pub const STORAGE_SAMPLE_RATE: u32 = 16_000;
pub const LOCAL_FILE: &str = "local-mic.wav";
pub const REMOTE_FILE: &str = "remote-system.wav";
pub const MIXED_FILE: &str = "mixed-preview.wav";
pub const METADATA_FILE: &str = "call.json";

// ------------------------------------------------------------------- WAV

/// 16-bit PCM mono WAV writer that patches the header sizes on `finish`.
pub struct WavWriter {
    out: BufWriter<File>,
    samples: u32,
}

impl WavWriter {
    pub fn create(path: &Path, sample_rate: u32) -> io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(&wav_header(sample_rate, 0))?;
        Ok(Self { out, samples: 0 })
    }

    pub fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            let v = (s.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        self.out.write_all(&bytes)?;
        self.samples = self.samples.saturating_add(samples.len() as u32);
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<u32> {
        self.out.flush()?;
        let mut file = self.out.into_inner().map_err(|e| e.into_error())?;
        let sample_rate = STORAGE_SAMPLE_RATE;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&wav_header(sample_rate, self.samples))?;
        file.sync_all()?;
        Ok(self.samples)
    }
}

fn wav_header(sample_rate: u32, samples: u32) -> [u8; 44] {
    let data = samples.saturating_mul(2);
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&data.saturating_add(36).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&(sample_rate * 2).to_le_bytes());
    h[32..34].copy_from_slice(&2u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

/// Reads a file written by [`WavWriter`] back as floats.
pub fn read_wav_16bit_mono(path: &Path) -> io::Result<Vec<f32>> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a WAV file"));
    }
    Ok(bytes[44..].chunks_exact(2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0).collect())
}

// -------------------------------------------------------------- decimator

/// 48 kHz → 16 kHz: windowed-sinc low-pass (7.2 kHz) then keep every third
/// sample. Keeps its filter state across calls.
pub struct Decimator {
    taps: Vec<f32>,
    history: Vec<f32>,
    phase: usize,
}

impl Default for Decimator {
    fn default() -> Self {
        const N: usize = 63;
        let cutoff = 7_200.0 / 48_000.0;
        let mid = (N - 1) as f64 / 2.0;
        let mut taps: Vec<f32> = (0..N)
            .map(|i| {
                let x = i as f64 - mid;
                let sinc = if x == 0.0 { 2.0 * cutoff } else { (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x) };
                let w = 0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / (N - 1) as f64).cos();
                (sinc * w) as f32
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        Self { history: vec![0.0; N], taps, phase: 0 }
    }
}

impl Decimator {
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let n = self.taps.len();
        for &s in input {
            self.history.copy_within(1.., 0);
            self.history[n - 1] = s;
            self.phase += 1;
            if self.phase == 3 {
                self.phase = 0;
                out.push(self.history.iter().zip(&self.taps).map(|(h, t)| h * t).sum());
            }
        }
    }
}

// ------------------------------------------------------------- metadata

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallStatus {
    Recording,
    Finalizing,
    Completed,
    Failed,
}

impl CallStatus {
    fn raw(self) -> &'static str {
        match self {
            CallStatus::Recording => "recording",
            CallStatus::Finalizing => "finalizing",
            CallStatus::Completed => "completed",
            CallStatus::Failed => "failed",
        }
    }

    fn from_raw(raw: &str) -> Self {
        match raw {
            "recording" => CallStatus::Recording,
            "finalizing" => CallStatus::Finalizing,
            "completed" => CallStatus::Completed,
            _ => CallStatus::Failed,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CallSession {
    pub id: String,
    pub title: String,
    /// Unix seconds.
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub status: CallStatus,
    pub app_name: Option<String>,
    pub directory: PathBuf,
    pub remote_audio_missing: bool,
}

impl CallSession {
    pub fn local_path(&self) -> PathBuf {
        self.directory.join(LOCAL_FILE)
    }
    pub fn remote_path(&self) -> PathBuf {
        self.directory.join(REMOTE_FILE)
    }
    pub fn mixed_path(&self) -> PathBuf {
        self.directory.join(MIXED_FILE)
    }

    pub fn duration_seconds(&self, now: f64) -> f64 {
        (self.ended_at.unwrap_or(now) - self.started_at).max(0.0)
    }

    fn to_json(&self) -> Value {
        let path = |p: PathBuf| p.to_string_lossy().into_owned();
        Value::object()
            .with("id", self.id.as_str())
            .with("title", self.title.as_str())
            .with("startedAt", self.started_at)
            .with("endedAt", self.ended_at)
            .with("status", self.status.raw())
            .with("appName", self.app_name.clone())
            .with("localMicAudioPath", path(self.local_path()))
            .with("remoteSystemAudioPath", path(self.remote_path()))
            .with("mixedPreviewAudioPath", path(self.mixed_path()))
            .with("remoteAudioMissing", self.remote_audio_missing)
    }

    fn from_json(v: &Value, directory: PathBuf) -> Option<Self> {
        Some(CallSession {
            id: v.get("id")?.as_str()?.to_string(),
            title: v.get("title")?.as_str()?.to_string(),
            started_at: v.get("startedAt")?.as_f64()?,
            ended_at: v.get("endedAt").and_then(Value::as_f64),
            status: CallStatus::from_raw(v.get("status").and_then(Value::as_str).unwrap_or("failed")),
            app_name: v.get("appName").and_then(Value::as_str).map(str::to_string),
            directory,
            remote_audio_missing: v.get("remoteAudioMissing").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

pub fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Random-enough 128-bit id in UUID form, without a dependency.
pub fn new_call_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut words = [0u64; 2];
    for (i, w) in words.iter_mut().enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u32(std::process::id());
        h.write_usize(i);
        *w = h.finish();
    }
    let mut b: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    b[6] = (b[6] & 0x0F) | 0x40; // version 4
    b[8] = (b[8] & 0x3F) | 0x80; // RFC 4122 variant
    let hex: String = b.iter().map(|x| format!("{x:02X}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

/// `CallMetadataStore.cleanTitle`: collapse whitespace; empty → None.
pub fn clean_title(title: Option<&str>) -> Option<String> {
    let collapsed = title?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// `CallMetadataStore.fallbackTitle`, with the timestamp already formatted
/// in the user's locale by the caller.
pub fn fallback_title(app_name: Option<&str>, timestamp: &str) -> String {
    match clean_title(app_name) {
        Some(app) => format!("{app} Call {timestamp}"),
        None => format!("Call {timestamp}"),
    }
}

pub struct CallStore {
    calls_dir: PathBuf,
}

impl CallStore {
    pub fn new(calls_dir: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&calls_dir)?;
        Ok(Self { calls_dir })
    }

    pub fn calls_dir(&self) -> &Path {
        &self.calls_dir
    }

    pub fn create(&self, title: String, app_name: Option<String>) -> io::Result<CallSession> {
        let id = new_call_id();
        let directory = self.calls_dir.join(&id);
        std::fs::create_dir_all(&directory)?;
        let session = CallSession {
            id,
            title,
            started_at: unix_now(),
            ended_at: None,
            status: CallStatus::Recording,
            app_name,
            directory,
            remote_audio_missing: false,
        };
        self.save(&session)?;
        Ok(session)
    }

    pub fn save(&self, session: &CallSession) -> io::Result<()> {
        let path = session.directory.join(METADATA_FILE);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, session.to_json().to_pretty())?;
        std::fs::rename(tmp, path)
    }

    /// Newest first.
    pub fn list(&self) -> Vec<CallSession> {
        let mut calls: Vec<CallSession> = std::fs::read_dir(&self.calls_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let dir = entry.path();
                let text = std::fs::read_to_string(dir.join(METADATA_FILE)).ok()?;
                CallSession::from_json(&Value::parse(&text).ok()?, dir)
            })
            .collect();
        calls.sort_by(|a, b| b.started_at.total_cmp(&a.started_at));
        calls
    }

    pub fn delete(&self, session: &CallSession) -> io::Result<()> {
        // Only ever delete a folder inside our own Calls directory.
        if session.directory.parent() != Some(self.calls_dir.as_path()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a call folder"));
        }
        std::fs::remove_dir_all(&session.directory)
    }
}

// --------------------------------------------------------------- recorder

enum Msg {
    Local(Vec<f32>),
    Remote(Vec<f32>),
    Stop,
}

/// Records one call on its own writer thread so the audio threads never touch
/// the disk. Feed it 48 kHz mono; it stores 16 kHz.
pub struct CallRecorder {
    tx: Sender<Msg>,
    worker: Option<JoinHandle<io::Result<bool>>>,
}

impl CallRecorder {
    pub fn start(session: &CallSession, capture_remote: bool) -> io::Result<Self> {
        let local = WavWriter::create(&session.local_path(), STORAGE_SAMPLE_RATE)?;
        let remote = if capture_remote { Some(WavWriter::create(&session.remote_path(), STORAGE_SAMPLE_RATE)?) } else { None };
        let (tx, rx) = mpsc::channel();
        let local_path = session.local_path();
        let remote_path = session.remote_path();
        let mixed_path = session.mixed_path();
        let worker = std::thread::Builder::new()
            .name("pabulong-recorder".into())
            .spawn(move || Self::run(rx, local, remote, &local_path, &remote_path, &mixed_path))?;
        Ok(Self { tx, worker: Some(worker) })
    }

    /// Your processed mic, one 480-sample 48 kHz frame at a time.
    pub fn record_local(&self, frame: &[f32]) {
        let _ = self.tx.send(Msg::Local(frame.to_vec()));
    }

    /// Speaker output (48 kHz mono) from the loopback capture.
    pub fn record_remote(&self, samples: &[f32]) {
        let _ = self.tx.send(Msg::Remote(samples.to_vec()));
    }

    /// Closes the files and builds the mixed preview. Returns whether any
    /// remote audio was recorded.
    pub fn stop(mut self) -> io::Result<bool> {
        let _ = self.tx.send(Msg::Stop);
        match self.worker.take() {
            Some(worker) => worker.join().unwrap_or_else(|_| Err(io::Error::other("recorder thread panicked"))),
            None => Ok(false),
        }
    }

    fn run(
        rx: Receiver<Msg>,
        mut local: WavWriter,
        mut remote: Option<WavWriter>,
        local_path: &Path,
        remote_path: &Path,
        mixed_path: &Path,
    ) -> io::Result<bool> {
        let mut local_dec = Decimator::default();
        let mut remote_dec = Decimator::default();
        let mut scratch = Vec::with_capacity(512);
        let mut local_ok = true;
        // Remote audio only counts once something has played.
        let mut remote_heard = false;
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Local(frame) => {
                    if local_ok {
                        scratch.clear();
                        local_dec.process(&frame, &mut scratch);
                        // A write error stops the local track (like macOS).
                        local_ok = local.write(&scratch).is_ok();
                    }
                }
                Msg::Remote(samples) => {
                    if let Some(w) = remote.as_mut() {
                        remote_heard |= samples.iter().any(|s| *s != 0.0);
                        scratch.clear();
                        remote_dec.process(&samples, &mut scratch);
                        if w.write(&scratch).is_err() {
                            remote = None;
                        }
                    }
                }
                Msg::Stop => break,
            }
        }
        local.finish()?;
        let has_remote = match remote {
            Some(w) => w.finish()? > 0 && remote_heard,
            None => false,
        };
        build_mixed_preview(local_path, has_remote.then_some(remote_path), mixed_path)?;
        Ok(has_remote)
    }
}

/// `buildMixedPreviewUnlocked` on macOS: local × 0.8 + remote × 0.8, clipped
/// to ±0.98; just a copy of the local track when there is no remote audio.
pub fn build_mixed_preview(local: &Path, remote: Option<&Path>, mixed: &Path) -> io::Result<()> {
    let Some(remote) = remote else {
        std::fs::copy(local, mixed)?;
        return Ok(());
    };
    let a = read_wav_16bit_mono(local)?;
    let b = read_wav_16bit_mono(remote)?;
    let mut w = WavWriter::create(mixed, STORAGE_SAMPLE_RATE)?;
    let n = a.len().max(b.len());
    let mut chunk = Vec::with_capacity(4096);
    for start in (0..n).step_by(4096) {
        chunk.clear();
        for i in start..(start + 4096).min(n) {
            let l = a.get(i).copied().unwrap_or(0.0);
            let r = b.get(i).copied().unwrap_or(0.0);
            chunk.push((l * 0.8 + r * 0.8).clamp(-0.98, 0.98));
        }
        w.write(&chunk)?;
    }
    w.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pabulong-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn wav_round_trip_and_header() {
        let dir = temp("wav");
        let path = dir.join("a.wav");
        let mut w = WavWriter::create(&path, 16_000).unwrap();
        w.write(&[0.0, 0.5, -0.5, 1.5]).unwrap();
        assert_eq!(w.finish().unwrap(), 4);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + 8);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 16_000);
        let back = read_wav_16bit_mono(&path).unwrap();
        assert!((back[1] - 0.5).abs() < 1e-3 && (back[3] - 1.0).abs() < 1e-3);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn decimator_keeps_speech_and_removes_what_16k_cannot_hold() {
        let tone = |hz: f64| -> f32 {
            let input: Vec<f32> = (0..48_000).map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / 48_000.0).sin() as f32).collect();
            let mut d = Decimator::default();
            let mut out = Vec::new();
            d.process(&input, &mut out);
            assert_eq!(out.len(), 16_000);
            noiseless_core::rms(&out[1000..])
        };
        assert!(tone(1_000.0) > 0.68, "1 kHz must pass");
        assert!(tone(12_000.0) < 0.01, "12 kHz must not alias into the file");
    }

    #[test]
    fn records_a_call_and_mixes_it() {
        let dir = temp("calls");
        let store = CallStore::new(dir.join("Calls")).unwrap();
        let mut session = store.create(fallback_title(Some("  Zoom  "), "Oct 17, 2026 at 3:04 PM"), None).unwrap();
        assert_eq!(session.title, "Zoom Call Oct 17, 2026 at 3:04 PM");
        let rec = CallRecorder::start(&session, true).unwrap();
        for _ in 0..100 {
            rec.record_local(&[0.25; 480]);
            rec.record_remote(&[0.25; 512]);
        }
        assert!(rec.stop().unwrap());
        session.status = CallStatus::Completed;
        session.ended_at = Some(session.started_at + 1.0);
        store.save(&session).unwrap();

        let local = read_wav_16bit_mono(&session.local_path()).unwrap();
        let mixed = read_wav_16bit_mono(&session.mixed_path()).unwrap();
        assert_eq!(local.len(), 100 * 160);
        assert!((mixed[8000] - 0.4).abs() < 0.01, "0.25*0.8*2 = 0.4, got {}", mixed[8000]);
        let listed = store.list();
        assert_eq!(listed, vec![session.clone()]);
        store.delete(&session).unwrap();
        assert!(store.list().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn silent_remote_means_mixed_is_the_local_track() {
        let dir = temp("silent");
        let store = CallStore::new(dir.clone()).unwrap();
        let session = store.create("Call".into(), None).unwrap();
        let rec = CallRecorder::start(&session, true).unwrap();
        rec.record_local(&[0.1; 480]);
        rec.record_remote(&[0.0; 480]);
        assert!(!rec.stop().unwrap());
        assert_eq!(std::fs::read(session.local_path()).unwrap(), std::fs::read(session.mixed_path()).unwrap());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn ids_look_like_uuids_and_differ() {
        let a = new_call_id();
        let b = new_call_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(a.matches('-').count(), 4);
    }
}
