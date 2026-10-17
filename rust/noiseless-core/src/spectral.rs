//! Streaming spectral-subtraction suppressor (the always-available fallback).
//!
//! 512-point STFT, 50% overlap, square-root Hann window on analysis and
//! synthesis (constant-overlap-add, so unity gain reconstructs the input).

use crate::fft::fft;
use crate::mode::NoiseMode;
use std::collections::VecDeque;
use std::f32::consts::PI;

const FFT_SIZE: usize = 512;
const HOP: usize = 256;
const BINS: usize = FFT_SIZE / 2 + 1;
/// Frames of noise-floor priming before speech detection is trusted.
const PRIMING_HOPS: u32 = 15;

pub struct SpectralSuppressor {
    in_fifo: VecDeque<f32>,
    out_fifo: VecDeque<f32>,
    analysis: [f32; FFT_SIZE],
    ola: [f32; FFT_SIZE],
    window: [f32; FFT_SIZE],
    noise_mag: [f32; BINS],
    gain_smooth: [f32; BINS],
    re: [f32; FFT_SIZE],
    im: [f32; FFT_SIZE],
    frames_seen: u32,
}

impl SpectralSuppressor {
    pub fn new() -> Self {
        let mut window = [0.0_f32; FFT_SIZE];
        for (i, w) in window.iter_mut().enumerate() {
            let hann = 0.5 * (1.0 - (2.0 * PI * i as f32 / FFT_SIZE as f32).cos());
            *w = hann.sqrt();
        }
        Self {
            in_fifo: VecDeque::new(),
            out_fifo: VecDeque::new(),
            analysis: [0.0; FFT_SIZE],
            ola: [0.0; FFT_SIZE],
            window,
            noise_mag: [0.0; BINS],
            gain_smooth: [1.0; BINS],
            re: [0.0; FFT_SIZE],
            im: [0.0; FFT_SIZE],
            frames_seen: 0,
        }
    }

    pub fn reset(&mut self) {
        self.in_fifo.clear();
        self.out_fifo.clear();
        self.analysis = [0.0; FFT_SIZE];
        self.ola = [0.0; FFT_SIZE];
        self.noise_mag = [0.0; BINS];
        self.gain_smooth = [1.0; BINS];
        self.frames_seen = 0;
    }

    /// Consumes one frame of (already sanitised, high-passed) input and fills
    /// `output` with one frame. Output is silent until the STFT latency has
    /// filled. Returns whether speech was detected in this call.
    pub fn process(&mut self, input: &[f32], output: &mut [f32], mode: NoiseMode) -> bool {
        self.in_fifo.extend(input.iter().copied());

        let floor_gain = mode.spectral_floor();
        let oversub = mode.oversubtraction();
        let mut voice_active = false;

        while self.in_fifo.len() >= HOP {
            self.analysis.copy_within(HOP.., 0);
            for slot in self.analysis[FFT_SIZE - HOP..].iter_mut() {
                *slot = self.in_fifo.pop_front().unwrap_or(0.0);
            }

            for i in 0..FFT_SIZE {
                self.re[i] = self.analysis[i] * self.window[i];
                self.im[i] = 0.0;
            }
            fft(&mut self.re, &mut self.im, false);

            // Voice-activity decision from frame energy vs. tracked noise energy.
            let mut frame_energy = 0.0_f64;
            let mut noise_energy = 0.0_f64;
            for f in 0..BINS {
                let mag = (self.re[f] * self.re[f] + self.im[f] * self.im[f]).sqrt();
                frame_energy += f64::from(mag) * f64::from(mag);
                noise_energy += f64::from(self.noise_mag[f]) * f64::from(self.noise_mag[f]);
            }
            let priming = self.frames_seen < PRIMING_HOPS;
            let snr = frame_energy / (noise_energy + 1e-9);
            let speech = !priming && snr > 3.0;
            if speech {
                voice_active = true;
            }

            for f in 0..BINS {
                let mag = (self.re[f] * self.re[f] + self.im[f] * self.im[f]).sqrt();
                if priming {
                    self.noise_mag[f] = 0.9 * self.noise_mag[f] + 0.1 * mag;
                } else if !speech {
                    self.noise_mag[f] = 0.95 * self.noise_mag[f] + 0.05 * mag;
                }

                let mut gain = 1.0 - oversub * (self.noise_mag[f] / (mag + 1e-9));
                if gain < floor_gain {
                    gain = floor_gain;
                } else if gain > 1.0 {
                    gain = 1.0;
                }
                // Temporal smoothing curbs musical noise from bin-to-bin jumps.
                gain = 0.6 * self.gain_smooth[f] + 0.4 * gain;
                self.gain_smooth[f] = gain;

                self.re[f] *= gain;
                self.im[f] *= gain;
                // Mirror onto the conjugate-symmetric bin so the IFFT stays real.
                if f > 0 && f < FFT_SIZE / 2 {
                    self.re[FFT_SIZE - f] *= gain;
                    self.im[FFT_SIZE - f] *= gain;
                }
            }

            fft(&mut self.re, &mut self.im, true);

            for i in 0..FFT_SIZE {
                self.ola[i] += self.re[i] * self.window[i];
            }
            self.out_fifo.extend(self.ola[..HOP].iter().copied());
            self.ola.copy_within(HOP.., 0);
            self.ola[FFT_SIZE - HOP..].fill(0.0);

            self.frames_seen = self.frames_seen.saturating_add(1);
        }

        if self.out_fifo.len() >= output.len() {
            for slot in output.iter_mut() {
                *slot = self.out_fifo.pop_front().unwrap_or(0.0);
            }
        } else {
            output.fill(0.0);
        }
        voice_active
    }
}

impl Default for SpectralSuppressor {
    fn default() -> Self {
        Self::new()
    }
}
