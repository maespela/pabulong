//! The complete mic chain shared by the macOS and Windows apps:
//!
//!   mic ─► echo canceller (reference = speaker output) ─► noise suppressor
//!       ─► background-voice gate ─► out
//!
//! Echo cancellation comes first because every later stage is non-linear.

use crate::aec::{AecMetrics, EchoCanceller};
use crate::frame::{FRAME_LENGTH, SAMPLE_RATE};
use crate::mode::NoiseMode;
use crate::processor::{Metrics, NoiseProcessor, ProcessError};
use crate::reference::ReferenceQueue;
use crate::voice_gate::{VoiceGate, VoiceIsolation};

/// Echo tail the canceller models, including reference/mic misalignment.
const ECHO_TAIL_MS: f32 = 250.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PipelineMetrics {
    pub noise: Metrics,
    pub echo: AecMetrics,
    pub voice_gate_gain: f32,
}

pub struct VoicePipeline {
    aec: Option<EchoCanceller>,
    noise: NoiseProcessor,
    gate: VoiceGate,
    reference: Vec<f32>,
    stage: Vec<f32>,
}

impl VoicePipeline {
    pub fn new(mode: NoiseMode, echo_cancellation: bool, isolation: VoiceIsolation) -> Self {
        Self {
            aec: echo_cancellation.then(|| EchoCanceller::new(ECHO_TAIL_MS, SAMPLE_RATE)),
            noise: NoiseProcessor::standard(mode),
            gate: VoiceGate::new(isolation),
            reference: vec![0.0; FRAME_LENGTH],
            stage: vec![0.0; FRAME_LENGTH],
        }
    }

    /// Samples of latency added on top of the noise suppressor's own.
    pub fn added_latency(&self) -> usize {
        if self.aec.is_some() { EchoCanceller::LATENCY } else { 0 }
    }

    pub fn set_mode(&mut self, mode: NoiseMode) {
        self.noise.set_mode(mode);
    }

    pub fn set_voice_isolation(&mut self, isolation: VoiceIsolation) {
        self.gate.set_isolation(isolation);
    }

    pub fn set_echo_cancellation(&mut self, enabled: bool) {
        if enabled != self.aec.is_some() {
            self.aec = enabled.then(|| EchoCanceller::new(ECHO_TAIL_MS, SAMPLE_RATE));
        }
    }

    pub fn uses_neural(&self) -> bool {
        self.noise.uses_neural()
    }

    pub fn metrics(&self) -> PipelineMetrics {
        PipelineMetrics {
            noise: self.noise.metrics(),
            echo: self.aec.as_ref().map(|a| a.metrics()).unwrap_or_default(),
            voice_gate_gain: self.gate.gain(),
        }
    }

    pub fn reset(&mut self) {
        if let Some(aec) = self.aec.as_mut() {
            aec.reset();
        }
        self.noise.reset();
        self.gate.reset();
    }

    /// Processes one 480-sample frame captured at `host_time_ns`, using
    /// `reference` (speaker output) for echo cancellation when enabled.
    pub fn process(
        &mut self,
        mic: &[f32],
        out: &mut [f32],
        reference: Option<&ReferenceQueue>,
        host_time_ns: u64,
    ) -> Result<(), ProcessError> {
        if mic.len() != FRAME_LENGTH || out.len() != FRAME_LENGTH {
            return Err(ProcessError::WrongFrameLength { expected: FRAME_LENGTH, got: mic.len().min(out.len()) });
        }
        match self.aec.as_mut() {
            Some(aec) => {
                match reference {
                    Some(q) => {
                        q.read_at(&mut self.reference, host_time_ns);
                    }
                    None => self.reference.fill(0.0),
                }
                aec.process(mic, &self.reference, &mut self.stage);
            }
            None => self.stage.copy_from_slice(mic),
        }
        self.noise.process(&self.stage, out)?;
        let vad = self.noise.metrics().vad;
        self.gate.process(out, vad);
        Ok(())
    }
}
