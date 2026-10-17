//! App settings: the same fields, JSON keys, raw values and defaults as
//! `AppSettings` on macOS, saved to `%APPDATA%\Pabulong\settings.json`.

use crate::json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuppressionMode {
    Light,
    Balanced,
    Max,
}

impl SuppressionMode {
    pub const ALL: [SuppressionMode; 3] = [SuppressionMode::Light, SuppressionMode::Balanced, SuppressionMode::Max];

    pub fn raw(self) -> &'static str {
        match self {
            SuppressionMode::Light => "light",
            SuppressionMode::Balanced => "balanced",
            SuppressionMode::Max => "max",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.raw() == raw)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            SuppressionMode::Light => "Light",
            SuppressionMode::Balanced => "Balanced",
            SuppressionMode::Max => "Max",
        }
    }

    /// Value for `nlr_pipeline_create`/`set_mode`.
    pub fn core_value(self) -> u32 {
        match self {
            SuppressionMode::Light => 0,
            SuppressionMode::Balanced => 1,
            SuppressionMode::Max => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceIsolation {
    Off,
    Light,
    Balanced,
    Strong,
}

impl VoiceIsolation {
    pub const ALL: [VoiceIsolation; 4] = [VoiceIsolation::Off, VoiceIsolation::Light, VoiceIsolation::Balanced, VoiceIsolation::Strong];

    pub fn raw(self) -> &'static str {
        match self {
            VoiceIsolation::Off => "off",
            VoiceIsolation::Light => "light",
            VoiceIsolation::Balanced => "balanced",
            VoiceIsolation::Strong => "strong",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.raw() == raw)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            VoiceIsolation::Off => "Off",
            VoiceIsolation::Light => "Light",
            VoiceIsolation::Balanced => "Balanced",
            VoiceIsolation::Strong => "Strong",
        }
    }

    pub fn core_value(self) -> u32 {
        match self {
            VoiceIsolation::Off => 0,
            VoiceIsolation::Light => 1,
            VoiceIsolation::Balanced => 2,
            VoiceIsolation::Strong => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    pub processing_enabled: bool,
    /// WASAPI endpoint id of the chosen physical mic (`None` = system default).
    pub selected_input_device_uid: Option<String>,
    pub suppression_mode: SuppressionMode,
    pub launch_at_login: bool,
    pub bypass_enabled: bool,
    /// Always "rnnoise" (kept for file compatibility with macOS).
    pub processor_type: String,
    pub automatic_call_recording_enabled: bool,
    pub capture_system_audio_enabled: bool,
    pub local_recording_enabled: bool,
    pub echo_cancellation_enabled: bool,
    pub voice_isolation: VoiceIsolation,
    pub recording_consent_acknowledged: bool,
    pub accepted_terms_version: i64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            processing_enabled: false,
            selected_input_device_uid: None,
            suppression_mode: SuppressionMode::Balanced,
            launch_at_login: false,
            bypass_enabled: false,
            processor_type: "rnnoise".into(),
            automatic_call_recording_enabled: false,
            capture_system_audio_enabled: true,
            local_recording_enabled: true,
            echo_cancellation_enabled: true,
            voice_isolation: VoiceIsolation::Balanced,
            recording_consent_acknowledged: false,
            accepted_terms_version: 0,
        }
    }
}

impl AppSettings {
    pub fn to_json(&self) -> Value {
        Value::object()
            .with("processingEnabled", self.processing_enabled)
            .with("selectedInputDeviceUID", self.selected_input_device_uid.clone())
            .with("suppressionMode", self.suppression_mode.raw())
            .with("launchAtLogin", self.launch_at_login)
            .with("bypassEnabled", self.bypass_enabled)
            .with("processorType", self.processor_type.as_str())
            .with("automaticCallRecordingEnabled", self.automatic_call_recording_enabled)
            .with("captureSystemAudioEnabled", self.capture_system_audio_enabled)
            .with("localRecordingEnabled", self.local_recording_enabled)
            .with("echoCancellationEnabled", self.echo_cancellation_enabled)
            .with("voiceIsolation", self.voice_isolation.raw())
            .with("recordingConsentAcknowledged", self.recording_consent_acknowledged)
            .with("acceptedTermsVersion", self.accepted_terms_version)
    }

    /// Missing or unreadable fields fall back to their defaults one by one,
    /// like the macOS decoder, so one bad value never resets everything.
    pub fn from_json(v: &Value) -> Self {
        let d = AppSettings::default();
        let b = |k: &str, def: bool| v.get(k).and_then(Value::as_bool).unwrap_or(def);
        AppSettings {
            processing_enabled: b("processingEnabled", d.processing_enabled),
            selected_input_device_uid: v.get("selectedInputDeviceUID").and_then(Value::as_str).map(str::to_string),
            suppression_mode: v
                .get("suppressionMode")
                .and_then(Value::as_str)
                .and_then(SuppressionMode::from_raw)
                .unwrap_or(d.suppression_mode),
            launch_at_login: b("launchAtLogin", d.launch_at_login),
            bypass_enabled: b("bypassEnabled", d.bypass_enabled),
            // Unknown engines saved by older builds map to RNNoise.
            processor_type: d.processor_type,
            automatic_call_recording_enabled: b("automaticCallRecordingEnabled", d.automatic_call_recording_enabled),
            capture_system_audio_enabled: b("captureSystemAudioEnabled", d.capture_system_audio_enabled),
            local_recording_enabled: b("localRecordingEnabled", d.local_recording_enabled),
            echo_cancellation_enabled: b("echoCancellationEnabled", d.echo_cancellation_enabled),
            voice_isolation: v
                .get("voiceIsolation")
                .and_then(Value::as_str)
                .and_then(VoiceIsolation::from_raw)
                .unwrap_or(d.voice_isolation),
            recording_consent_acknowledged: b("recordingConsentAcknowledged", d.recording_consent_acknowledged),
            accepted_terms_version: v.get("acceptedTermsVersion").and_then(Value::as_f64).map_or(d.accepted_terms_version, |n| n as i64),
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> AppSettings {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| Value::parse(&text).ok())
            .map(|v| AppSettings::from_json(&v))
            .unwrap_or_default()
    }

    /// Writes atomically (temp file + rename) so a crash never leaves half a file.
    pub fn save(&self, settings: &AppSettings) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, settings.to_json().to_pretty())?;
        std::fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_macos() {
        let s = AppSettings::default();
        assert!(!s.processing_enabled);
        assert_eq!(s.suppression_mode, SuppressionMode::Balanced);
        assert!(s.echo_cancellation_enabled);
        assert_eq!(s.voice_isolation, VoiceIsolation::Balanced);
        assert!(s.capture_system_audio_enabled && s.local_recording_enabled);
        assert!(!s.automatic_call_recording_enabled && !s.recording_consent_acknowledged);
        assert_eq!(s.accepted_terms_version, 0);
    }

    #[test]
    fn reads_a_macos_settings_blob() {
        // As written by JSONEncoder on macOS.
        let mac = r#"{"processingEnabled":true,"suppressionMode":"max","launchAtLogin":false,
            "bypassEnabled":false,"processorType":"deepFilterNet","automaticCallRecordingEnabled":true,
            "captureSystemAudioEnabled":false,"localRecordingEnabled":true,"echoCancellationEnabled":false,
            "voiceIsolation":"strong","recordingConsentAcknowledged":true,"acceptedTermsVersion":2}"#;
        let s = AppSettings::from_json(&Value::parse(mac).unwrap());
        assert!(s.processing_enabled);
        assert_eq!(s.suppression_mode, SuppressionMode::Max);
        assert_eq!(s.voice_isolation, VoiceIsolation::Strong);
        assert_eq!(s.processor_type, "rnnoise");
        assert!(!s.echo_cancellation_enabled && !s.capture_system_audio_enabled);
        assert_eq!(s.accepted_terms_version, 2);
        assert_eq!(s.selected_input_device_uid, None);
    }

    #[test]
    fn bad_values_fall_back_individually() {
        let s = AppSettings::from_json(&Value::parse(r#"{"suppressionMode":"loud","bypassEnabled":true}"#).unwrap());
        assert_eq!(s.suppression_mode, SuppressionMode::Balanced);
        assert!(s.bypass_enabled);
    }

    #[test]
    fn store_round_trips() {
        let dir = std::env::temp_dir().join(format!("pabulong-settings-{}", std::process::id()));
        let store = SettingsStore::new(dir.join("settings.json"));
        assert_eq!(store.load(), AppSettings::default());
        let mut s = AppSettings::default();
        s.selected_input_device_uid = Some("{0.0.1.00000000}.{abc}".into());
        s.voice_isolation = VoiceIsolation::Off;
        store.save(&s).unwrap();
        assert_eq!(store.load(), s);
        std::fs::remove_dir_all(dir).ok();
    }
}
