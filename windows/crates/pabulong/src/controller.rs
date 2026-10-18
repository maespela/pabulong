//! What the app does and says, independent of Win32 — a port of
//! `MenuBarModel` in `Sources/NoiselessApp/main.swift`.
//!
//! The physical mic is captured only while a call app is reading Pabulong
//! Mic (the driver's `consumer_running_count` > 0). When the call ends the
//! pipeline stops after a 1 s debounce. With Mic Processing off, a call that
//! uses Pabulong Mic still gets your voice, unprocessed ("Pass Through").

use crate::chain::ChainMetrics;
use crate::recording::CallSession;
use crate::settings::{AppSettings, VoiceIsolation};
use crate::text;
use pabulong_ring::RingHealth;
use std::time::{Duration, Instant};

pub const PIPELINE_STOP_DEBOUNCE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartError {
    /// No physical capture endpoint (or the chosen one is gone).
    InputDeviceUnavailable,
    /// The chosen input is Pabulong Mic itself (would feed back).
    WouldCaptureVirtualDevice,
    /// Windows privacy settings block desktop apps from the mic.
    AccessDenied,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingOpenError {
    /// The Pabulong Mic driver is not installed or not started.
    DriverMissing,
    /// Another process already feeds Pabulong Mic.
    Busy,
}

/// The Windows side: audio threads, the driver connection and recording.
pub trait Platform {
    /// Housekeeping at the start of every tick: notice a lost mic, restart
    /// loopback when the default speakers change.
    fn poll(&mut self) {}
    /// Connects to the driver's ring if not connected yet.
    fn open_ring(&mut self) -> Result<(), RingOpenError>;
    fn ring_health(&self) -> Option<RingHealth>;
    fn start_pipeline(&mut self, settings: &AppSettings, bypass: bool) -> Result<(), StartError>;
    fn stop_pipeline(&mut self);
    fn pipeline_running(&self) -> bool;
    /// The chosen input device the running pipeline captures (to detect changes).
    fn pipeline_device(&self) -> Option<Option<String>>;
    fn set_bypass(&mut self, bypass: bool);
    fn apply_chain_settings(&mut self, settings: &AppSettings);
    fn metrics(&self) -> Option<ChainMetrics>;
    fn engine_name(&self) -> Option<&'static str>;
    fn echo_reference_active(&self) -> bool;
    fn start_recording(&mut self, settings: &AppSettings) -> std::io::Result<CallSession>;
    fn stop_recording(&mut self) -> Option<CallSession>;
}

pub struct Controller {
    pub settings: AppSettings,
    pub pipeline_status: &'static str,
    pub mic_access_denied: bool,
    pub ring: RingHealth,
    pub metrics: ChainMetrics,
    pub echo_reference_active: bool,
    pub engine_name: &'static str,
    pub active_call: Option<CallSession>,
    pub call_status_message: &'static str,
    pending_stop_at: Option<Instant>,
    last_reported: Option<(u32, bool)>,
}

impl Controller {
    pub fn new(settings: AppSettings) -> Self {
        Self {
            settings,
            pipeline_status: text::STATUS_STOPPED,
            mic_access_denied: false,
            ring: RingHealth::default(),
            metrics: ChainMetrics::default(),
            echo_reference_active: false,
            engine_name: crate::chain::default_engine_name(),
            active_call: None,
            call_status_message: "",
            pending_stop_at: None,
            last_reported: None,
        }
    }

    pub fn enabled(&self) -> bool {
        self.settings.processing_enabled
    }

    fn idle_status(&self) -> &'static str {
        if self.enabled() {
            text::STATUS_WAITING_FOR_CALL
        } else {
            text::OFF
        }
    }

    // ------------------------------------------------------------ state

    pub fn is_call_active(&self) -> bool {
        self.ring.consumer_running_count > 0
    }

    pub fn is_virtual_mic_in_use(&self, p: &dyn Platform) -> bool {
        self.is_call_active() || self.pending_stop_at.is_some() || p.pipeline_running()
    }

    pub fn can_start_manual_recording(&self, p: &dyn Platform) -> bool {
        self.enabled() && self.is_virtual_mic_in_use(p) && self.active_call.is_none() && self.settings.local_recording_enabled
    }

    fn bypass(&self) -> bool {
        self.settings.bypass_enabled || !self.enabled()
    }

    // ---------------------------------------------------------- actions

    /// Launch, and whenever the driver connection may have come back.
    pub fn start_monitoring(&mut self, p: &mut dyn Platform) {
        match p.open_ring() {
            Ok(()) => {
                if !p.pipeline_running() {
                    self.pipeline_status = self.idle_status();
                }
            }
            Err(RingOpenError::DriverMissing) => self.pipeline_status = text::STATUS_VIRTUAL_MIC_UNAVAILABLE,
            Err(RingOpenError::Busy) => self.pipeline_status = text::STATUS_ALREADY_RUNNING,
        }
    }

    pub fn set_processing_enabled(&mut self, on: bool, p: &mut dyn Platform) {
        self.settings.processing_enabled = on;
        self.start_monitoring(p);
        p.set_bypass(self.bypass());
        if self.is_call_active() {
            self.start_pipeline_if_needed(p);
        } else if !on {
            self.schedule_stop_if_idle(p, Instant::now());
        }
    }

    /// Call after any settings edit (`reconfigurePipeline`).
    pub fn settings_changed(&mut self, p: &mut dyn Platform) {
        p.apply_chain_settings(&self.settings);
        p.set_bypass(self.bypass());
        // The capture device is fixed when the pipeline starts.
        if p.pipeline_running() && p.pipeline_device() != Some(self.settings.selected_input_device_uid.clone()) {
            self.stop_pipeline(p, None);
            if self.is_call_active() {
                self.start_pipeline_if_needed(p);
            }
        } else if self.enabled() {
            self.start_monitoring(p);
        }
    }

    /// Every 50 ms (the macOS realtime watchdog interval).
    pub fn tick(&mut self, p: &mut dyn Platform, now: Instant) {
        p.poll();
        if let Some(health) = p.ring_health() {
            self.ring = health;
        } else {
            self.ring = RingHealth::default();
            self.start_monitoring(p);
        }
        if let Some(m) = p.metrics() {
            self.metrics = m;
        }
        if let Some(name) = p.engine_name() {
            self.engine_name = name;
        }
        self.echo_reference_active = p.echo_reference_active();

        if self.is_call_active() {
            self.start_pipeline_if_needed(p);
        } else if p.pipeline_running() || self.pending_stop_at.is_some() {
            self.schedule_stop_if_idle(p, now);
            if let Some(at) = self.pending_stop_at {
                if now >= at {
                    self.pending_stop_at = None;
                    if !self.is_call_active() {
                        self.stop_pipeline(p, None);
                    }
                }
            }
        } else if self.ring_ok() {
            self.pipeline_status = self.idle_status();
        }

        let effective = u32::from(self.is_call_active() || self.pending_stop_at.is_some());
        self.report_consumer_count(effective, p);
    }

    fn ring_ok(&self) -> bool {
        self.pipeline_status != text::STATUS_VIRTUAL_MIC_UNAVAILABLE && self.pipeline_status != text::STATUS_ALREADY_RUNNING
    }

    fn start_pipeline_if_needed(&mut self, p: &mut dyn Platform) {
        if !(self.enabled() || self.is_call_active() || self.pending_stop_at.is_some()) {
            return;
        }
        self.pending_stop_at = None;
        if p.pipeline_running() {
            return;
        }
        if p.open_ring().is_err() {
            self.start_monitoring(p);
            return;
        }
        match p.start_pipeline(&self.settings, self.bypass()) {
            Ok(()) => {
                self.mic_access_denied = false;
                self.pipeline_status = if self.enabled() { text::STATUS_RUNNING } else { text::STATUS_PASS_THROUGH };
            }
            Err(StartError::WouldCaptureVirtualDevice) => self.stop_pipeline(p, Some(text::STATUS_PICK_PHYSICAL_MIC)),
            Err(StartError::InputDeviceUnavailable) => self.stop_pipeline(p, Some(text::STATUS_NO_INPUT_DEVICE)),
            Err(StartError::AccessDenied) => {
                self.stop_pipeline(p, Some(text::STATUS_MIC_ACCESS_DENIED));
                self.mic_access_denied = true;
            }
            Err(StartError::Other) => self.stop_pipeline(p, Some(text::STATUS_ERROR)),
        }
    }

    fn schedule_stop_if_idle(&mut self, p: &mut dyn Platform, now: Instant) {
        if !p.pipeline_running() {
            self.pending_stop_at = None;
            self.pipeline_status = self.idle_status();
            return;
        }
        self.pipeline_status = self.idle_status();
        if self.pending_stop_at.is_none() {
            self.pending_stop_at = Some(now + PIPELINE_STOP_DEBOUNCE);
        }
    }

    fn stop_pipeline(&mut self, p: &mut dyn Platform, status: Option<&'static str>) {
        self.pending_stop_at = None;
        p.stop_pipeline();
        self.finish_active_call(p);
        self.last_reported = None;
        self.metrics = ChainMetrics::default();
        self.mic_access_denied = false;
        self.pipeline_status = status.unwrap_or(self.idle_status());
    }

    fn report_consumer_count(&mut self, count: u32, p: &mut dyn Platform) {
        let s = &self.settings;
        let recording_enabled = s.automatic_call_recording_enabled && s.local_recording_enabled && s.recording_consent_acknowledged;
        if self.last_reported == Some((count, recording_enabled)) {
            return;
        }
        self.last_reported = Some((count, recording_enabled));
        if count == 0 {
            self.finish_active_call(p);
        } else if recording_enabled && self.active_call.is_none() && p.pipeline_running() {
            self.active_call = p.start_recording(&self.settings).ok();
        }
    }

    fn finish_active_call(&mut self, p: &mut dyn Platform) {
        if self.active_call.take().is_some() {
            p.stop_recording();
        }
    }

    /// The Record button. Returns false if consent must be asked first.
    pub fn start_manual_recording(&mut self, p: &mut dyn Platform) -> bool {
        if !self.settings.recording_consent_acknowledged {
            return false;
        }
        self.call_status_message = if !self.enabled() {
            "Turn on Mic Processing first"
        } else if !self.is_virtual_mic_in_use(p) {
            "No active meeting detected"
        } else if !self.settings.local_recording_enabled {
            text::LOCAL_RECORDING_IS_OFF
        } else if self.active_call.is_some() {
            ""
        } else {
            match p.start_recording(&self.settings) {
                Ok(call) => {
                    self.active_call = Some(call);
                    ""
                }
                Err(_) => text::STATUS_CALL_STORAGE_UNAVAILABLE,
            }
        };
        true
    }

    pub fn stop_active_call(&mut self, p: &mut dyn Platform) {
        self.finish_active_call(p);
    }

    pub fn shutdown(&mut self, p: &mut dyn Platform) {
        self.stop_pipeline(p, Some(text::STATUS_STOPPED));
    }

    // ------------------------------------------------------------- text

    pub fn virtual_device_status(&self) -> &'static str {
        if self.ring.producer_active {
            if self.enabled() {
                text::STATUS_FEEDING
            } else {
                text::STATUS_PASS_THROUGH
            }
        } else {
            self.pipeline_status
        }
    }

    /// Subtitle under "Pabulong" in the tray menu.
    pub fn menu_subtitle(&self, p: &dyn Platform) -> &'static str {
        if self.enabled() || self.is_virtual_mic_in_use(p) {
            self.virtual_device_status()
        } else {
            text::OFF
        }
    }

    pub fn recording_title(&self, p: &dyn Platform) -> &'static str {
        if self.active_call.is_some() {
            text::RECORDING
        } else if self.is_virtual_mic_in_use(p) {
            text::MEETING_DETECTED
        } else {
            text::NO_ACTIVE_MEETING
        }
    }

    pub fn recording_subtitle(&self, p: &dyn Platform, now_unix: f64) -> String {
        if let Some(call) = &self.active_call {
            return format!("{} · {}", duration_text(call.duration_seconds(now_unix)), text::RECORDING_LOCALLY);
        }
        if self.is_virtual_mic_in_use(p) {
            return if self.settings.local_recording_enabled { text::READY_TO_RECORD } else { text::LOCAL_RECORDING_IS_OFF }.to_string();
        }
        text::SELECT_PABULONG_MIC.to_string()
    }

    pub fn echo_status_text(&self) -> String {
        echo_status_text(self.settings.echo_cancellation_enabled, self.echo_reference_active, &self.metrics)
    }

    pub fn echo_path_text(&self) -> String {
        echo_path_text(self.echo_reference_active, &self.metrics)
    }

    pub fn voice_gate_text(&self) -> String {
        voice_gate_text(self.settings.voice_isolation, &self.metrics)
    }
}

/// `DiagnosticsSnapshot` on macOS, same keys, as pretty JSON.
pub fn diagnostics_json(c: &Controller, app_version: &str, input_devices: &[String]) -> String {
    use crate::json::Value;
    let r = &c.ring;
    Value::object()
        .with("appVersion", app_version)
        .with("virtualDeviceName", text::VIRTUAL_MIC_NAME)
        .with("settings", c.settings.to_json())
        .with("inputDevices", input_devices.iter().map(|d| Value::from(d.as_str())).collect::<Vec<_>>())
        .with(
            "bridge",
            Value::object()
                .with("availableReadFrames", r.available_read_frames)
                .with("availableWriteFrames", r.available_write_frames)
                .with("underrunCount", r.underrun_count)
                .with("overrunCount", r.overrun_count)
                .with("producerAttached", r.producer_active)
                .with("consumerRunningCount", r.consumer_running_count)
                // Windows only: frames the driver skipped to bound latency.
                .with("droppedCount", r.dropped_count)
                .with("driverVersion", r.driver_version),
        )
        .with(
            "processor",
            Value::object()
                .with("inputRMS", c.metrics.input_rms)
                .with("outputRMS", c.metrics.output_rms)
                .with("voiceActive", c.metrics.voice_probability > 0.5),
        )
        .with("pipelineStatus", c.pipeline_status)
        .with("engine", c.engine_name)
        .to_pretty()
}

pub fn echo_status_text(enabled: bool, reference_active: bool, m: &ChainMetrics) -> String {
    if !enabled {
        return text::OFF.into();
    }
    if !reference_active {
        return text::WAITING_FOR_SPEAKER_AUDIO.into();
    }
    if m.echo_reduction_db > 0.5 {
        format!("{:.0} dB removed", m.echo_reduction_db)
    } else if m.far_end_active {
        text::LEARNING.into()
    } else {
        text::READY.into()
    }
}

pub fn echo_path_text(reference_active: bool, m: &ChainMetrics) -> String {
    if !reference_active {
        return text::EM_DASH.into();
    }
    format!("{:.0} ms delay · ref −{:.0} ms", m.echo_delay_ms, m.reference_lookback_ms)
}

pub fn voice_gate_text(isolation: VoiceIsolation, m: &ChainMetrics) -> String {
    if isolation == VoiceIsolation::Off {
        return text::OFF.into();
    }
    if m.voice_gate_gain < 0.9 {
        format!("Ducking ({:.0} dB)", 20.0 * m.voice_gate_gain.max(0.001).log10())
    } else {
        format!("{} · open", isolation.display_name())
    }
}

/// `callDurationText` on macOS: "4m 05s" style without padding ("4m 5s"), or "1h 3m".
pub fn duration_text(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    let minutes = seconds / 60;
    if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{}m {}s", minutes, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::CallStatus;
    use std::path::PathBuf;

    #[derive(Default)]
    struct Fake {
        ring_missing: bool,
        consumers: u32,
        running: Option<Option<String>>,
        bypass: bool,
        start_result: Option<StartError>,
        starts: u32,
        stops: u32,
        recording: bool,
        recordings_started: u32,
    }

    impl Platform for Fake {
        fn open_ring(&mut self) -> Result<(), RingOpenError> {
            if self.ring_missing { Err(RingOpenError::DriverMissing) } else { Ok(()) }
        }
        fn ring_health(&self) -> Option<RingHealth> {
            (!self.ring_missing).then(|| RingHealth {
                consumer_running_count: self.consumers,
                producer_active: self.running.is_some(),
                ..RingHealth::default()
            })
        }
        fn start_pipeline(&mut self, s: &AppSettings, bypass: bool) -> Result<(), StartError> {
            if let Some(e) = self.start_result {
                return Err(e);
            }
            self.starts += 1;
            self.bypass = bypass;
            self.running = Some(s.selected_input_device_uid.clone());
            Ok(())
        }
        fn stop_pipeline(&mut self) {
            if self.running.take().is_some() {
                self.stops += 1;
            }
        }
        fn pipeline_running(&self) -> bool {
            self.running.is_some()
        }
        fn pipeline_device(&self) -> Option<Option<String>> {
            self.running.clone()
        }
        fn set_bypass(&mut self, bypass: bool) {
            self.bypass = bypass;
        }
        fn apply_chain_settings(&mut self, _: &AppSettings) {}
        fn metrics(&self) -> Option<ChainMetrics> {
            None
        }
        fn engine_name(&self) -> Option<&'static str> {
            None
        }
        fn echo_reference_active(&self) -> bool {
            self.running.is_some()
        }
        fn start_recording(&mut self, _: &AppSettings) -> std::io::Result<CallSession> {
            self.recording = true;
            self.recordings_started += 1;
            Ok(CallSession {
                id: "x".into(),
                title: "Call".into(),
                started_at: 100.0,
                ended_at: None,
                status: CallStatus::Recording,
                app_name: None,
                directory: PathBuf::from("x"),
                remote_audio_missing: false,
            })
        }
        fn stop_recording(&mut self) -> Option<CallSession> {
            self.recording = false;
            None
        }
    }

    fn enabled() -> AppSettings {
        AppSettings { processing_enabled: true, ..AppSettings::default() }
    }

    #[test]
    fn mic_is_captured_only_during_a_call_and_stops_after_the_debounce() {
        let mut p = Fake::default();
        let mut c = Controller::new(enabled());
        let t0 = Instant::now();
        c.start_monitoring(&mut p);
        c.tick(&mut p, t0);
        assert!(!p.pipeline_running());
        assert_eq!(c.pipeline_status, text::STATUS_WAITING_FOR_CALL);
        assert_eq!(c.virtual_device_status(), text::STATUS_WAITING_FOR_CALL);

        p.consumers = 1;
        c.tick(&mut p, t0);
        assert!(p.pipeline_running());
        assert!(!p.bypass);
        c.tick(&mut p, t0); // ring now reports producer_active
        assert_eq!(c.virtual_device_status(), text::STATUS_FEEDING);

        p.consumers = 0;
        c.tick(&mut p, t0 + Duration::from_millis(50));
        assert!(p.pipeline_running(), "debounced");
        // A call app that reconnects within the debounce keeps the pipeline.
        p.consumers = 1;
        c.tick(&mut p, t0 + Duration::from_millis(500));
        p.consumers = 0;
        c.tick(&mut p, t0 + Duration::from_millis(600));
        c.tick(&mut p, t0 + Duration::from_millis(1500));
        assert!(p.pipeline_running());
        c.tick(&mut p, t0 + Duration::from_millis(1650));
        assert!(!p.pipeline_running());
        assert_eq!((p.starts, p.stops), (1, 1));
    }

    #[test]
    fn processing_off_still_passes_the_mic_through_during_a_call() {
        let mut p = Fake { consumers: 1, ..Fake::default() };
        let mut c = Controller::new(AppSettings::default());
        c.tick(&mut p, Instant::now());
        assert!(p.pipeline_running());
        assert!(p.bypass);
        assert_eq!(c.pipeline_status, text::STATUS_PASS_THROUGH);
        c.set_processing_enabled(true, &mut p);
        assert!(!p.bypass);
    }

    #[test]
    fn start_errors_map_to_the_macos_status_text() {
        for (err, status) in [
            (StartError::WouldCaptureVirtualDevice, text::STATUS_PICK_PHYSICAL_MIC),
            (StartError::InputDeviceUnavailable, text::STATUS_NO_INPUT_DEVICE),
            (StartError::AccessDenied, text::STATUS_MIC_ACCESS_DENIED),
            (StartError::Other, text::STATUS_ERROR),
        ] {
            let mut p = Fake { consumers: 1, start_result: Some(err), ..Fake::default() };
            let mut c = Controller::new(enabled());
            c.tick(&mut p, Instant::now());
            assert_eq!(c.pipeline_status, status);
            assert_eq!(c.mic_access_denied, err == StartError::AccessDenied);
        }
    }

    #[test]
    fn missing_driver_says_virtual_mic_unavailable() {
        let mut p = Fake { ring_missing: true, ..Fake::default() };
        let mut c = Controller::new(enabled());
        c.start_monitoring(&mut p);
        c.tick(&mut p, Instant::now());
        assert_eq!(c.virtual_device_status(), text::STATUS_VIRTUAL_MIC_UNAVAILABLE);
    }

    #[test]
    fn automatic_recording_needs_consent_and_follows_the_call() {
        let mut p = Fake { consumers: 1, ..Fake::default() };
        let mut s = enabled();
        s.automatic_call_recording_enabled = true;
        let mut c = Controller::new(s);
        let t0 = Instant::now();
        c.tick(&mut p, t0);
        assert!(!p.recording, "no recording before consent");
        c.settings.recording_consent_acknowledged = true;
        c.tick(&mut p, t0);
        assert!(p.recording);
        assert_eq!(c.recording_title(&p), text::RECORDING);
        assert_eq!(c.recording_subtitle(&p, 165.0), "1m 5s · Recording locally");
        p.consumers = 0;
        c.tick(&mut p, t0 + Duration::from_secs(2));
        c.tick(&mut p, t0 + Duration::from_secs(4));
        assert!(!p.recording);
        assert_eq!(p.recordings_started, 1);
    }

    #[test]
    fn manual_recording_asks_for_consent_first() {
        let mut p = Fake { consumers: 1, ..Fake::default() };
        let mut c = Controller::new(enabled());
        c.tick(&mut p, Instant::now());
        assert!(!c.start_manual_recording(&mut p));
        c.settings.recording_consent_acknowledged = true;
        assert!(c.start_manual_recording(&mut p));
        assert!(p.recording);
    }

    #[test]
    fn changing_the_mic_restarts_a_running_pipeline() {
        let mut p = Fake { consumers: 1, ..Fake::default() };
        let mut c = Controller::new(enabled());
        c.tick(&mut p, Instant::now());
        c.settings.selected_input_device_uid = Some("usb".into());
        c.settings_changed(&mut p);
        assert_eq!(p.running, Some(Some("usb".into())));
        assert_eq!((p.starts, p.stops), (2, 1));
    }

    #[test]
    fn diagnostics_export_has_the_macos_keys() {
        let c = Controller::new(enabled());
        let json = crate::json::Value::parse(&diagnostics_json(&c, "0.1.0", &["USB Mic [id]".into()])).unwrap();
        for key in ["appVersion", "virtualDeviceName", "settings", "inputDevices", "bridge", "processor"] {
            assert!(json.get(key).is_some(), "{key}");
        }
        assert_eq!(json.get("virtualDeviceName").and_then(|v| v.as_str()), Some("Pabulong Mic"));
        assert_eq!(json.get("settings").and_then(|s| s.get("processingEnabled")).and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn status_text_matches_macos() {
        let m = ChainMetrics { echo_reduction_db: 29.4, echo_delay_ms: 12.2, reference_lookback_ms: 8.6, voice_gate_gain: 0.05, ..Default::default() };
        assert_eq!(echo_status_text(true, true, &m), "29 dB removed");
        assert_eq!(echo_status_text(true, false, &m), "Waiting for speaker audio");
        assert_eq!(echo_status_text(false, true, &m), "Off");
        assert_eq!(echo_status_text(true, true, &ChainMetrics { far_end_active: true, ..Default::default() }), "Learning…");
        assert_eq!(echo_path_text(true, &m), "12 ms delay · ref −9 ms");
        assert_eq!(voice_gate_text(VoiceIsolation::Balanced, &m), "Ducking (-26 dB)");
        assert_eq!(voice_gate_text(VoiceIsolation::Light, &ChainMetrics { voice_gate_gain: 1.0, ..Default::default() }), "Light · open");
        assert_eq!(duration_text(3725.0), "1h 2m");
        assert_eq!(duration_text(65.0), "1m 5s");
    }
}
