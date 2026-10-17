//! Audio frame geometry and small sample-level helpers.

/// Sample rate the processing path runs at.
pub const SAMPLE_RATE: u32 = 48_000;
/// Samples per processing frame (10 ms at 48 kHz). Matches RNNoise's frame size.
pub const FRAME_LENGTH: usize = 480;

/// Root-mean-square level of `samples` (0.0 for an empty slice).
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_squares: f32 = samples.iter().map(|s| s * s).sum();
    (sum_squares / samples.len() as f32).sqrt()
}

/// Scales `samples` down so no sample exceeds `ceiling`. A no-op when the peak
/// is already at or below the ceiling (or `ceiling` is not positive).
pub fn limit(samples: &mut [f32], ceiling: f32) {
    if samples.is_empty() || ceiling <= 0.0 {
        return;
    }
    let peak = samples.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
    if peak <= ceiling {
        return;
    }
    let gain = ceiling / peak;
    for s in samples.iter_mut() {
        *s *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_constant_signal() {
        assert!((rms(&[0.5; 480]) - 0.5).abs() < 1e-6);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn limiter_caps_peak_and_preserves_shape() {
        let mut samples = [0.0, 2.0, -4.0, 1.0];
        limit(&mut samples, 0.98);
        let peak = samples.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
        assert!((peak - 0.98).abs() < 1e-6);
        assert!(
            (samples[1] / samples[3] - 2.0).abs() < 1e-5,
            "relative levels must be preserved"
        );
    }

    #[test]
    fn limiter_leaves_quiet_audio_untouched() {
        let mut samples = [0.1, -0.2, 0.3];
        limit(&mut samples, 0.98);
        assert_eq!(samples, [0.1, -0.2, 0.3]);
    }
}
