//! Compares the Rust processor against output dumped by the original C++
//! implementation (see `rust/parity/run.sh`). Skipped unless `NL_PARITY_DIR`
//! points at a directory holding the dumps.

use noiseless_core::{NoiseMode, NoiseProcessor, FRAME_LENGTH};
use std::path::PathBuf;

fn lcg_noise(state: &mut u32, amp: f32) -> f32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    ((*state >> 8) as f32 / 16_777_216.0 - 0.5) * 2.0 * amp
}

fn signal_at(state: &mut u32, t: u64) -> f32 {
    let seconds = t as f64 / 48000.0;
    let burst = seconds % 1.0 > 0.5;
    let mut s = lcg_noise(state, 0.03);
    if burst {
        s += (0.3 * (2.0 * std::f64::consts::PI * 440.0 * t as f64 / 48000.0).sin()) as f32;
    }
    s
}

fn read_f32(path: &PathBuf) -> Vec<f32> {
    std::fs::read(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn compare(label: &str, file: &str, mode: NoiseMode, tolerance: f32) {
    let Some(dir) = std::env::var_os("NL_PARITY_DIR") else {
        eprintln!("NL_PARITY_DIR not set; skipping {label}");
        return;
    };
    let expected = read_f32(&PathBuf::from(dir).join(file));
    let frames = expected.len() / FRAME_LENGTH;
    assert!(frames > 0, "empty reference dump");

    let mut processor = NoiseProcessor::standard(mode);
    if cfg!(feature = "rnnoise") {
        assert!(
            processor.uses_neural(),
            "rnnoise feature on but processor is not neural"
        );
    }
    let mut state = 12345u32;
    let mut input = vec![0.0; FRAME_LENGTH];
    let mut output = vec![0.0; FRAME_LENGTH];
    let mut max_err = 0.0_f32;
    let mut worst = 0usize;
    for f in 0..frames {
        for (i, s) in input.iter_mut().enumerate() {
            *s = signal_at(&mut state, (f * FRAME_LENGTH + i) as u64);
        }
        processor.process(&input, &mut output).unwrap();
        for (i, (a, b)) in output.iter().zip(&expected[f * FRAME_LENGTH..]).enumerate() {
            let err = (a - b).abs();
            if err > max_err {
                max_err = err;
                worst = f * FRAME_LENGTH + i;
            }
        }
    }
    eprintln!("{label}: {frames} frames, max |rust - c++| = {max_err:e} at sample {worst}");
    assert!(
        max_err <= tolerance,
        "{label}: max error {max_err:e} exceeds {tolerance:e} (sample {worst})"
    );
}

#[cfg(not(feature = "rnnoise"))]
#[test]
fn spectral_matches_cpp() {
    compare(
        "spectral/balanced",
        "spectral-1.f32",
        NoiseMode::Balanced,
        1e-4,
    );
    compare("spectral/light", "spectral-0.f32", NoiseMode::Light, 1e-4);
    compare("spectral/max", "spectral-2.f32", NoiseMode::Max, 1e-4);
}

#[cfg(feature = "rnnoise")]
#[test]
fn neural_matches_cpp() {
    compare(
        "rnnoise/balanced",
        "neural-1.f32",
        NoiseMode::Balanced,
        1e-4,
    );
    compare("rnnoise/light", "neural-0.f32", NoiseMode::Light, 1e-4);
    compare("rnnoise/max", "neural-2.f32", NoiseMode::Max, 1e-4);
}
