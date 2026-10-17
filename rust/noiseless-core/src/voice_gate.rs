//! Background-voice suppression.
//!
//! RNNoise removes *noise* but deliberately keeps *all* speech, including a
//! colleague talking across the room. This gate keeps the dominant (closest)
//! talker: it learns the level of your own speech and turns down speech that
//! is well below it. A near talker is typically 15–25 dB louder at the mic
//! than someone a few metres away.
//!
//! It runs after the noise suppressor, frame by frame, and adds no latency.

use crate::frame::rms;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum VoiceIsolation {
    Off = 0,
    Light = 1,
    Balanced = 2,
    Strong = 3,
}

impl VoiceIsolation {
    pub fn from_u32(value: u32) -> Self {
        match value {
            0 => VoiceIsolation::Off,
            1 => VoiceIsolation::Light,
            3 => VoiceIsolation::Strong,
            _ => VoiceIsolation::Balanced,
        }
    }

    /// (dB below your speech level that counts as background, gain applied to it)
    fn params(self) -> Option<(f32, f32)> {
        match self {
            VoiceIsolation::Off => None,
            VoiceIsolation::Light => Some((14.0, 0.18)),    // -15 dB
            VoiceIsolation::Balanced => Some((10.0, 0.05)), // -26 dB
            VoiceIsolation::Strong => Some((7.0, 0.02)),    // -34 dB
        }
    }
}

const VOICE_PROBABILITY: f32 = 0.6;
const INITIAL_LEVEL_DB: f32 = -60.0; // "not learnt yet": nothing is gated
const MIN_FOREGROUND_DB: f32 = -50.0; // anything louder can be your voice
const HOLD_FRAMES: u32 = 25; // 250 ms: keep word tails and short pauses

pub struct VoiceGate {
    isolation: VoiceIsolation,
    /// Learnt level of the dominant (your) voice, dBFS.
    foreground_db: f32,
    gain: f32,
    hold: u32,
}

impl VoiceGate {
    pub fn new(isolation: VoiceIsolation) -> Self {
        Self { isolation, foreground_db: INITIAL_LEVEL_DB, gain: 1.0, hold: 0 }
    }

    pub fn set_isolation(&mut self, isolation: VoiceIsolation) {
        self.isolation = isolation;
    }

    pub fn isolation(&self) -> VoiceIsolation {
        self.isolation
    }

    pub fn reset(&mut self) {
        self.foreground_db = INITIAL_LEVEL_DB;
        self.gain = 1.0;
        self.hold = 0;
    }

    /// Current gain applied (1 = open).
    pub fn gain(&self) -> f32 {
        self.gain
    }

    pub fn foreground_level_db(&self) -> f32 {
        self.foreground_db
    }

    /// Applies the gate in place. `vad` is the voice probability of this frame.
    pub fn process(&mut self, frame: &mut [f32], vad: f32) {
        let Some((threshold_db, background_gain)) = self.isolation.params() else {
            self.gain = 1.0;
            return;
        };
        let level_db = 20.0 * (rms(frame) + 1e-9).log10();
        let speech = vad >= VOICE_PROBABILITY;

        if speech {
            // Learn your level from strong speech: rise quickly, fall slowly and
            // only while the speech is still close to it, so a background talker
            // can never drag the reference down to its own level.
            if level_db > self.foreground_db {
                self.foreground_db += 0.3 * (level_db - self.foreground_db);
            } else if level_db > self.foreground_db - 6.0 {
                self.foreground_db += 0.01 * (level_db - self.foreground_db);
            }
        }

        // Anything well below your level is ducked — whether or not the noise
        // model is confident it is speech (distant, reverberant voices often
        // score low and would otherwise pass untouched).
        let learnt = self.foreground_db > MIN_FOREGROUND_DB;
        let foreground = !learnt || level_db >= self.foreground_db - threshold_db;
        let mut target = 1.0;
        if foreground && speech {
            self.hold = HOLD_FRAMES;
        } else if !foreground && self.hold == 0 {
            target = background_gain;
        }
        if self.hold > 0 {
            self.hold -= 1;
            target = 1.0;
        }

        // Fast open (don't clip your first syllable), gentle close.
        let rate = if target > self.gain { 0.8 } else { 0.15 };
        let start = self.gain;
        self.gain += rate * (target - self.gain);
        // Ramp across the frame so gain changes never click.
        let n = frame.len().max(1) as f32;
        for (i, s) in frame.iter_mut().enumerate() {
            let g = start + (self.gain - start) * (i as f32 + 1.0) / n;
            *s *= g;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(level: f32) -> Vec<f32> {
        (0..480).map(|i| level * (i as f32 * 0.07).sin() * std::f32::consts::SQRT_2).collect()
    }

    fn gain_after(gate: &mut VoiceGate, level: f32, frames: usize) -> f32 {
        let mut last = 0.0;
        for _ in 0..frames {
            let mut f = frame(level);
            let before = rms(&f);
            gate.process(&mut f, 0.95);
            last = rms(&f) / before;
        }
        last
    }

    #[test]
    fn keeps_your_voice_and_ducks_quiet_background_voice() {
        let mut gate = VoiceGate::new(VoiceIsolation::Balanced);
        // You talk at ~-20 dBFS.
        assert!(gain_after(&mut gate, 0.1, 100) > 0.95);
        // Someone ~20 dB quieter talks while you are silent.
        let g = gain_after(&mut gate, 0.01, 60);
        assert!(g < 0.15, "background voice gain {g}");
        // You start again: the gate opens immediately.
        assert!(gain_after(&mut gate, 0.1, 3) > 0.9);
    }

    #[test]
    fn background_alone_never_becomes_the_reference() {
        let mut gate = VoiceGate::new(VoiceIsolation::Balanced);
        gain_after(&mut gate, 0.1, 100);
        gain_after(&mut gate, 0.01, 3000); // 30 s of only background speech
        assert!(gate.foreground_level_db() > -25.0, "learnt level drifted to {}", gate.foreground_level_db());
    }

    #[test]
    fn nothing_is_gated_before_your_voice_is_learnt_or_when_off() {
        let mut gate = VoiceGate::new(VoiceIsolation::Strong);
        assert!(gain_after(&mut gate, 0.01, 50) > 0.95, "first speech heard must pass");
        let mut off = VoiceGate::new(VoiceIsolation::Off);
        gain_after(&mut off, 0.1, 50);
        assert!(gain_after(&mut off, 0.005, 50) > 0.99);
    }
}
