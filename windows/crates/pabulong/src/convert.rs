//! Turns WASAPI buffers into 48 kHz mono float.
//!
//! Normally WASAPI converts for us (`AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM`
//! delivers 48 kHz mono float directly). This is the fallback for drivers
//! that refuse that, mirroring the AVAudioConverter step on macOS: decode the
//! sample format, average the channels, and resample.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    F32,
    I16,
    /// 24-bit samples in 3 bytes.
    I24,
    /// 24-bit samples left-justified in 4 bytes, or true 32-bit.
    I32,
}

impl SampleFormat {
    pub fn bytes(self) -> usize {
        match self {
            SampleFormat::F32 | SampleFormat::I32 => 4,
            SampleFormat::I16 => 2,
            SampleFormat::I24 => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub format: SampleFormat,
}

/// Average of all channels, as float in -1..1.
pub fn downmix(bytes: &[u8], format: InputFormat, out: &mut Vec<f32>) {
    let channels = usize::from(format.channels.max(1));
    let width = format.format.bytes();
    let frames = bytes.len() / (width * channels);
    out.clear();
    out.reserve(frames);
    let scale = 1.0 / channels as f32;
    for f in 0..frames {
        let mut sum = 0.0f32;
        for c in 0..channels {
            let at = (f * channels + c) * width;
            let b = &bytes[at..at + width];
            sum += match format.format {
                SampleFormat::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                SampleFormat::I16 => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0,
                SampleFormat::I24 => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
                SampleFormat::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            };
        }
        out.push(sum * scale);
    }
}

/// Streaming resampler to 48 kHz. Cubic (Catmull-Rom) interpolation after a
/// gentle low-pass when downsampling; enough for speech going into the core,
/// and only used when WASAPI cannot convert.
pub struct Resampler {
    ratio: f64, // input samples per output sample
    position: f64,
    history: [f32; 4],
    lowpass: Option<Biquad>,
}

impl Resampler {
    pub fn new(input_rate: u32) -> Self {
        let ratio = f64::from(input_rate) / 48_000.0;
        let lowpass = (input_rate > 48_000).then(|| Biquad::lowpass(f64::from(input_rate), 20_000.0));
        Self { ratio, position: 0.0, history: [0.0; 4], lowpass }
    }

    pub fn is_passthrough(&self) -> bool {
        self.ratio == 1.0
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.is_passthrough() {
            out.extend_from_slice(input);
            return;
        }
        for &raw in input {
            let s = match self.lowpass.as_mut() {
                Some(lp) => lp.process(raw),
                None => raw,
            };
            self.history = [self.history[1], self.history[2], self.history[3], s];
            // Emit every output sample that falls between history[1] and history[2].
            while self.position < 1.0 {
                let t = self.position as f32;
                let [p0, p1, p2, p3] = self.history;
                let v = p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)));
                out.push(v);
                self.position += self.ratio;
            }
            self.position -= 1.0;
        }
    }
}

struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    fn lowpass(rate: f64, cutoff: f64) -> Self {
        let w = 2.0 * std::f64::consts::PI * cutoff / rate;
        let alpha = w.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let cos = w.cos();
        let a0 = 1.0 + alpha;
        let b1 = (1.0 - cos) / a0;
        Biquad {
            b: [(b1 / 2.0) as f32, b1 as f32, (b1 / 2.0) as f32],
            a: [(-2.0 * cos / a0) as f32, ((1.0 - alpha) / a0) as f32],
            z: [0.0; 2],
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmixes_every_format() {
        let mut out = Vec::new();
        let stereo_f32: Vec<u8> = [0.5f32, -0.5, 1.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
        downmix(&stereo_f32, InputFormat { sample_rate: 48_000, channels: 2, format: SampleFormat::F32 }, &mut out);
        assert_eq!(out, vec![0.0, 0.5]);
        let mono_i16: Vec<u8> = [16_384i16, -32_768].iter().flat_map(|v| v.to_le_bytes()).collect();
        downmix(&mono_i16, InputFormat { sample_rate: 48_000, channels: 1, format: SampleFormat::I16 }, &mut out);
        assert_eq!(out, vec![0.5, -1.0]);
        let mono_i24 = [0x00, 0x00, 0x40, 0x00, 0x00, 0xC0]; // +0.5, -0.5
        downmix(&mono_i24, InputFormat { sample_rate: 48_000, channels: 1, format: SampleFormat::I24 }, &mut out);
        assert_eq!(out, vec![0.5, -0.5]);
    }

    fn tone(rate: u32, hz: f64, n: usize) -> Vec<f32> {
        (0..n).map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(rate)).sin() as f32).collect()
    }

    #[test]
    fn resamples_to_48k_with_the_right_length_and_pitch() {
        for rate in [16_000u32, 44_100, 96_000] {
            let input = tone(rate, 1_000.0, rate as usize); // 1 s
            let mut r = Resampler::new(rate);
            let mut out = Vec::new();
            for chunk in input.chunks(333) {
                r.process(chunk, &mut out);
            }
            assert!((out.len() as i64 - 48_000).abs() <= 2, "{rate}: {} samples", out.len());
            // Compare with an ideal 1 kHz tone at 48 kHz (allowing the filter delay).
            let ideal = tone(48_000, 1_000.0, out.len());
            let best = (0..48)
                .map(|lag| {
                    let err: f64 = out[1000..40_000].iter().zip(&ideal[1000 - lag..40_000 - lag]).map(|(a, b)| f64::from(a - b).powi(2)).sum();
                    (err / 39_000.0).sqrt()
                })
                .fold(f64::MAX, f64::min);
            assert!(best < 0.05, "{rate}: rms error {best}");
        }
    }
}
