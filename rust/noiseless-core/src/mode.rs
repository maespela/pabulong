//! Suppression strength presets. Values mirror the macOS implementation.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum NoiseMode {
    Light = 0,
    Balanced = 1,
    Max = 2,
}

impl NoiseMode {
    pub fn from_u32(value: u32) -> Self {
        match value {
            0 => NoiseMode::Light,
            2 => NoiseMode::Max,
            _ => NoiseMode::Balanced,
        }
    }

    /// Wet/dry mix for the neural backend (RNNoise is a single fixed model).
    #[cfg_attr(not(feature = "rnnoise"), allow(dead_code))]
    pub(crate) fn wet_mix(self) -> f32 {
        match self {
            NoiseMode::Light => 0.75,
            NoiseMode::Balanced | NoiseMode::Max => 1.0,
        }
    }

    #[cfg_attr(not(feature = "rnnoise"), allow(dead_code))]
    pub(crate) fn vad_open_threshold(self) -> f32 {
        match self {
            NoiseMode::Light => 0.25,
            NoiseMode::Balanced => 0.32,
            NoiseMode::Max => 0.38,
        }
    }

    #[cfg_attr(not(feature = "rnnoise"), allow(dead_code))]
    pub(crate) fn silence_gain(self) -> f32 {
        match self {
            NoiseMode::Light => 0.55,
            NoiseMode::Balanced => 0.22,
            NoiseMode::Max => 0.08,
        }
    }

    /// Minimum per-bin gain of the spectral suppressor (lower = more aggressive).
    pub(crate) fn spectral_floor(self) -> f32 {
        match self {
            NoiseMode::Light => 0.30,
            NoiseMode::Balanced => 0.14,
            NoiseMode::Max => 0.05,
        }
    }

    /// How much of the estimated noise magnitude the spectral suppressor removes.
    pub(crate) fn oversubtraction(self) -> f32 {
        match self {
            NoiseMode::Light => 1.0,
            NoiseMode::Balanced => 1.6,
            NoiseMode::Max => 2.4,
        }
    }
}
