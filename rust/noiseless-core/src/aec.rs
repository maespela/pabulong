//! Acoustic echo canceller.
//!
//! Removes the far end's voice (what the speakers are playing) from the mic.
//! The reference is the system output mix: a Core Audio process tap on macOS,
//! WASAPI loopback on Windows. It must run *before* any non-linear processing
//! (RNNoise, gates): a gated/denoised echo cannot be modelled by any linear
//! canceller, including the one inside the call app.
//!
//! Structure:
//! - Partitioned-block frequency-domain adaptive filter (overlap-save, NLMS
//!   normalised per bin), block 256 / FFT 512, tail `tail_ms` long.
//! - Two filters (foreground/background): the background adapts every block;
//!   it is copied to the foreground only when it is measurably better, and is
//!   reset from the foreground when it diverges (e.g. during double talk). The
//!   output always uses the foreground, so double talk never corrupts it.
//! - Residual echo suppressor: a spectral gain on the linear error, driven by
//!   the part of the error that is coherent with the echo estimate plus a small
//!   allowance for loudspeaker non-linearity.
//!
//! Latency: 2 blocks (512 samples, 10.7 ms at 48 kHz).

use crate::fft::fft;
use std::collections::VecDeque;
use std::f32::consts::PI;

const B: usize = 256; // block (hop) size
const N: usize = 2 * B; // FFT size
const BINS: usize = N / 2 + 1;
/// Partitions whose gradient is constrained per block (rotating).
const CONSTRAIN_PER_BLOCK: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AecMetrics {
    /// Echo removed by the linear filter (dB), measured while the far end is
    /// talking alone. 0 until measured.
    pub erle_db: f32,
    /// Bulk delay of the echo path the filter has found (ms).
    pub delay_ms: f32,
    /// True when the reference is carrying signal.
    pub far_end_active: bool,
}

struct Filter {
    re: Vec<f32>, // [partition * BINS + bin]
    im: Vec<f32>,
}

impl Filter {
    fn new(partitions: usize) -> Self {
        Self { re: vec![0.0; partitions * BINS], im: vec![0.0; partitions * BINS] }
    }
    fn copy_from(&mut self, other: &Filter) {
        self.re.copy_from_slice(&other.re);
        self.im.copy_from_slice(&other.im);
    }
    fn clear(&mut self) {
        self.re.fill(0.0);
        self.im.fill(0.0);
    }
}

pub struct EchoCanceller {
    partitions: usize,
    // Reference spectra history, ring of `partitions`; `head` is the newest.
    x_re: Vec<f32>,
    x_im: Vec<f32>,
    x_pow: Vec<f32>,
    x_pow_sum: [f32; BINS],
    head: usize,
    blocks_since_resum: u32,
    ref_hist: [f32; N],

    fg: Filter,
    bg: Filter,
    constrain_cursor: usize,

    // Error/echo energies (smoothed) for the foreground/background decision.
    s_fg: f32,
    s_bg: f32,
    s_mic: f32,
    s_ref: f32,
    better_count: u32,

    // Residual echo suppressor state.
    window: [f32; N],
    e_hist: [f32; N],
    y_hist: [f32; N],
    ola: [f32; N],
    cey_re: [f32; BINS],
    cey_im: [f32; BINS],
    pyy: [f32; BINS],
    pee: [f32; BINS],
    res_gain: [f32; BINS],

    // Frame <-> block adapters.
    mic_fifo: VecDeque<f32>,
    ref_fifo: VecDeque<f32>,
    out_fifo: VecDeque<f32>,

    // Scratch.
    re: [f32; N],
    im: [f32; N],
    y_fg: [f32; B],
    y_bg: [f32; B],
    e_fg: [f32; B],
    e_bg: [f32; B],

    metrics: AecMetrics,
    erle_mic: f32,
    erle_err: f32,
}

impl EchoCanceller {
    /// `tail_ms` is the longest echo path delay + reverberation the filter can
    /// model (including any misalignment between the reference and the mic).
    pub fn new(tail_ms: f32, sample_rate: u32) -> Self {
        let samples = (tail_ms / 1000.0 * sample_rate as f32).max(B as f32);
        let partitions = (samples / B as f32).ceil() as usize;
        let mut window = [0.0_f32; N];
        for (i, w) in window.iter_mut().enumerate() {
            *w = (0.5 * (1.0 - (2.0 * PI * i as f32 / N as f32).cos())).sqrt();
        }
        let mut s = Self {
            partitions,
            x_re: vec![0.0; partitions * BINS],
            x_im: vec![0.0; partitions * BINS],
            x_pow: vec![0.0; partitions * BINS],
            x_pow_sum: [0.0; BINS],
            head: 0,
            blocks_since_resum: 0,
            ref_hist: [0.0; N],
            fg: Filter::new(partitions),
            bg: Filter::new(partitions),
            constrain_cursor: 0,
            s_fg: 0.0,
            s_bg: 0.0,
            s_mic: 0.0,
            s_ref: 0.0,
            better_count: 0,
            window,
            e_hist: [0.0; N],
            y_hist: [0.0; N],
            ola: [0.0; N],
            cey_re: [0.0; BINS],
            cey_im: [0.0; BINS],
            pyy: [0.0; BINS],
            pee: [0.0; BINS],
            res_gain: [1.0; BINS],
            mic_fifo: VecDeque::new(),
            ref_fifo: VecDeque::new(),
            out_fifo: VecDeque::new(),
            re: [0.0; N],
            im: [0.0; N],
            y_fg: [0.0; B],
            y_bg: [0.0; B],
            e_fg: [0.0; B],
            e_bg: [0.0; B],
            metrics: AecMetrics::default(),
            erle_mic: 0.0,
            erle_err: 0.0,
        };
        s.reset();
        s
    }

    /// Samples of latency this stage adds.
    pub const LATENCY: usize = 2 * B;

    pub fn metrics(&self) -> AecMetrics {
        self.metrics
    }

    pub fn reset(&mut self) {
        self.x_re.fill(0.0);
        self.x_im.fill(0.0);
        self.x_pow.fill(0.0);
        self.x_pow_sum = [0.0; BINS];
        self.ref_hist = [0.0; N];
        self.fg.clear();
        self.bg.clear();
        self.s_fg = 0.0;
        self.s_bg = 0.0;
        self.s_mic = 0.0;
        self.s_ref = 0.0;
        self.better_count = 0;
        self.e_hist = [0.0; N];
        self.y_hist = [0.0; N];
        self.ola = [0.0; N];
        self.cey_re = [0.0; BINS];
        self.cey_im = [0.0; BINS];
        self.pyy = [0.0; BINS];
        self.pee = [0.0; BINS];
        self.res_gain = [1.0; BINS];
        self.mic_fifo.clear();
        self.ref_fifo.clear();
        self.out_fifo.clear();
        // Prime the output with one block so every call returns a full frame.
        self.out_fifo.extend(std::iter::repeat_n(0.0, B));
        self.metrics = AecMetrics::default();
        self.erle_mic = 0.0;
        self.erle_err = 0.0;
    }

    /// Processes one frame of mic audio against the time-aligned reference
    /// (`reference.len() == mic.len()`), writing the echo-cancelled frame to
    /// `out`. Any frame length works; output is delayed by `LATENCY`.
    pub fn process(&mut self, mic: &[f32], reference: &[f32], out: &mut [f32]) {
        debug_assert_eq!(mic.len(), reference.len());
        debug_assert_eq!(mic.len(), out.len());
        self.mic_fifo.extend(mic.iter().copied());
        self.ref_fifo.extend(reference.iter().copied());

        let mut d = [0.0_f32; B];
        let mut x = [0.0_f32; B];
        while self.mic_fifo.len() >= B {
            for i in 0..B {
                d[i] = self.mic_fifo.pop_front().unwrap_or(0.0);
                x[i] = self.ref_fifo.pop_front().unwrap_or(0.0);
            }
            self.process_block(&d, &x);
        }

        for o in out.iter_mut() {
            *o = self.out_fifo.pop_front().unwrap_or(0.0);
        }
    }

    fn slot(&self, age: usize) -> usize {
        // Partition holding the reference spectrum `age` blocks old.
        (self.head + self.partitions - age) % self.partitions
    }

    fn process_block(&mut self, d: &[f32; B], x: &[f32; B]) {
        let p_count = self.partitions;

        // 1. New reference spectrum from the last two blocks (overlap-save).
        self.ref_hist.copy_within(B.., 0);
        self.ref_hist[B..].copy_from_slice(x);
        self.re.copy_from_slice(&self.ref_hist);
        self.im.fill(0.0);
        fft(&mut self.re, &mut self.im, false);

        self.head = (self.head + 1) % p_count;
        let base = self.head * BINS;
        for k in 0..BINS {
            let pw = self.re[k] * self.re[k] + self.im[k] * self.im[k];
            self.x_pow_sum[k] += pw - self.x_pow[base + k];
            self.x_re[base + k] = self.re[k];
            self.x_im[base + k] = self.im[k];
            self.x_pow[base + k] = pw;
        }
        // Periodically resum to stop floating-point drift in the running sum.
        self.blocks_since_resum += 1;
        if self.blocks_since_resum >= 128 {
            self.blocks_since_resum = 0;
            self.x_pow_sum = [0.0; BINS];
            for p in 0..p_count {
                for k in 0..BINS {
                    self.x_pow_sum[k] += self.x_pow[p * BINS + k];
                }
            }
        }

        // 2. Echo estimates and errors for both filters.
        self.estimate(true);
        self.estimate(false);
        let mut e_fg_energy = 0.0_f32;
        let mut e_bg_energy = 0.0_f32;
        let mut mic_energy = 0.0_f32;
        let mut ref_energy = 0.0_f32;
        let mut echo_energy = 0.0_f32;
        for i in 0..B {
            echo_energy += self.y_fg[i] * self.y_fg[i];
            self.e_fg[i] = d[i] - self.y_fg[i];
            self.e_bg[i] = d[i] - self.y_bg[i];
            e_fg_energy += self.e_fg[i] * self.e_fg[i];
            e_bg_energy += self.e_bg[i] * self.e_bg[i];
            mic_energy += d[i] * d[i];
            ref_energy += x[i] * x[i];
        }

        // 3. Adapt the background filter (NLMS, per-bin normalised).
        self.adapt_background();

        // 4. Foreground/background control.
        let a = 0.7;
        self.s_fg = a * self.s_fg + (1.0 - a) * e_fg_energy;
        self.s_bg = a * self.s_bg + (1.0 - a) * e_bg_energy;
        self.s_mic = a * self.s_mic + (1.0 - a) * mic_energy;
        self.s_ref = 0.9 * self.s_ref + 0.1 * ref_energy;
        let far_active = self.s_ref > 1e-7 * B as f32; // ~ -70 dBFS
        if far_active {
            if self.s_bg < 0.75 * self.s_fg && self.s_bg < self.s_mic {
                self.better_count += 1;
                if self.better_count >= 2 {
                    self.fg.copy_from(&self.bg);
                    self.s_fg = self.s_bg;
                    self.better_count = 0;
                }
            } else {
                self.better_count = 0;
            }
            // Background has diverged (typically double talk): restart it
            // from the foreground so it re-converges quickly.
            if self.s_bg > 4.0 * self.s_fg.max(1e-12) && self.s_bg > self.s_mic {
                self.bg.copy_from(&self.fg);
                self.s_bg = self.s_fg;
            }
        }
        // A foreground that makes things worse than no cancellation is discarded.
        if self.s_fg > 2.0 * self.s_mic && self.s_mic > 0.0 {
            self.fg.clear();
            self.s_fg = self.s_mic;
        }

        // 5. Residual echo suppression + synthesis.
        self.suppress_residual(far_active);

        // Metrics.
        // Measure only while the mic is mostly echo (far end talking alone);
        // during double talk the ratio would describe your voice, not echo.
        if far_active && echo_energy > 0.5 * mic_energy {
            self.erle_mic = 0.95 * self.erle_mic + 0.05 * mic_energy;
            self.erle_err = 0.95 * self.erle_err + 0.05 * e_fg_energy;
            self.metrics.erle_db = 10.0 * ((self.erle_mic + 1e-12) / (self.erle_err + 1e-12)).log10();
        }
        self.metrics.far_end_active = far_active;
        self.metrics.delay_ms = self.dominant_partition() as f32 * B as f32 / 48.0;
    }

    /// Computes the echo estimate of the foreground (`true`) or background
    /// filter for the current block into `y_fg` / `y_bg`.
    fn estimate(&mut self, foreground: bool) {
        let filter = if foreground { &self.fg } else { &self.bg };
        let mut acc_re = [0.0_f32; BINS];
        let mut acc_im = [0.0_f32; BINS];
        for age in 0..self.partitions {
            let xs = self.slot(age) * BINS;
            let ws = age * BINS;
            for k in 0..BINS {
                let (xr, xi) = (self.x_re[xs + k], self.x_im[xs + k]);
                let (wr, wi) = (filter.re[ws + k], filter.im[ws + k]);
                acc_re[k] += wr * xr - wi * xi;
                acc_im[k] += wr * xi + wi * xr;
            }
        }
        spectrum_to_full(&acc_re, &acc_im, &mut self.re, &mut self.im);
        fft(&mut self.re, &mut self.im, true);
        let y = if foreground { &mut self.y_fg } else { &mut self.y_bg };
        y.copy_from_slice(&self.re[B..]);
    }

    fn adapt_background(&mut self) {
        // Error spectrum of [0; B] ++ e_bg.
        self.re[..B].fill(0.0);
        self.re[B..].copy_from_slice(&self.e_bg);
        self.im.fill(0.0);
        fft(&mut self.re, &mut self.im, false);
        let e_re: [f32; BINS] = std::array::from_fn(|k| self.re[k]);
        let e_im: [f32; BINS] = std::array::from_fn(|k| self.im[k]);

        let mu = 0.5_f32;
        let delta = 1e-4 * self.partitions as f32;
        let mut step = [0.0_f32; BINS];
        for k in 0..BINS {
            step[k] = mu / (self.x_pow_sum[k] + delta);
        }
        for age in 0..self.partitions {
            let xs = self.slot(age) * BINS;
            let ws = age * BINS;
            for k in 0..BINS {
                let (xr, xi) = (self.x_re[xs + k], self.x_im[xs + k]);
                // E * conj(X)
                let gr = e_re[k] * xr + e_im[k] * xi;
                let gi = e_im[k] * xr - e_re[k] * xi;
                self.bg.re[ws + k] += step[k] * gr;
                self.bg.im[ws + k] += step[k] * gi;
            }
        }

        // Gradient constraint on a rotating subset of partitions: keep only the
        // first half of each partition's impulse response (linear convolution).
        for _ in 0..CONSTRAIN_PER_BLOCK.min(self.partitions) {
            let ws = self.constrain_cursor * BINS;
            let (wr, wi) = (
                std::array::from_fn::<f32, BINS, _>(|k| self.bg.re[ws + k]),
                std::array::from_fn::<f32, BINS, _>(|k| self.bg.im[ws + k]),
            );
            spectrum_to_full(&wr, &wi, &mut self.re, &mut self.im);
            fft(&mut self.re, &mut self.im, true);
            self.re[B..].fill(0.0);
            self.im.fill(0.0);
            fft(&mut self.re, &mut self.im, false);
            for k in 0..BINS {
                self.bg.re[ws + k] = self.re[k];
                self.bg.im[ws + k] = self.im[k];
            }
            self.constrain_cursor = (self.constrain_cursor + 1) % self.partitions;
        }
    }

    fn suppress_residual(&mut self, far_active: bool) {
        // Windowed analysis of the last two blocks of error and echo estimate.
        self.e_hist.copy_within(B.., 0);
        self.e_hist[B..].copy_from_slice(&self.e_fg);
        self.y_hist.copy_within(B.., 0);
        self.y_hist[B..].copy_from_slice(&self.y_fg);

        let mut y_re = [0.0_f32; N];
        let mut y_im = [0.0_f32; N];
        for i in 0..N {
            y_re[i] = self.y_hist[i] * self.window[i];
            self.re[i] = self.e_hist[i] * self.window[i];
            self.im[i] = 0.0;
        }
        fft(&mut y_re, &mut y_im, false);
        fft(&mut self.re, &mut self.im, false);

        let a = 0.75_f32; // statistics smoothing
        let nonlinear_allowance = 0.06_f32; // ~ -12 dB of the echo estimate
        let oversub = 1.6_f32;
        let floor = 0.03_f32; // -30 dB
        for k in 0..BINS {
            let (er, ei, yr, yi) = (self.re[k], self.im[k], y_re[k], y_im[k]);
            let yy = yr * yr + yi * yi;
            let ee = er * er + ei * ei;
            self.pyy[k] = a * self.pyy[k] + (1.0 - a) * yy;
            self.pee[k] = a * self.pee[k] + (1.0 - a) * ee;
            // E * conj(Y)
            self.cey_re[k] = a * self.cey_re[k] + (1.0 - a) * (er * yr + ei * yi);
            self.cey_im[k] = a * self.cey_im[k] + (1.0 - a) * (ei * yr - er * yi);

            let mut target = 1.0;
            if far_active && self.pyy[k] > 1e-12 {
                let coherent = (self.cey_re[k] * self.cey_re[k] + self.cey_im[k] * self.cey_im[k]) / self.pyy[k];
                let residual = coherent + nonlinear_allowance * self.pyy[k];
                target = (1.0 - oversub * residual / (self.pee[k] + 1e-12)).clamp(floor, 1.0);
            }
            // Fast to suppress, slower to release (avoids residual "chirps").
            let g = self.res_gain[k];
            self.res_gain[k] = if target < g { 0.5 * g + 0.5 * target } else { 0.85 * g + 0.15 * target };
        }
        // Light smoothing across frequency limits musical noise.
        let raw = self.res_gain;
        for k in 1..BINS - 1 {
            self.res_gain[k] = 0.25 * raw[k - 1] + 0.5 * raw[k] + 0.25 * raw[k + 1];
        }

        for k in 0..BINS {
            let g = self.res_gain[k];
            self.re[k] *= g;
            self.im[k] *= g;
            if k > 0 && k < N / 2 {
                self.re[N - k] *= g;
                self.im[N - k] *= g;
            }
        }
        fft(&mut self.re, &mut self.im, true);
        for i in 0..N {
            self.ola[i] += self.re[i] * self.window[i];
        }
        self.out_fifo.extend(self.ola[..B].iter().copied());
        self.ola.copy_within(B.., 0);
        self.ola[B..].fill(0.0);
    }

    fn dominant_partition(&self) -> usize {
        let mut best = 0;
        let mut best_energy = 0.0_f32;
        for age in 0..self.partitions {
            let ws = age * BINS;
            let energy: f32 = (0..BINS).map(|k| self.fg.re[ws + k].powi(2) + self.fg.im[ws + k].powi(2)).sum();
            if energy > best_energy {
                best_energy = energy;
                best = age;
            }
        }
        best
    }
}

/// Expands a half spectrum (0..=N/2) into a full conjugate-symmetric spectrum.
fn spectrum_to_full(half_re: &[f32; BINS], half_im: &[f32; BINS], re: &mut [f32; N], im: &mut [f32; N]) {
    re[..BINS].copy_from_slice(half_re);
    im[..BINS].copy_from_slice(half_im);
    for k in 1..N / 2 {
        re[N - k] = half_re[k];
        im[N - k] = -half_im[k];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (*seed >> 8) as f32 / 16_777_216.0 - 0.5
    }

    /// Echo = reference delayed 30 ms through a short decaying room response.
    fn run(seconds: f32, near: impl Fn(usize) -> f32) -> (f32, f32, f32) {
        let mut aec = EchoCanceller::new(200.0, 48_000);
        let total = (seconds * 48_000.0) as usize;
        let mut seed = 9;
        let reference: Vec<f32> = (0..total).map(|_| 0.3 * noise(&mut seed)).collect();
        let delay = 1440; // 30 ms
        let taps: Vec<f32> = (0..400).map(|i| 0.6 * (-(i as f32) / 60.0).exp() * if i % 7 == 0 { -1.0 } else { 1.0 }).collect();
        let mut mic = vec![0.0_f32; total];
        for t in 0..total {
            let mut e = 0.0;
            for (j, h) in taps.iter().enumerate() {
                if t >= delay + j {
                    e += h * reference[t - delay - j];
                }
            }
            mic[t] = e + near(t);
        }
        let mut out = vec![0.0_f32; total];
        for f in 0..total / 480 {
            let r = f * 480..(f + 1) * 480;
            aec.process(&mic[r.clone()], &reference[r.clone()], &mut out[r]);
        }
        // Energies over the last second (after convergence), aligned for latency.
        let start = total - 48_000;
        let mic_e: f32 = mic[start - EchoCanceller::LATENCY..total - EchoCanceller::LATENCY].iter().map(|s| s * s).sum();
        let out_e: f32 = out[start..].iter().map(|s| s * s).sum();
        (mic_e, out_e, aec.metrics().delay_ms)
    }

    #[test]
    fn cancels_echo_with_delay() {
        let (mic_e, out_e, delay) = run(6.0, |_| 0.0);
        let erle = 10.0 * (mic_e / out_e.max(1e-12)).log10();
        assert!(erle > 25.0, "ERLE only {erle:.1} dB");
        assert!((delay - 30.0).abs() < 8.0, "found delay {delay} ms, expected ~30");
    }

    #[test]
    fn passes_near_speech_when_far_end_silent() {
        let mut aec = EchoCanceller::new(200.0, 48_000);
        let mut seed = 5;
        let mic: Vec<f32> = (0..48_000).map(|_| 0.2 * noise(&mut seed)).collect();
        let reference = vec![0.0; 48_000];
        let mut out = vec![0.0; 48_000];
        for f in 0..100 {
            let r = f * 480..(f + 1) * 480;
            aec.process(&mic[r.clone()], &reference[r.clone()], &mut out[r]);
        }
        // Without a reference the canceller must be transparent (pure delay).
        for t in 2000..48_000 {
            assert!((out[t] - mic[t - EchoCanceller::LATENCY]).abs() < 1e-4, "sample {t}");
        }
    }
}
