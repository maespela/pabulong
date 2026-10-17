//! Speaker-output reference for the echo canceller, aligned to the mic by time.
//!
//! The reference (system output) and the mic arrive on different threads with
//! different buffering. Both carry the host time of their first sample
//! (mach_absolute_time ns on macOS, QPC on Windows).
//!
//! The canceller must never be fed reference audio that has not arrived yet:
//! that audio would be treated as silence, and when it does arrive the echo it
//! causes can no longer be modelled. So the reference is read slightly in the
//! past — `lookback` before the mic frame — where `lookback` is learnt from how
//! late the reference actually arrives. The echo then appears to the canceller
//! with a delay of (echo path − lookback), which is positive whenever the tap
//! delivers audio sooner than it travels out of the speaker and back into the
//! mic (true for built-in and Bluetooth speakers).

use std::collections::VecDeque;
use std::sync::Mutex;

const MAX_BUFFERED: usize = 48_000; // 1 s
const MARGIN_NS: f64 = 1_500_000.0; // 1.5 ms on top of the observed lateness
const MAX_LOOKBACK_NS: f64 = 200_000_000.0; // 200 ms
const WINDOW_READS: usize = 300; // ~3 s of 10 ms frames
/// Beyond this deviation between the contiguous read position and the
/// timestamp-derived one, jump (resync) instead of slipping single samples.
const RESYNC_SAMPLES: i64 = 96; // 2 ms
/// Deviation at which one sample is slipped to follow clock drift.
const SLIP_SAMPLES: i64 = 6;

struct Inner {
    samples: VecDeque<f32>,
    /// Absolute stream index of `samples[0]`.
    first_index: u64,
    /// Latest timing anchor: (stream index, host time ns) of a pushed chunk.
    anchor: Option<(u64, u64)>,
    sample_rate: f64,
    lookback_ns: f64,
    /// Recent lateness observations (ns) for the lookback estimate.
    lateness: VecDeque<f64>,
    realignments: u32,
    /// Next stream index to read. Reads are contiguous so timestamp jitter
    /// never shifts the reference by a sample (to the canceller that looks
    /// like the echo path changing).
    next_read: Option<i64>,
    /// Low-passed (timestamp - contiguous) deviation in samples: averages out
    /// timestamp jitter so only genuine clock drift causes a slip.
    drift: f64,
}

impl Inner {
    fn newest_time_ns(&self) -> Option<f64> {
        let (anchor_index, anchor_time) = self.anchor?;
        let end_index = self.first_index + self.samples.len() as u64;
        Some(anchor_time as f64 + (end_index as f64 - anchor_index as f64) * 1e9 / self.sample_rate)
    }
}

pub struct ReferenceQueue {
    inner: Mutex<Inner>,
}

impl ReferenceQueue {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            inner: Mutex::new(Inner {
                samples: VecDeque::with_capacity(MAX_BUFFERED),
                first_index: 0,
                anchor: None,
                sample_rate: f64::from(sample_rate),
                lookback_ns: 0.0,
                lateness: VecDeque::with_capacity(WINDOW_READS),
                realignments: 0,
                next_read: None,
                drift: 0.0,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Appends reference audio (mono, the processing sample rate) whose first
    /// sample was played at `host_time_ns`. Safe to call from any thread.
    pub fn push(&self, samples: &[f32], host_time_ns: u64) {
        let mut inner = self.lock();
        let index = inner.first_index + inner.samples.len() as u64;
        inner.anchor = Some((index, host_time_ns));
        inner.samples.extend(samples.iter().copied());
        let excess = inner.samples.len().saturating_sub(MAX_BUFFERED);
        if excess > 0 {
            inner.samples.drain(..excess);
            inner.first_index += excess as u64;
        }
    }

    /// Fills `out` with the reference for the mic frame captured at
    /// `host_time_ns` (read `lookback` earlier). Missing audio is silence.
    /// Returns true if any reference audio was present.
    pub fn read_at(&self, out: &mut [f32], host_time_ns: u64) -> bool {
        out.fill(0.0);
        let mut inner = self.lock();
        let Some(newest) = inner.newest_time_ns() else { return false };
        let (anchor_index, anchor_time) = inner.anchor.expect("checked");
        let frame_ns = out.len() as f64 * 1e9 / inner.sample_rate;

        // How far beyond the newest arrived sample this frame reaches.
        let late = (host_time_ns as f64 + frame_ns - newest).max(0.0);
        if inner.lateness.len() == WINDOW_READS {
            inner.lateness.pop_front();
        }
        inner.lateness.push_back(late);
        let worst = inner.lateness.iter().copied().fold(0.0, f64::max);
        let target = (worst + MARGIN_NS).min(MAX_LOOKBACK_NS);
        // Grow immediately (losing reference is the worst failure); shrink only
        // after a full window of consistently lower lateness. Each change
        // shifts the apparent echo delay, which the canceller re-learns.
        let full = inner.lateness.len() == WINDOW_READS;
        let mut lookback_changed = false;
        if late > inner.lookback_ns || (full && target < inner.lookback_ns - 5e6) {
            // Grow with extra headroom so this happens rarely.
            inner.lookback_ns = if late > inner.lookback_ns { (target + 2e6).min(MAX_LOOKBACK_NS) } else { target };
            lookback_changed = true;
        }

        let read_time = host_time_ns as f64 - inner.lookback_ns;
        let offset_samples = (read_time - anchor_time as f64) * inner.sample_rate / 1e9;
        let desired = anchor_index as i64 + offset_samples.round() as i64;
        let start = match inner.next_read {
            Some(next) if !lookback_changed && (desired - next).abs() <= RESYNC_SAMPLES => {
                // Follow clock drift one sample at a time.
                inner.drift = 0.95 * inner.drift + 0.05 * (desired - next) as f64;
                if inner.drift >= SLIP_SAMPLES as f64 {
                    inner.drift -= 1.0;
                    next + 1
                } else if inner.drift <= -(SLIP_SAMPLES as f64) {
                    inner.drift += 1.0;
                    next - 1
                } else {
                    next
                }
            }
            _ => {
                inner.realignments += 1;
                inner.drift = 0.0;
                desired
            }
        };
        inner.next_read = Some(start + out.len() as i64);

        let first = inner.first_index as i64;
        let end = first + inner.samples.len() as i64;
        let _ = target;
        let mut any = false;
        for (i, o) in out.iter_mut().enumerate() {
            let idx = start + i as i64;
            if idx >= first && idx < end {
                *o = inner.samples[(idx - first) as usize];
                any = true;
            }
        }
        // Keep a little history behind the read position (a later, larger
        // lookback may need it); drop the rest.
        let keep_from = start - (inner.sample_rate * 0.25) as i64;
        let drop = (keep_from - first).clamp(0, inner.samples.len() as i64) as usize;
        if drop > 0 {
            inner.samples.drain(..drop);
            inner.first_index += drop as u64;
        }
        any
    }

    /// Current lookback (ms): how far behind the mic the reference is read.
    pub fn lookback_ms(&self) -> f32 {
        (self.lock().lookback_ns / 1e6) as f32
    }

    pub fn realignments(&self) -> u32 {
        self.lock().realignments
    }

    pub fn clear(&self) {
        let mut inner = self.lock();
        let len = inner.samples.len() as u64;
        inner.first_index += len;
        inner.samples.clear();
        inner.anchor = None;
        inner.lateness.clear();
        inner.lookback_ns = 0.0;
        inner.next_read = None;
        inner.drift = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(sample: usize) -> u64 {
        (sample as f64 * 1e9 / 48_000.0).round() as u64
    }

    #[test]
    fn reads_by_time_regardless_of_chunking() {
        let q = ReferenceQueue::new(48_000);
        let all: Vec<f32> = (0..2000).map(|i| i as f32).collect();
        let mut pos = 0;
        for chunk in [300usize, 512, 188, 1000] {
            q.push(&all[pos..pos + chunk], t(pos));
            pos += chunk;
        }
        let mut out = [0.0; 10];
        // Everything up to 2000 has arrived, so only the safety margin applies.
        assert!(q.read_at(&mut out, t(400)));
        let lookback = (q.lookback_ms() * 48.0).round();
        assert!(lookback <= 96.0, "lookback {lookback} samples");
        assert_eq!(out, std::array::from_fn(|i| 400.0 - lookback + i as f32));
    }

    #[test]
    fn learns_lookback_so_late_reference_is_never_lost() {
        // Reference arrives in 512-sample chunks, 10 ms late; mic frames of
        // 480 are read as soon as they are captured.
        let q = ReferenceQueue::new(48_000);
        let all: Vec<f32> = (0..96_000).map(|i| (i % 1000) as f32 + 1.0).collect();
        let mut pushed = 0;
        let mut zeros_after_warmup = 0;
        for f in 0..190 {
            let now = (f + 1) * 480; // processing time, in samples
            while pushed + 512 + 480 <= now {
                q.push(&all[pushed..pushed + 512], t(pushed));
                pushed += 512;
            }
            let mut out = [0.0; 480];
            q.read_at(&mut out, t(f * 480));
            if f > 20 {
                zeros_after_warmup += out.iter().filter(|s| **s == 0.0).count();
            }
        }
        assert_eq!(zeros_after_warmup, 0, "reference was dropped after warm-up");
        let lb = q.lookback_ms();
        assert!(lb > 20.0 && lb < 35.0, "lookback {lb} ms");
    }

    #[test]
    fn jittery_timestamps_still_give_a_contiguous_stream() {
        let q = ReferenceQueue::new(48_000);
        let all: Vec<f32> = (0..200_000).map(|i| i as f32).collect();
        let mut pushed = 0;
        let mut seed = 1u32;
        let mut jitter = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            ((seed >> 8) as f64 / 16_777_216.0 - 0.5) * 120_000.0 // +-60 us
        };
        let mut previous_last: Option<f32> = None;
        let mut discontinuities = 0;
        for f in 0..400 {
            while pushed + 512 <= (f + 2) * 480 {
                q.push(&all[pushed..pushed + 512], (t(pushed) as f64 + jitter()) as u64);
                pushed += 512;
            }
            let mut out = [0.0; 480];
            q.read_at(&mut out, (t(f * 480) as f64 + jitter()) as u64);
            if out[0] == 0.0 {
                continue;
            }
            for w in out.windows(2) {
                if w[1] - w[0] != 1.0 {
                    discontinuities += 1;
                }
            }
            if let Some(p) = previous_last {
                if out[0] - p != 1.0 {
                    discontinuities += 1;
                }
            }
            previous_last = Some(out[479]);
        }
        assert!(discontinuities <= 2, "{discontinuities} discontinuities with 60 us jitter");
    }

    #[test]
    fn follows_clock_drift_without_resyncing() {
        // Reference clock 100 ppm fast relative to the mic: the read position
        // must follow it with single-sample slips, not jumps.
        let q = ReferenceQueue::new(48_000);
        let all: Vec<f32> = (0..3_000_000).map(|i| i as f32).collect();
        let ref_ns = |n: usize| (n as f64 * 1e9 / (48_000.0 * 1.0001)) as u64;
        let mut pushed = 0;
        for f in 0..5_000 {
            while pushed + 512 <= ((f + 2) as f64 * 480.0 * 1.0001) as usize {
                q.push(&all[pushed..pushed + 512], ref_ns(pushed));
                pushed += 512;
            }
            let mut out = [0.0; 480];
            q.read_at(&mut out, t(f * 480));
        }
        assert!(q.realignments() <= 2, "{} resyncs over 50 s of drift", q.realignments());
    }

    #[test]
    fn missing_reference_is_silence() {
        let q = ReferenceQueue::new(48_000);
        let mut out = [1.0; 8];
        assert!(!q.read_at(&mut out, 123));
        assert_eq!(out, [0.0; 8]);
    }
}
