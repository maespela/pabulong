//! In-place iterative radix-2 FFT. Mirrors the C++ implementation operation for
//! operation (including the twiddle recurrence) so the Rust and C++ suppressors
//! produce matching output.

use std::f32::consts::PI;

/// Transforms `re`/`im` in place. `inverse` computes the IFFT with 1/N scaling.
/// The length must be a power of two and `re.len() == im.len()`.
pub fn fft(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
    debug_assert_eq!(n, im.len());
    debug_assert!(n.is_power_of_two());

    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let mut len = 2usize;
    while len <= n {
        let sign = if inverse { 1.0_f32 } else { -1.0_f32 };
        let ang = 2.0 * PI / len as f32 * sign;
        let wr = ang.cos();
        let wi = ang.sin();
        let half = len / 2;
        let mut i = 0usize;
        while i < n {
            let mut cwr = 1.0_f32;
            let mut cwi = 0.0_f32;
            for k in 0..half {
                let xr = re[i + k + half] * cwr - im[i + k + half] * cwi;
                let xi = re[i + k + half] * cwi + im[i + k + half] * cwr;
                let ur = re[i + k];
                let ui = im[i + k];
                re[i + k] = ur + xr;
                im[i + k] = ui + xi;
                re[i + k + half] = ur - xr;
                im[i + k + half] = ui - xi;
                let ncwr = cwr * wr - cwi * wi;
                cwi = cwr * wi + cwi * wr;
                cwr = ncwr;
            }
            i += len;
        }
        len <<= 1;
    }

    if inverse {
        let scale = n as f32;
        for i in 0..n {
            re[i] /= scale;
            im[i] /= scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_then_inverse_recovers_signal() {
        let n = 512;
        let original: Vec<f32> = (0..n)
            .map(|i| ((i * 7 % 31) as f32 - 15.0) / 15.0)
            .collect();
        let mut re = original.clone();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        for (a, b) in original.iter().zip(&re) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
    }

    #[test]
    fn pure_tone_lands_in_expected_bin() {
        let n = 512;
        let bin = 20usize;
        let mut re: Vec<f32> = (0..n)
            .map(|i| (2.0 * PI * bin as f32 * i as f32 / n as f32).sin())
            .collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im, false);
        let mags: Vec<f32> = (0..n / 2)
            .map(|f| (re[f] * re[f] + im[f] * im[f]).sqrt())
            .collect();
        let peak = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert_eq!(peak, bin);
    }
}
