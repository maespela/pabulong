//! Every label and caption the Windows app shows, copied from the macOS app
//! (`Sources/NoiselessApp/main.swift`) so both read the same. The only edits
//! are platform nouns, marked `// Windows:`.

pub const APP_NAME: &str = "Pabulong";
pub const VIRTUAL_MIC_NAME: &str = "Pabulong Mic";

// Menu (tray) -----------------------------------------------------------------
pub const MIC_PROCESSING: &str = "Mic Processing";
pub const OFF: &str = "Off";
pub const MICROPHONE_SETTINGS: &str = "Microphone Settings";
pub const DEFAULT_INPUT_WARNING: &str = "Pabulong Mic is your default input";
pub const DEFAULT_INPUT_DETAIL: &str =
    "That keeps your mic active all the time. Use Pabulong Mic only inside your call app.";
pub const DEFAULT_INPUT_FIX: &str = "Fix — switch default to a real mic";
pub const OPEN_PABULONG: &str = "Open Pabulong";
pub const QUIT: &str = "Quit";

// Recording strip ---------------------------------------------------------------
pub const RECORDING: &str = "Recording";
pub const MEETING_DETECTED: &str = "Meeting Detected";
pub const NO_ACTIVE_MEETING: &str = "No Active Meeting";
pub const RECORDING_LOCALLY: &str = "Recording locally";
pub const READY_TO_RECORD: &str = "Ready to record";
pub const LOCAL_RECORDING_IS_OFF: &str = "Local recording is off";
pub const SELECT_PABULONG_MIC: &str = "Select Pabulong Mic in a call";
pub const RECORD: &str = "Record";
pub const STOP: &str = "Stop";
pub const START_RECORDING_HELP: &str = "Start recording";
pub const STOP_RECORDING_HELP: &str = "Stop recording";
pub const STOP_RECORDING: &str = "Stop Recording";

// Terms (first run) ---------------------------------------------------------------
/// Bump when the Terms of Use or Privacy Notice change materially (same
/// number as `MenuBarModel.currentTermsVersion` on macOS).
pub const CURRENT_TERMS_VERSION: i64 = 2;
pub const TERMS_URL: &str = "https://nextmatterpro.com/training/pabulong-terms.php";
pub const PRIVACY_URL: &str = "https://nextmatterpro.com/training/pabulong-privacy.php";
pub const WELCOME_TITLE: &str = "Welcome to Pabulong";
// Windows: "this Mac" → "this PC".
pub const WELCOME_BODY: &str = "Please read and agree to the Pabulong Terms of Use and Privacy Notice before you continue.\n\n\
• Pabulong works on this PC. It does not send your audio or recordings to anyone.\n\
• If you record calls, you are bound by the recording laws where you and everyone on the call are, and you must know and follow them. Next Matter Pro does not control your recordings and is not liable for them.";
pub const I_AGREE: &str = "I Agree";
pub const READ_TERMS: &str = "Read Terms";

// Recording consent (once, before the first recording) ----------------------------
pub const CONSENT_TITLE: &str = "Recording laws are your responsibility";
// Windows: "this Mac" → "this PC".
pub const CONSENT_BODY: &str = "When you record, you are bound by the recording laws where you are and where everyone on the call is, and you need to know them. Many places, including the Philippines and several U.S. states such as California and Florida, require everyone's consent. Follow your client's or law firm's confidentiality rules too. Pabulong does not control or review your recordings, and Next Matter Pro is not liable for them. Recordings stay on this PC.";
pub const I_UNDERSTAND: &str = "I Understand";
pub const CANCEL: &str = "Cancel";

// Main window ---------------------------------------------------------------------
pub const CALLS: &str = "Calls";
pub const SETTINGS: &str = "Settings";
pub const DIAGNOSTICS: &str = "Diagnostics";
pub const NO_CALLS: &str = "No Calls";
pub const RECORDINGS_APPEAR_HERE: &str = "Recordings will appear here.";
pub const REVEAL: &str = "Reveal";
pub const DELETE: &str = "Delete";
pub const SAVED_SUFFIX: &str = "saved";

// Settings --------------------------------------------------------------------------
pub const SECTION_AUDIO: &str = "Audio";
pub const MICROPHONE: &str = "Microphone";
pub const SYSTEM_DEFAULT: &str = "System Default";
pub const DEFAULT_SUFFIX: &str = " (Default)";
pub const SUPPRESSION: &str = "Suppression";
pub const ECHO_CANCELLATION: &str = "Echo Cancellation";
pub const ECHO_CANCELLATION_CAPTION: &str =
    "Removes the other person's voice from your mic when you use speakers instead of headphones.";
pub const BACKGROUND_VOICES: &str = "Background Voices";
pub const BACKGROUND_VOICES_CAPTION: &str =
    "Turns down people talking near you who aren't on the call. Strong also trims your own softest speech.";
pub const BYPASS: &str = "Bypass";
pub const SECTION_RECORDING: &str = "Recording";
pub const AUTOMATIC_CALL_RECORDING: &str = "Automatic Call Recording";
pub const PAUSED_UNTIL_CONSENT: &str = "Paused until you confirm recording consent.";
pub const REVIEW: &str = "Review";
pub const LOCAL_RECORDING: &str = "Local Recording";
pub const CAPTURE_SYSTEM_AUDIO: &str = "Capture System Audio";
// Windows: the macOS caption names Core Audio; WASAPI loopback is the Windows
// equivalent and also needs no extra permission.
pub const CAPTURE_SYSTEM_AUDIO_CAPTION: &str =
    "Captures the other side of the call via WASAPI loopback — no extra permission needed.";
pub const OPEN_CALL_STORAGE: &str = "Open Call Storage";
pub const SECTION_SYSTEM: &str = "System";
// Windows: "Launch at Login" is the macOS wording; kept identical.
pub const LAUNCH_AT_LOGIN: &str = "Launch at Login";
pub const SECTION_ABOUT: &str = "About";
pub const VERSION: &str = "Version";
pub const COPYRIGHT: &str = "© 2026 Next Matter Pro. Open source under the MIT License.";
pub const PRIVACY_NOTICE: &str = "Privacy Notice";
pub const TERMS_OF_USE: &str = "Terms of Use";
pub const THIRD_PARTY_NOTICES: &str = "Third-Party Notices";
pub const DONE: &str = "Done";

// Diagnostics -------------------------------------------------------------------------
pub const INPUT: &str = "Input";
pub const OUTPUT: &str = "Output";
pub const ENGINE: &str = "Engine";
pub const MIC_CLIENTS: &str = "Mic Clients";
pub const LATENCY: &str = "Latency";
pub const LATENCY_VALUE: &str = "< 40 ms target";
pub const VIRTUAL_MIC: &str = "Virtual Mic";
pub const ECHO_PATH: &str = "Echo Path";
pub const SPEAKER_REFERENCE: &str = "Speaker Reference";
pub const TAPPING_SYSTEM_OUTPUT: &str = "Tapping system output";
pub const NOT_RUNNING: &str = "Not running";
pub const REFRESH: &str = "Refresh";
pub const EXPORT_DIAGNOSTICS: &str = "Export Diagnostics";

// Pipeline status (MenuBarModel.pipelineStatus) ----------------------------------------
pub const STATUS_STOPPED: &str = "Stopped";
pub const STATUS_WAITING_FOR_CALL: &str = "Waiting for call";
pub const STATUS_RUNNING: &str = "Running";
pub const STATUS_PASS_THROUGH: &str = "Pass Through";
pub const STATUS_FEEDING: &str = "Feeding";
pub const STATUS_VIRTUAL_MIC_UNAVAILABLE: &str = "Virtual mic unavailable";
pub const STATUS_MIC_ACCESS_DENIED: &str = "Microphone access denied";
pub const STATUS_PICK_PHYSICAL_MIC: &str = "Pick a physical mic";
pub const STATUS_NO_INPUT_DEVICE: &str = "No input device";
pub const STATUS_ALREADY_RUNNING: &str = "Already running elsewhere";
pub const STATUS_ERROR: &str = "Error";
pub const STATUS_LOGIN_ITEM_UNAVAILABLE: &str = "Login item unavailable";
pub const STATUS_CALL_STORAGE_UNAVAILABLE: &str = "Call storage unavailable";

// Echo / voice gate (MenuBarModel.echoStatusText etc.) ---------------------------------
pub const WAITING_FOR_SPEAKER_AUDIO: &str = "Waiting for speaker audio";
pub const LEARNING: &str = "Learning…";
pub const READY: &str = "Ready";
pub const EM_DASH: &str = "—";

// Engine names (RustVoiceProcessor.engineName) --------------------------------------------
pub const ENGINE_NEURAL: &str = "RNNoise (neural) · shared core";
pub const ENGINE_SPECTRAL: &str = "Spectral DSP · shared core";
pub const ENGINE_SHARED_CORE: &str = "RNNoise + echo cancellation (shared core)";
