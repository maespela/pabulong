//! The noise processor: high-pass → (RNNoise | spectral) → limiter.

use crate::frame::{limit, rms, FRAME_LENGTH, SAMPLE_RATE};
use crate::mode::NoiseMode;
use crate::spectral::SpectralSuppressor;

#[cfg(feature = "rnnoise")]
use crate::neural::Rnnoise;

/// One-pole high-pass around 100 Hz at 48 kHz; removes rumble before suppression.
const HIGH_PASS_ALPHA: f32 = 0.98708;
const LIMITER_CEILING: f32 = 0.98;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub input_rms: f32,
    pub output_rms: f32,
    pub voice_active: bool,
    /// Voice probability 0..1 (RNNoise's estimate; 0/1 on the spectral path).
    pub vad: f32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProcessError {
    /// `input`/`output` length does not match the processor's frame length.
    WrongFrameLength { expected: usize, got: usize },
}

pub struct NoiseProcessor {
    mode: NoiseMode,
    frame_length: usize,
    spectral: SpectralSuppressor,
    hp_prev_input: f32,
    hp_prev_output: f32,
    metrics: Metrics,
    scratch_in: Vec<f32>,
    #[cfg(feature = "rnnoise")]
    neural: Option<Rnnoise>,
    #[cfg(feature = "rnnoise")]
    scratch_out: Vec<f32>,
    #[cfg(feature = "rnnoise")]
    gate_gain: f32,
}

impl NoiseProcessor {
    /// Creates a processor. The neural backend is used only at 48 kHz with
    /// 480-sample frames (RNNoise's native geometry); other geometries fall
    /// back to the spectral suppressor.
    pub fn new(mode: NoiseMode, sample_rate: u32, frame_length: usize) -> Option<Self> {
        if sample_rate == 0 || frame_length == 0 {
            return None;
        }
        Some(Self {
            mode,
            frame_length,
            spectral: SpectralSuppressor::new(),
            hp_prev_input: 0.0,
            hp_prev_output: 0.0,
            metrics: Metrics::default(),
            scratch_in: vec![0.0; frame_length],
            #[cfg(feature = "rnnoise")]
            neural: if sample_rate == SAMPLE_RATE && frame_length == FRAME_LENGTH {
                Rnnoise::new()
            } else {
                None
            },
            #[cfg(feature = "rnnoise")]
            scratch_out: vec![0.0; frame_length],
            #[cfg(feature = "rnnoise")]
            gate_gain: 1.0,
        })
    }

    /// Convenience constructor for the standard 48 kHz / 480-sample geometry.
    pub fn standard(mode: NoiseMode) -> Self {
        Self::new(mode, SAMPLE_RATE, FRAME_LENGTH).expect("standard geometry is valid")
    }

    /// True when the neural (RNNoise) backend is compiled in.
    pub fn neural_available() -> bool {
        cfg!(feature = "rnnoise")
    }

    /// True when this instance is running the neural backend.
    pub fn uses_neural(&self) -> bool {
        #[cfg(feature = "rnnoise")]
        {
            self.neural.is_some()
        }
        #[cfg(not(feature = "rnnoise"))]
        {
            false
        }
    }

    pub fn engine_name(&self) -> &'static str {
        if self.uses_neural() {
            "RNNoise (neural)"
        } else {
            "Spectral DSP"
        }
    }

    pub fn mode(&self) -> NoiseMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: NoiseMode) {
        self.mode = mode;
    }

    pub fn metrics(&self) -> Metrics {
        self.metrics
    }

    pub fn frame_length(&self) -> usize {
        self.frame_length
    }

    pub fn reset(&mut self) {
        self.spectral.reset();
        self.hp_prev_input = 0.0;
        self.hp_prev_output = 0.0;
        self.metrics = Metrics::default();
        #[cfg(feature = "rnnoise")]
        {
            // RNNoise has no reset entry point; recreate the state.
            if self.neural.is_some() {
                self.neural = Rnnoise::new();
            }
            self.gate_gain = 1.0;
        }
    }

    fn high_pass(&mut self, sample: f32) -> f32 {
        let out = HIGH_PASS_ALPHA * (self.hp_prev_output + sample - self.hp_prev_input);
        self.hp_prev_input = sample;
        self.hp_prev_output = out;
        out
    }

    /// Processes exactly one frame. Non-finite input samples are treated as
    /// silence so a single NaN can never poison the filter state.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), ProcessError> {
        if input.len() != self.frame_length {
            return Err(ProcessError::WrongFrameLength {
                expected: self.frame_length,
                got: input.len(),
            });
        }
        if output.len() != self.frame_length {
            return Err(ProcessError::WrongFrameLength {
                expected: self.frame_length,
                got: output.len(),
            });
        }

        self.metrics.input_rms = rms(input);

        for (i, &raw) in input.iter().enumerate() {
            let sample = if raw.is_finite() { raw } else { 0.0 };
            self.scratch_in[i] = self.high_pass(sample);
        }

        #[cfg(feature = "rnnoise")]
        if self.neural.is_some() {
            self.process_neural(output);
            return Ok(());
        }

        let voice = self.spectral.process(&self.scratch_in, output, self.mode);

        limit(output, LIMITER_CEILING);
        self.metrics.output_rms = rms(output);
        self.metrics.voice_active = voice;
        self.metrics.vad = if voice { 1.0 } else { 0.0 };
        Ok(())
    }

    #[cfg(feature = "rnnoise")]
    fn process_neural(&mut self, output: &mut [f32]) {
        for s in self.scratch_in.iter_mut() {
            *s *= 32768.0;
        }
        let neural = self.neural.as_mut().expect("checked by caller");
        let vad = neural.process_frame(&self.scratch_in, &mut self.scratch_out);

        let wet = self.mode.wet_mix();
        let dry = 1.0 - wet;
        let target_gate = if vad >= self.mode.vad_open_threshold() {
            1.0
        } else {
            self.mode.silence_gain()
        };
        let smoothing = if target_gate > self.gate_gain {
            0.55
        } else {
            0.12
        };
        self.gate_gain += smoothing * (target_gate - self.gate_gain);

        for ((out, &wet_sample), &dry_sample) in output
            .iter_mut()
            .zip(&self.scratch_out)
            .zip(&self.scratch_in)
        {
            *out = self.gate_gain * ((wet * wet_sample + dry * dry_sample) / 32768.0);
        }
        limit(output, LIMITER_CEILING);
        self.metrics.output_rms = rms(output);
        self.metrics.voice_active = vad > 0.5;
        self.metrics.vad = vad;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic white noise in [-amp, amp].
    fn noise(seed: &mut u32, amp: f32) -> f32 {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        ((*seed >> 8) as f32 / 16_777_216.0 - 0.5) * 2.0 * amp
    }

    fn run(
        processor: &mut NoiseProcessor,
        frames: usize,
        mut sample: impl FnMut(usize) -> f32,
    ) -> (f32, f32) {
        let mut in_energy = 0.0_f64;
        let mut out_energy = 0.0_f64;
        let mut input = vec![0.0; FRAME_LENGTH];
        let mut output = vec![0.0; FRAME_LENGTH];
        for frame in 0..frames {
            for (i, s) in input.iter_mut().enumerate() {
                *s = sample(frame * FRAME_LENGTH + i);
            }
            processor.process(&input, &mut output).unwrap();
            in_energy += input.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
            out_energy += output.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
        }
        let n = (frames * FRAME_LENGTH) as f64;
        (
            (in_energy / n).sqrt() as f32,
            (out_energy / n).sqrt() as f32,
        )
    }

    #[test]
    fn rejects_wrong_frame_length() {
        let mut p = NoiseProcessor::standard(NoiseMode::Balanced);
        let mut out = vec![0.0; FRAME_LENGTH];
        assert_eq!(
            p.process(&[0.0; 100], &mut out),
            Err(ProcessError::WrongFrameLength {
                expected: FRAME_LENGTH,
                got: 100
            })
        );
    }

    #[test]
    fn non_finite_input_never_poisons_output() {
        let mut p = NoiseProcessor::standard(NoiseMode::Max);
        let mut out = vec![0.0; FRAME_LENGTH];
        let mut input = vec![0.1; FRAME_LENGTH];
        input[7] = f32::NAN;
        input[9] = f32::INFINITY;
        for _ in 0..50 {
            p.process(&input, &mut out).unwrap();
            assert!(out.iter().all(|s| s.is_finite()));
        }
    }

    #[test]
    fn suppresses_steady_background_noise() {
        // 3 s of stationary noise: after the noise floor is learnt, output must
        // be well below input.
        let mut p = NoiseProcessor::new(NoiseMode::Max, SAMPLE_RATE, FRAME_LENGTH).unwrap();
        let mut seed = 1;
        run(&mut p, 150, |_| noise(&mut seed, 0.05)); // learn
        let (input_rms, output_rms) = run(&mut p, 100, |_| noise(&mut seed, 0.05));
        // When the neural backend is active the gate/model decide; the spectral
        // fallback must still cut at least ~6 dB.
        assert!(
            output_rms < input_rms * 0.5,
            "input {input_rms}, output {output_rms}"
        );
    }

    #[test]
    fn speech_like_tone_passes_through() {
        let mut p = NoiseProcessor::new(NoiseMode::Balanced, SAMPLE_RATE, FRAME_LENGTH).unwrap();
        if p.uses_neural() {
            // RNNoise is trained on speech and (correctly) treats a pure
            // synthetic tone as noise, so this assertion only applies to the
            // spectral path. Neural behaviour is covered by the C++ parity test.
            return;
        }
        let mut seed = 7;
        run(&mut p, 150, |_| noise(&mut seed, 0.02));
        let (input_rms, output_rms) = run(&mut p, 100, |i| {
            0.3 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin()
                + noise(&mut seed, 0.02)
        });
        assert!(
            output_rms > input_rms * 0.5,
            "input {input_rms}, output {output_rms}"
        );
    }

    #[test]
    fn metrics_and_reset() {
        let mut p = NoiseProcessor::standard(NoiseMode::Balanced);
        let mut seed = 3;
        run(&mut p, 20, |_| noise(&mut seed, 0.1));
        assert!(p.metrics().input_rms > 0.0);
        p.reset();
        assert_eq!(p.metrics(), Metrics::default());
        assert_eq!(p.frame_length(), FRAME_LENGTH);
    }

    #[test]
    fn non_standard_geometry_falls_back_to_spectral() {
        let p = NoiseProcessor::new(NoiseMode::Balanced, 16_000, 160).unwrap();
        assert!(!p.uses_neural());
        assert!(NoiseProcessor::new(NoiseMode::Balanced, 0, 160).is_none());
    }
}
