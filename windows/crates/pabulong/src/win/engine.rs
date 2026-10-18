//! `Platform` for Windows: owns the driver connection, the capture threads,
//! the shared chain and the call recorder.

use super::devices::{self, ResolveError};
use super::driver::DriverConnection;
use super::wasapi::{Stream, StreamError};
use crate::chain::{ChainMetrics, VoiceChain};
use crate::controller::{Platform, RingOpenError, StartError};
use crate::mic_path::MicPath;
use crate::recording::{fallback_title, CallRecorder, CallSession, CallStatus, CallStore};
use crate::settings::AppSettings;
use pabulong_ring::RingHealth;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type SharedRecorder = Arc<Mutex<Option<CallRecorder>>>;

struct Running {
    chain: Arc<VoiceChain>,
    mic: Stream,
    loopback: Option<Stream>,
    last_loopback_attempt: Instant,
    device: Option<String>,
}

pub struct WinPlatform {
    driver: Option<DriverConnection>,
    store: Option<CallStore>,
    running: Option<Running>,
    recorder: SharedRecorder,
    session: Option<CallSession>,
    bypass: Arc<AtomicBool>,
    /// Push speaker audio to the echo canceller.
    echo_on: Arc<AtomicBool>,
    /// Record speaker audio as the remote track.
    record_remote: Arc<AtomicBool>,
    last_driver_attempt: Option<Instant>,
}

impl WinPlatform {
    pub fn new(store: Option<CallStore>) -> Self {
        Self {
            driver: None,
            store,
            running: None,
            recorder: Arc::new(Mutex::new(None)),
            session: None,
            bypass: Arc::new(AtomicBool::new(false)),
            echo_on: Arc::new(AtomicBool::new(true)),
            record_remote: Arc::new(AtomicBool::new(false)),
            last_driver_attempt: None,
        }
    }

    pub fn store(&self) -> Option<&CallStore> {
        self.store.as_ref()
    }

    fn loopback_wanted(&self) -> bool {
        self.echo_on.load(Ordering::Relaxed) || self.record_remote.load(Ordering::Relaxed)
    }

    fn start_loopback(&self, chain: &Arc<VoiceChain>) -> Option<Stream> {
        let chain = chain.clone();
        let echo_on = self.echo_on.clone();
        let record_remote = self.record_remote.clone();
        let recorder = self.recorder.clone();
        Stream::loopback(Box::new(move |samples, ns| {
            if echo_on.load(Ordering::Relaxed) {
                chain.push_reference(samples, ns);
            }
            if record_remote.load(Ordering::Relaxed) {
                if let Ok(guard) = recorder.try_lock() {
                    if let Some(r) = guard.as_ref() {
                        r.record_remote(samples);
                    }
                }
            }
        }))
        .ok()
    }

    fn ensure_loopback(&mut self) {
        let wanted = self.loopback_wanted();
        let Some(chain) = self.running.as_ref().map(|r| r.chain.clone()) else { return };
        let needs_start = {
            let r = self.running.as_mut().expect("checked");
            if !wanted {
                r.loopback = None;
                false
            } else {
                let dead = r.loopback.as_ref().is_none_or(|l| !l.is_alive());
                if dead {
                    r.loopback = None;
                }
                dead && r.last_loopback_attempt.elapsed() > Duration::from_secs(1)
            }
        };
        if needs_start {
            let stream = self.start_loopback(&chain);
            let r = self.running.as_mut().expect("checked");
            r.last_loopback_attempt = Instant::now();
            r.loopback = stream;
        }
    }
}

impl Platform for WinPlatform {
    fn poll(&mut self) {
        // A lost mic (unplugged, disabled) ends the pipeline; the controller
        // starts it again on the next tick if a call is still reading.
        if self.running.as_ref().is_some_and(|r| !r.mic.is_alive()) {
            self.stop_pipeline();
        }
        self.ensure_loopback();
        // A driver mapping can disappear if the device is disabled.
        if self.driver.as_ref().is_some_and(|d| d.info().is_none()) {
            self.stop_pipeline();
            self.driver = None;
        }
    }

    fn open_ring(&mut self) -> Result<(), RingOpenError> {
        if self.driver.is_some() {
            return Ok(());
        }
        // Don't hammer CreateFile every 50 ms while the driver is missing.
        if self.last_driver_attempt.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return Err(RingOpenError::DriverMissing);
        }
        self.last_driver_attempt = Some(Instant::now());
        self.driver = Some(DriverConnection::open()?);
        Ok(())
    }

    fn ring_health(&self) -> Option<RingHealth> {
        self.driver.as_ref().map(|d| d.ring().health())
    }

    fn start_pipeline(&mut self, settings: &AppSettings, bypass: bool) -> Result<(), StartError> {
        let ring = self.driver.as_ref().ok_or(StartError::Other)?.ring();
        let device = devices::resolve_physical_input(settings.selected_input_device_uid.as_deref()).map_err(|e| match e {
            ResolveError::IsVirtual => StartError::WouldCaptureVirtualDevice,
            ResolveError::NoDevice => StartError::InputDeviceUnavailable,
        })?;
        let id = devices::device_id(&device).ok_or(StartError::InputDeviceUnavailable)?;
        drop(device);

        let chain = Arc::new(VoiceChain::new(settings.suppression_mode, settings.echo_cancellation_enabled, settings.voice_isolation));
        self.bypass.store(bypass, Ordering::Relaxed);
        self.echo_on.store(settings.echo_cancellation_enabled, Ordering::Relaxed);
        let mut path = MicPath::new(chain.clone(), self.bypass.clone());
        let recorder = self.recorder.clone();
        let mic = Stream::capture(
            id,
            Box::new(move |samples, ns| {
                path.push(samples, ns, |frame| {
                    ring.write_frame(frame);
                    if let Ok(guard) = recorder.try_lock() {
                        if let Some(r) = guard.as_ref() {
                            r.record_local(frame);
                        }
                    }
                });
            }),
        )
        .map_err(|e| match e {
            StreamError::AccessDenied => StartError::AccessDenied,
            StreamError::DeviceGone => StartError::InputDeviceUnavailable,
            StreamError::Other(_) => StartError::Other,
        })?;
        ring.set_producer_active(true);
        self.running = Some(Running {
            chain,
            mic,
            loopback: None,
            last_loopback_attempt: Instant::now() - Duration::from_secs(10),
            device: settings.selected_input_device_uid.clone(),
        });
        self.ensure_loopback();
        Ok(())
    }

    fn stop_pipeline(&mut self) {
        if let Some(d) = &self.driver {
            d.ring().set_producer_active(false);
        }
        // Dropping the streams stops and joins their threads.
        self.running = None;
    }

    fn pipeline_running(&self) -> bool {
        self.running.is_some()
    }

    fn pipeline_device(&self) -> Option<Option<String>> {
        self.running.as_ref().map(|r| r.device.clone())
    }

    fn set_bypass(&mut self, bypass: bool) {
        self.bypass.store(bypass, Ordering::Relaxed);
    }

    fn apply_chain_settings(&mut self, s: &AppSettings) {
        self.echo_on.store(s.echo_cancellation_enabled, Ordering::Relaxed);
        if let Some(r) = &self.running {
            r.chain.set_mode(s.suppression_mode);
            r.chain.set_voice_isolation(s.voice_isolation);
            r.chain.set_echo_cancellation(s.echo_cancellation_enabled);
        }
        self.ensure_loopback();
    }

    fn metrics(&self) -> Option<ChainMetrics> {
        self.running.as_ref().map(|r| r.chain.metrics())
    }

    fn engine_name(&self) -> Option<&'static str> {
        self.running.as_ref().map(|r| r.chain.engine_name())
    }

    fn echo_reference_active(&self) -> bool {
        self.echo_on.load(Ordering::Relaxed) && self.running.as_ref().is_some_and(|r| r.loopback.as_ref().is_some_and(Stream::is_alive))
    }

    fn start_recording(&mut self, settings: &AppSettings) -> std::io::Result<CallSession> {
        let store = self.store.as_ref().ok_or_else(|| std::io::Error::other("call storage unavailable"))?;
        let session = store.create(fallback_title(None, &super::shell::local_timestamp()), None)?;
        let recorder = CallRecorder::start(&session, settings.capture_system_audio_enabled)?;
        *self.recorder.lock().unwrap_or_else(|e| e.into_inner()) = Some(recorder);
        self.record_remote.store(settings.capture_system_audio_enabled, Ordering::Relaxed);
        self.session = Some(session.clone());
        self.ensure_loopback();
        Ok(session)
    }

    fn stop_recording(&mut self) -> Option<CallSession> {
        self.record_remote.store(false, Ordering::Relaxed);
        let recorder = self.recorder.lock().unwrap_or_else(|e| e.into_inner()).take();
        let mut session = self.session.take()?;
        self.ensure_loopback();
        session.ended_at = Some(crate::recording::unix_now());
        session.status = CallStatus::Finalizing;
        let store_dir = self.store.as_ref().map(|s| s.calls_dir().to_path_buf());
        let finished = session.clone();
        // Closing the files and mixing the preview can take a moment for a
        // long call; do it off the UI thread.
        std::thread::spawn(move || {
            let has_remote = recorder.map(CallRecorder::stop);
            let mut s = finished;
            s.status = match has_remote {
                Some(Ok(_)) => CallStatus::Completed,
                _ => CallStatus::Failed,
            };
            s.remote_audio_missing = !matches!(has_remote, Some(Ok(true)));
            if let Some(dir) = store_dir {
                if let Ok(store) = CallStore::new(dir) {
                    let _ = store.save(&s);
                }
            }
        });
        Some(session)
    }
}
