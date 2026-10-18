//! Cuts the mic's device-sized buffers into the core's 480-sample frames and
//! keeps the capture time of each frame's first sample — the same bookkeeping
//! as `AudioPipeline.consume` on macOS.

use crate::chain::FRAME_LENGTH;

const NANOS_PER_SAMPLE: f64 = 1e9 / 48_000.0;
/// Safety valve only (250 ms); steady state holds less than one frame.
const MAX_BACKLOG: usize = FRAME_LENGTH * 25;

pub struct Framer {
    pending: Vec<f32>,
    /// Host time (ns) of `pending[0]`.
    start_ns: f64,
}

impl Default for Framer {
    fn default() -> Self {
        Self { pending: Vec::with_capacity(MAX_BACKLOG + 4096), start_ns: 0.0 }
    }
}

impl Framer {
    /// Adds 48 kHz mono samples whose first sample was captured at
    /// `capture_ns`, then calls `frame` for every complete frame.
    pub fn push(&mut self, samples: &[f32], capture_ns: u64, mut frame: impl FnMut(&[f32], u64)) {
        // Anchor the accumulator to this chunk's capture time.
        self.start_ns = capture_ns as f64 - self.pending.len() as f64 * NANOS_PER_SAMPLE;
        self.pending.extend_from_slice(samples);
        if self.pending.len() > MAX_BACKLOG {
            let dropped = self.pending.len() - MAX_BACKLOG;
            self.pending.drain(..dropped);
            self.start_ns += dropped as f64 * NANOS_PER_SAMPLE;
        }
        let mut offset = 0;
        while self.pending.len() - offset >= FRAME_LENGTH {
            frame(&self.pending[offset..offset + FRAME_LENGTH], self.start_ns.max(0.0) as u64);
            offset += FRAME_LENGTH;
            self.start_ns += FRAME_LENGTH as f64 * NANOS_PER_SAMPLE;
        }
        self.pending.drain(..offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_contiguous_and_timestamped() {
        let mut f = Framer::default();
        let mut frames = Vec::new();
        let mut next = 0.0f32;
        let mut t = 1_000_000_000u64;
        for chunk in [441usize, 512, 100, 960, 7] {
            let s: Vec<f32> = (0..chunk).map(|i| next + i as f32).collect();
            next += chunk as f32;
            f.push(&s, t, |frame, ns| frames.push((frame[0], frame[479], ns)));
            t += (chunk as f64 * NANOS_PER_SAMPLE) as u64;
        }
        assert_eq!(frames.len(), (441 + 512 + 100 + 960 + 7) / 480);
        for (i, (first, last, ns)) in frames.iter().enumerate() {
            assert_eq!(*first, (i * 480) as f32);
            assert_eq!(*last, (i * 480 + 479) as f32);
            let expected = 1_000_000_000.0 + (i * 480) as f64 * NANOS_PER_SAMPLE;
            assert!((*ns as f64 - expected).abs() < 2_000.0, "frame {i}: {ns} vs {expected}");
        }
    }
}
