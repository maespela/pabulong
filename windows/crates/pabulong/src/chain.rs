//! The shared mic chain (echo canceller → RNNoise → background-voice gate)
//! from `rust/noiseless-core`, driven through the same `nlr_pipeline_*` C ABI
//! the macOS app calls from Swift (`RustVoiceProcessor` in VoiceChain.swift),
//! so both platforms get identical processing and threading behaviour.

use crate::settings::{SuppressionMode, VoiceIsolation};
use crate::text;
use noiseless_core::ffi;

pub const FRAME_LENGTH: usize = noiseless_core::FRAME_LENGTH;

/// Shown before the first pipeline exists (`NoiseProcessorFactory.engineDescription`).
pub fn default_engine_name() -> &'static str {
    text::ENGINE_SHARED_CORE
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChainMetrics {
    pub input_rms: f32,
    pub output_rms: f32,
    pub voice_probability: f32,
    pub echo_reduction_db: f32,
    pub echo_delay_ms: f32,
    pub reference_lookback_ms: f32,
    pub voice_gate_gain: f32,
    pub far_end_active: bool,
}

/// Owns one `NlrPipeline`.
///
/// Threading (the core's contract): `process` from one audio thread only;
/// everything else from any thread.
pub struct VoiceChain {
    handle: *mut ffi::PipelineHandle,
}

// SAFETY: the core documents the handle as usable from any thread except that
// `nlr_pipeline_process` must not run concurrently with itself, which
// `process(&self)` callers guarantee by owning the single audio thread.
unsafe impl Send for VoiceChain {}
unsafe impl Sync for VoiceChain {}

impl VoiceChain {
    pub fn new(mode: SuppressionMode, echo_cancellation: bool, isolation: VoiceIsolation) -> Self {
        let handle = ffi::nlr_pipeline_create(mode.core_value(), echo_cancellation, isolation.core_value());
        assert!(!handle.is_null(), "nlr_pipeline_create returned null");
        Self { handle }
    }

    pub fn set_mode(&self, mode: SuppressionMode) {
        unsafe { ffi::nlr_pipeline_set_mode(self.handle, mode.core_value()) }
    }

    pub fn set_voice_isolation(&self, isolation: VoiceIsolation) {
        unsafe { ffi::nlr_pipeline_set_voice_isolation(self.handle, isolation.core_value()) }
    }

    pub fn set_echo_cancellation(&self, enabled: bool) {
        unsafe { ffi::nlr_pipeline_set_echo_cancellation(self.handle, enabled) }
    }

    pub fn uses_neural(&self) -> bool {
        unsafe { ffi::nlr_pipeline_uses_neural(self.handle) }
    }

    pub fn engine_name(&self) -> &'static str {
        if self.uses_neural() {
            text::ENGINE_NEURAL
        } else {
            text::ENGINE_SPECTRAL
        }
    }

    /// One 480-sample frame whose first sample was captured at `host_time_ns`
    /// (QPC converted to ns). Returns false on a bad frame length.
    pub fn process(&self, mic: &[f32], out: &mut [f32], host_time_ns: u64) -> bool {
        if mic.len() != FRAME_LENGTH || out.len() != FRAME_LENGTH {
            return false;
        }
        unsafe { ffi::nlr_pipeline_process(self.handle, mic.as_ptr(), out.as_mut_ptr(), FRAME_LENGTH as u32, host_time_ns) }
    }

    /// Speaker output (48 kHz mono) whose first sample was played at `host_time_ns`.
    pub fn push_reference(&self, samples: &[f32], host_time_ns: u64) {
        if samples.is_empty() {
            return;
        }
        unsafe { ffi::nlr_pipeline_push_reference(self.handle, samples.as_ptr(), samples.len() as u32, host_time_ns) }
    }

    pub fn metrics(&self) -> ChainMetrics {
        let m = unsafe { ffi::nlr_pipeline_metrics(self.handle) };
        ChainMetrics {
            input_rms: m.input_rms,
            output_rms: m.output_rms,
            voice_probability: m.voice_probability,
            echo_reduction_db: m.echo_reduction_db,
            echo_delay_ms: m.echo_delay_ms,
            reference_lookback_ms: m.reference_lookback_ms,
            voice_gate_gain: m.voice_gate_gain,
            far_end_active: m.far_end_active,
        }
    }
}

impl Drop for VoiceChain {
    fn drop(&mut self) {
        unsafe { ffi::nlr_pipeline_destroy(self.handle) }
    }
}
