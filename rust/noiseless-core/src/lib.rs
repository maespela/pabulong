//! Portable real-time noise suppression core for Noiseless.
//!
//! The same processing code runs on macOS and Windows. Platform-specific audio
//! I/O (the virtual microphone driver, system-audio capture) lives in the apps;
//! this crate only turns 48 kHz mono frames of noisy audio into cleaner ones.

pub mod ffi;
pub mod aec;
pub mod fft;
pub mod frame;
pub mod mode;
#[cfg(feature = "rnnoise")]
mod neural;
pub mod pipeline;
pub mod processor;
pub mod reference;
pub mod ring;
pub mod spectral;
pub mod voice_gate;

pub use aec::{AecMetrics, EchoCanceller};
pub use frame::{limit, rms, FRAME_LENGTH, SAMPLE_RATE};
pub use mode::NoiseMode;
pub use pipeline::{PipelineMetrics, VoicePipeline};
pub use processor::{Metrics, NoiseProcessor, ProcessError};
pub use reference::ReferenceQueue;
pub use ring::RingBuffer;
pub use voice_gate::{VoiceGate, VoiceIsolation};
