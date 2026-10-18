//! The per-frame work of the capture thread: framing, the shared chain (or
//! bypass), then the virtual-mic ring and the recorder. Platform-independent
//! so it is tested end to end on any OS.

use crate::chain::{VoiceChain, FRAME_LENGTH};
use crate::framer::Framer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct MicPath {
    framer: Framer,
    chain: Arc<VoiceChain>,
    bypass: Arc<AtomicBool>,
    out: Vec<f32>,
}

impl MicPath {
    pub fn new(chain: Arc<VoiceChain>, bypass: Arc<AtomicBool>) -> Self {
        Self { framer: Framer::default(), chain, bypass, out: vec![0.0; FRAME_LENGTH] }
    }

    /// 48 kHz mono samples whose first sample was captured at `capture_ns`.
    /// `sink` receives each finished 480-sample frame.
    pub fn push(&mut self, samples: &[f32], capture_ns: u64, mut sink: impl FnMut(&[f32])) {
        let chain = &self.chain;
        let bypass = self.bypass.load(Ordering::Relaxed);
        let out = &mut self.out;
        self.framer.push(samples, capture_ns, |frame, frame_ns| {
            if bypass || !chain.process(frame, out, frame_ns) {
                out.copy_from_slice(frame);
            }
            sink(out);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{SuppressionMode, VoiceIsolation};
    use pabulong_ring::Ring;

    /// Deterministic speech-band noise.
    fn noise(n: usize, seed: &mut u32) -> Vec<f32> {
        let mut prev = 0.0f32;
        (0..n)
            .map(|_| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let white = ((*seed >> 8) as f32 / 16_777_216.0 - 0.5) * 0.6;
                prev = 0.7 * prev + white; // tilt towards low frequencies
                prev
            })
            .collect()
    }

    #[test]
    fn speaker_echo_is_removed_through_the_host_path_and_reaches_the_driver_ring() {
        // Speakers play noise; the mic hears it 30 ms later at half level. The
        // loopback delivers 10 ms chunks with their play time; the mic delivers
        // 512-sample WASAPI-sized chunks. Everything uses QPC-style ns.
        let chain = Arc::new(VoiceChain::new(SuppressionMode::Balanced, true, VoiceIsolation::Off));
        let mut path = MicPath::new(chain.clone(), Arc::new(AtomicBool::new(false)));
        let mut mem = vec![0u64; 65_536 / 8];
        let ring = unsafe { Ring::initialize(mem.as_mut_ptr() as *mut u8, 65_536, 1).unwrap() };
        ring.set_producer_active(true);

        let seconds = 12;
        let total = 48_000 * seconds;
        let mut seed = 7;
        let far = noise(total, &mut seed);
        let delay = 48 * 30;
        let mic: Vec<f32> = (0..total).map(|i| if i >= delay { 0.5 * far[i - delay] } else { 0.0 }).collect();
        let ns = |sample: usize| (sample as f64 * 1e9 / 48_000.0) as u64 + 5_000_000_000;

        let (mut in_energy, mut out_energy) = (0.0f64, 0.0f64);
        let mut frames = 0usize;
        let mut read_back = vec![0.0f32; FRAME_LENGTH];
        let mut pos = 0;
        let mut ref_pos = 0;
        while pos + 512 <= total {
            // Reference arrives a little ahead of the mic, in 480-sample chunks.
            while ref_pos + 480 <= (pos + 960).min(total) {
                chain.push_reference(&far[ref_pos..ref_pos + 480], ns(ref_pos));
                ref_pos += 480;
            }
            let start_frame = frames;
            path.push(&mic[pos..pos + 512], ns(pos), |out| {
                assert!(ring.write_frame(out));
                frames += 1;
            });
            for f in start_frame..frames {
                assert!(ring.read_frame_latest(&mut read_back));
                if f * 480 > 48_000 * 6 {
                    // After 6 s of convergence.
                    let s = f * 480;
                    in_energy += mic[s..s + 480].iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
                    out_energy += read_back.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
                }
            }
            pos += 512;
        }
        let reduction_db = 10.0 * (in_energy / out_energy.max(1e-12)).log10();
        assert!(reduction_db > 15.0, "echo reduced by only {reduction_db:.1} dB");
    }

    #[test]
    fn bypass_passes_the_mic_through_untouched() {
        let chain = Arc::new(VoiceChain::new(SuppressionMode::Max, true, VoiceIsolation::Strong));
        let mut path = MicPath::new(chain, Arc::new(AtomicBool::new(true)));
        let input: Vec<f32> = (0..960).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
        let mut got = Vec::new();
        path.push(&input, 1, |f| got.extend_from_slice(f));
        assert_eq!(got, input);
    }
}
