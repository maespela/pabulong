//! Offline acoustic simulator for the Noiseless mic chain.
//!
//! Builds realistic call scenes from synthesized speech with known ground
//! truth, runs them through (a) the chain the Mac app shipped with (noise
//! suppressor only) and (b) the shared pipeline (echo canceller + noise
//! suppressor + background-voice gate), prints measurements, and writes WAVs
//! so the result can be listened to.
//!
//! usage: noiseless-sim <voices dir> <out dir>

use noiseless_core::fft::fft;
use noiseless_core::{NoiseMode, NoiseProcessor, ReferenceQueue, VoiceIsolation, VoicePipeline, FRAME_LENGTH};
use std::path::{Path, PathBuf};
use std::time::Instant;

const FS: usize = 48_000;

// ---------------------------------------------------------------- WAV I/O

fn read_wav(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let u16le = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let u32le = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    assert_eq!(&bytes[0..4], b"RIFF");
    let (mut format, mut channels, mut bits) = (0, 0, 0);
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32le(pos + 4) as usize;
        let body = pos + 8;
        if id == b"fmt " {
            format = u16le(body);
            channels = u16le(body + 2) as usize;
            assert_eq!(u32le(body + 4) as usize, FS, "voices must be 48 kHz");
            bits = u16le(body + 14);
        } else if id == b"data" {
            let data = &bytes[body..(body + size).min(bytes.len())];
            let mono: Vec<f32> = match (format, bits) {
                (3, 32) | (0xFFFE, 32) => data.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
                (1, 16) => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect(),
                other => panic!("unsupported WAV format {other:?}"),
            };
            return mono.chunks(channels.max(1)).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
        }
        pos = body + size + (size & 1);
    }
    panic!("{}: no data chunk", path.display());
}

fn write_wav(path: &Path, samples: &[f32]) {
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&(FS as u32).to_le_bytes());
    out.extend_from_slice(&((FS * 2) as u32).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

// ------------------------------------------------------------- signal tools

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.0 >> 8) as f32 / 16_777_216.0 - 0.5
    }
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt() as f32
}

fn db(x: f32) -> f32 {
    20.0 * (x + 1e-9).log10()
}

/// Scales `x` so its active-speech RMS (frames above -50 dB of peak) is `target_db`.
fn normalize(x: &[f32], target_db: f32) -> Vec<f32> {
    let peak = x.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
    let active: Vec<f32> = x.chunks(480).filter(|c| rms(c) > peak * 0.003).flatten().copied().collect();
    let gain = 10f32.powf(target_db / 20.0) / rms(&active).max(1e-9);
    x.iter().map(|s| s * gain).collect()
}

/// Linear convolution via one large FFT (inputs are tens of seconds long).
fn convolve(x: &[f32], h: &[f32]) -> Vec<f32> {
    let n = (x.len() + h.len()).next_power_of_two();
    let (mut ar, mut ai) = (vec![0.0; n], vec![0.0; n]);
    let (mut br, mut bi) = (vec![0.0; n], vec![0.0; n]);
    ar[..x.len()].copy_from_slice(x);
    br[..h.len()].copy_from_slice(h);
    fft(&mut ar, &mut ai, false);
    fft(&mut br, &mut bi, false);
    for k in 0..n {
        let (r, i) = (ar[k] * br[k] - ai[k] * bi[k], ar[k] * bi[k] + ai[k] * br[k]);
        ar[k] = r;
        ai[k] = i;
    }
    fft(&mut ar, &mut ai, true);
    ar.truncate(x.len());
    ar
}

/// Room impulse response: direct path + exponentially decaying diffuse tail
/// with a given RT60 and direct-to-reverberant ratio, unit energy.
fn room_ir(rt60: f32, drr_db: f32, seed: u32) -> Vec<f32> {
    let len = (rt60 * FS as f32) as usize;
    let mut rng = Lcg(seed);
    let decay = 6.91 / (rt60 * FS as f32);
    let mut h: Vec<f32> = (0..len).map(|n| rng.next() * (-decay * n as f32).exp()).collect();
    // Gentle low-pass on the tail (air/wall absorption).
    for n in 1..len {
        h[n] = 0.6 * h[n] + 0.4 * h[n - 1];
    }
    h[0] = 0.0;
    let tail = h.iter().map(|s| s * s).sum::<f32>().sqrt();
    let direct = 10f32.powf(drr_db / 20.0) * tail;
    h[0] = direct;
    let norm = h.iter().map(|s| s * s).sum::<f32>().sqrt();
    h.iter().map(|s| s / norm).collect()
}

/// Places `clip[from_s..to_s]` into a timeline at `at_s`.
fn place(track: &mut [f32], clip: &[f32], at_s: f32, from_s: f32, to_s: f32) {
    let (a, f, t) = ((at_s * FS as f32) as usize, (from_s * FS as f32) as usize, ((to_s * FS as f32) as usize).min(clip.len()));
    for (i, s) in clip[f..t].iter().enumerate() {
        if let Some(slot) = track.get_mut(a + i) {
            *slot += s;
        }
    }
}

fn add_noise(track: &mut [f32], level_db: f32, seed: u32) {
    let mut rng = Lcg(seed);
    let amp = 10f32.powf(level_db / 20.0) * 3.46; // uniform -> rms
    let mut lp = 0.0;
    for s in track.iter_mut() {
        lp = 0.9 * lp + 0.1 * rng.next(); // brownish office noise
        *s += amp * 2.2 * lp;
    }
}

fn window(x: &[f32], from_s: f32, to_s: f32) -> &[f32] {
    let (a, b) = ((from_s * FS as f32) as usize, ((to_s * FS as f32) as usize).min(x.len()));
    &x[a..b]
}

/// Best normalised cross-correlation of `out` against `truth` for lags 0..max.
fn correlation(out: &[f32], truth: &[f32], max_lag: usize) -> f32 {
    let mut best = 0.0_f32;
    for lag in (0..max_lag).step_by(16) {
        let n = out.len().min(truth.len() + lag) - lag;
        let (a, b) = (&out[lag..lag + n], &truth[..n]);
        let num: f64 = a.iter().zip(b).map(|(x, y)| (*x as f64) * (*y as f64)).sum();
        let den = (a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() * b.iter().map(|y| (*y as f64).powi(2)).sum::<f64>()).sqrt();
        if den > 0.0 {
            best = best.max((num / den) as f32);
        }
    }
    best
}

// ------------------------------------------------------------------ chains

/// The chain the Mac app shipped with: noise suppressor only.
fn run_current(mic: &[f32]) -> Vec<f32> {
    let mut p = NoiseProcessor::standard(NoiseMode::Balanced);
    let mut out = vec![0.0; mic.len()];
    for f in 0..mic.len() / FRAME_LENGTH {
        let r = f * FRAME_LENGTH..(f + 1) * FRAME_LENGTH;
        p.process(&mic[r.clone()], &mut out[r]).unwrap();
    }
    out
}

/// Timing of the reference relative to the mic, as on a real Mac.
struct Timing {
    /// Delay from tap to the echo reaching the mic: output latency + acoustics.
    echo_path_s: f32,
    /// How late tapped audio becomes available to the app.
    tap_delivery_s: f32,
    /// How late a captured mic frame is processed.
    mic_processing_s: f32,
    /// Size of the chunks the tap delivers.
    tap_chunk: usize,
}

/// The shared pipeline, fed with a time-stamped reference delivered in tap
/// chunks, exactly like the app does it.
fn run_pipeline(mic: &[f32], speaker: Option<&[f32]>, isolation: VoiceIsolation, timing: &Timing) -> (Vec<f32>, f32, f32, f32) {
    let mut p = VoicePipeline::new(NoiseMode::Balanced, speaker.is_some(), isolation);
    let queue = ReferenceQueue::new(FS as u32);
    let ns = |samples: usize| (samples as f64 * 1e9 / FS as f64) as u64;
    let mut out = vec![0.0; mic.len()];
    let mut next_chunk = 0usize;
    let started = Instant::now();
    for f in 0..mic.len() / FRAME_LENGTH {
        let start = f * FRAME_LENGTH;
        if let Some(spk) = speaker {
            // Deliver every tap chunk that has arrived by the time this mic
            // frame is processed.
            let now = (start + FRAME_LENGTH) as f32 / FS as f32 + timing.mic_processing_s;
            while next_chunk < spk.len() && (next_chunk + timing.tap_chunk) as f32 / FS as f32 + timing.tap_delivery_s <= now {
                let end = (next_chunk + timing.tap_chunk).min(spk.len());
                queue.push(&spk[next_chunk..end], ns(next_chunk));
                next_chunk = end;
            }
        }
        let r = start..start + FRAME_LENGTH;
        p.process(&mic[r.clone()], &mut out[r], speaker.map(|_| &queue), ns(start)).unwrap();
    }
    let secs = mic.len() as f32 / FS as f32;
    let speed = secs / started.elapsed().as_secs_f32();
    let m = p.metrics();
    if speaker.is_some() {
        println!("  reference lookback {:.1} ms ({} realignments)", queue.lookback_ms(), queue.realignments());
    }
    (out, speed, m.echo.erle_db, m.echo.delay_ms)
}

// --------------------------------------------------------------- scenarios

fn echo_scenario(voices: &Path, out_dir: &Path, timing: &Timing, label: &str, write: bool) {
    let you = normalize(&read_wav(&voices.join("you.wav")), -24.0);
    let caller = normalize(&read_wav(&voices.join("caller.wav")), -20.0);
    let total = 21 * FS;

    // What the call app plays out of the speakers (the tap reference).
    let mut speaker = vec![0.0; total];
    place(&mut speaker, &caller, 0.0, 0.0, 8.0);
    place(&mut speaker, &caller, 13.0, 8.0, 16.0);

    // Laptop speaker: mild soft clipping, then the speaker -> mic acoustic path.
    let drive: f32 = std::env::var("NL_SIM_DRIVE").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0);
    let driven: Vec<f32> = speaker.iter().map(|s| if drive > 0.0 { (drive * s).tanh() / drive } else { *s }).collect();
    let mut echo = convolve(&driven, &room_ir(0.3, 4.0, 11));
    let delay = (timing.echo_path_s * FS as f32) as usize;
    echo.rotate_right(delay);
    echo[..delay].fill(0.0);
    let echo = {
        let active = window(&echo, 1.0, 7.0);
        let g = 10f32.powf(-24.0 / 20.0) / rms(active); // echo as loud as your voice
        echo.iter().map(|s| s * g).collect::<Vec<_>>()
    };

    let mut near = vec![0.0; total];
    place(&mut near, &you, 8.0, 0.0, 13.0);
    let near = convolve(&near, &room_ir(0.15, 12.0, 21)); // you, close to the mic

    let mut mic: Vec<f32> = near.iter().zip(&echo).map(|(a, b)| a + b).collect();
    add_noise(&mut mic, -55.0, 3);

    let current = run_current(&mic);
    let (new, speed, erle, found_delay) = run_pipeline(&mic, Some(&speaker), VoiceIsolation::Balanced, timing);

    println!("\n== Speaker echo [{label}] (echo path {:.0} ms, tap chunks {}) ==", timing.echo_path_s * 1000.0, timing.tap_chunk);
    println!("  segments: 0-8 s caller only | 8-13 s you only | 13-21 s both talking");
    let far = |x: &[f32]| db(rms(window(x, 2.0, 8.0)));
    println!("  caller-only, echo level at mic       : {:6.1} dBFS", far(&mic));
    println!("  caller-only, after current app        : {:6.1} dBFS  ({:+.1} dB)", far(&current), far(&current) - far(&mic));
    println!("  caller-only, after new pipeline       : {:6.1} dBFS  ({:+.1} dB)", far(&new), far(&new) - far(&mic));
    let conv = |x: &[f32]| db(rms(window(x, 0.0, 2.0)));
    println!("  first 2 s (still converging), new     : {:6.1} dBFS  ({:+.1} dB)", conv(&new), conv(&new) - conv(&mic));
    let nr = |x: &[f32]| db(rms(window(x, 8.5, 13.0)));
    println!("  you-only, level change current / new  : {:+.1} / {:+.1} dB", nr(&current) - nr(&mic), nr(&new) - nr(&mic));
    let dt = |x: &[f32], t: &[f32]| correlation(window(x, 14.0, 21.0), window(t, 14.0, 21.0), 3000);
    println!(
        "  double talk: corr with YOUR voice   current {:.2} | new {:.2}   (higher = you kept)",
        dt(&current, &near),
        dt(&new, &near)
    );
    println!(
        "  double talk: corr with CALLER echo  current {:.2} | new {:.2}   (lower = less echo)",
        dt(&current, &echo),
        dt(&new, &echo)
    );
    println!("  canceller: ERLE {erle:.1} dB, found echo delay {found_delay:.0} ms, {speed:.0}x realtime");
    if write {
        write_wav(&out_dir.join("echo_1_mic_input.wav"), &mic);
        write_wav(&out_dir.join("echo_2_current_app.wav"), &current);
        write_wav(&out_dir.join("echo_3_new_pipeline.wav"), &new);
    }
}

fn background_scenario(voices: &Path, out_dir: &Path, below_db: f32, write: bool) {
    let you = normalize(&read_wav(&voices.join("you.wav")), -24.0);
    let other = normalize(&read_wav(&voices.join("background.wav")), -24.0 - below_db);
    let total = 21 * FS;

    let mut near = vec![0.0; total];
    place(&mut near, &you, 0.0, 0.0, 5.0);
    place(&mut near, &you, 10.0, 5.0, 12.0);
    let near = convolve(&near, &room_ir(0.15, 12.0, 21));
    let mut far = vec![0.0; total];
    place(&mut far, &other, 3.0, 0.0, 17.5);
    let far = convolve(&far, &room_ir(0.5, -3.0, 31)); // across the room: reverberant

    let mut mic: Vec<f32> = near.iter().zip(&far).map(|(a, b)| a + b).collect();
    add_noise(&mut mic, -55.0, 4);

    let timing = Timing { echo_path_s: 0.0, tap_delivery_s: 0.0, mic_processing_s: 0.0, tap_chunk: 512 };
    let current = run_current(&mic);
    println!("\n== Background talker {below_db:.0} dB quieter than you ==");
    println!("  segments: you 0-5 s, 10-17 s | colleague 3-20.5 s (alone 5-10 s, 17-20.5 s)");
    let bg = |x: &[f32]| 10.0 * ((rms(window(x, 5.5, 10.0)).powi(2) + rms(window(x, 17.5, 20.5)).powi(2)) / 2.0).log10();
    let me = |x: &[f32]| 10.0 * ((rms(window(x, 0.3, 3.0)).powi(2) + rms(window(x, 10.5, 16.5)).powi(2)) / 2.0).log10();
    println!("  colleague alone, at mic       : {:6.1} dBFS", bg(&mic));
    println!("  {:<14} colleague {:+6.1} dB | your voice {:+5.1} dB", "current app", bg(&current) - bg(&mic), me(&current) - me(&mic));
    for (name, iso) in [("isolation Light", VoiceIsolation::Light), ("isolation Balanced", VoiceIsolation::Balanced), ("isolation Strong", VoiceIsolation::Strong)] {
        let (new, ..) = run_pipeline(&mic, None, iso, &timing);
        println!("  {:<14} colleague {:+6.1} dB | your voice {:+5.1} dB", name, bg(&new) - bg(&mic), me(&new) - me(&mic));
        if write && iso == VoiceIsolation::Balanced {
            write_wav(&out_dir.join("background_1_mic_input.wav"), &mic);
            write_wav(&out_dir.join("background_2_current_app.wav"), &current);
            write_wav(&out_dir.join("background_3_new_pipeline.wav"), &new);
        }
    }
}

/// You alone, alternating normal and quieter speech: the gate must not treat
/// your own soft speech as background.
fn soft_voice_scenario(voices: &Path) {
    let you = normalize(&read_wav(&voices.join("you.wav")), -24.0);
    let total = 20 * FS;
    let mut near = vec![0.0; total];
    let soft = |db: f32| 10f32.powf(db / 20.0);
    // 0-4 s normal, 4-8 s -6 dB, 8-12 s normal, 12-16 s -10 dB, 16-20 s normal.
    for (i, (from, to)) in [(0.0, 4.0), (4.0, 8.0), (8.0, 12.0), (12.0, 16.0), (16.0, 20.0)].iter().enumerate() {
        let gain = match i { 1 => soft(-6.0), 3 => soft(-10.0), _ => 1.0 };
        let clip: Vec<f32> = you.iter().map(|s| s * gain).collect();
        place(&mut near, &clip, *from, *from, *to);
    }
    let mut mic = convolve(&near, &room_ir(0.15, 12.0, 21));
    add_noise(&mut mic, -55.0, 5);
    let timing = Timing { echo_path_s: 0.0, tap_delivery_s: 0.0, mic_processing_s: 0.0, tap_chunk: 512 };
    let current = run_current(&mic);
    println!("\n== Your own voice getting quieter (no one else talking) ==");
    let seg = |x: &[f32], a: f32, b: f32| db(rms(window(x, a, b)));
    for (name, iso) in [("isolation Light", VoiceIsolation::Light), ("isolation Balanced", VoiceIsolation::Balanced), ("isolation Strong", VoiceIsolation::Strong)] {
        let (new, ..) = run_pipeline(&mic, None, iso, &timing);
        let change = |a: f32, b: f32| (seg(&new, a, b) - seg(&mic, a, b)) - (seg(&current, a, b) - seg(&mic, a, b));
        println!("  {:<18} vs current app: normal {:+5.1} dB | 6 dB softer {:+5.1} dB | 10 dB softer {:+5.1} dB", name, change(8.3, 12.0), change(4.3, 8.0), change(12.3, 16.0));
    }
}

/// A home-office noise bed (desk fan + a motorbike passing outside) and what
/// the pipeline leaves of it, for the promo video's before/after.
fn noise_bed(out_dir: &Path) {
    let total = 10 * FS;
    let mut rng = Lcg(7);
    let (mut b0, mut b1, mut b2) = (0.0_f32, 0.0_f32, 0.0_f32);
    let mut mic = vec![0.0; total];
    for (n, s) in mic.iter_mut().enumerate() {
        let t = n as f32 / FS as f32;
        // Fan: pink-ish noise with a slow blade flutter and a motor hum.
        let w = rng.next();
        b0 = 0.997 * b0 + 0.029 * w;
        b1 = 0.985 * b1 + 0.032 * w;
        b2 = 0.950 * b2 + 0.048 * w;
        let flutter = 1.0 + 0.12 * (2.0 * std::f32::consts::PI * 14.0 * t).sin();
        let hum = 0.004 * (2.0 * std::f32::consts::PI * 120.0 * t).sin() + 0.002 * (2.0 * std::f32::consts::PI * 240.0 * t).sin();
        *s = 0.9 * (b0 + b1 + b2 + 0.1 * w) * flutter + hum;
        // Motorbike: harmonic engine note that rises, passes and fades (2-7 s).
        if (2.0..7.0).contains(&t) {
            let x = (t - 2.0) / 5.0;
            let env = (std::f32::consts::PI * x).sin().powi(2);
            let f0 = 62.0 + 26.0 * x;
            let engine: f32 = (1..9).map(|k| (2.0 * std::f32::consts::PI * f0 * k as f32 * t).sin() / k as f32).sum();
            *s += 0.05 * env * engine * (1.0 + 0.3 * (2.0 * std::f32::consts::PI * 9.0 * t).sin());
        }
    }
    let mic = normalize(&mic, -30.0);
    let timing = Timing { echo_path_s: 0.0, tap_delivery_s: 0.0, mic_processing_s: 0.0, tap_chunk: 512 };
    let (out, ..) = run_pipeline(&mic, None, VoiceIsolation::Balanced, &timing);
    println!("noise bed: {:.1} dBFS in, {:.1} dBFS out (last 6 s)", db(rms(window(&mic, 4.0, 10.0))), db(rms(window(&out, 4.0, 10.0))));
    write_wav(&out_dir.join("noise_bed_mic.wav"), &mic);
    write_wav(&out_dir.join("noise_bed_pabulong.wav"), &out);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("noise-bed") {
        let out_dir = PathBuf::from(args.get(2).map(String::as_str).unwrap_or("sim/out"));
        std::fs::create_dir_all(&out_dir).unwrap();
        return noise_bed(&out_dir);
    }
    let voices = PathBuf::from(args.get(1).map(String::as_str).unwrap_or("sim/voices"));
    let out_dir = PathBuf::from(args.get(2).map(String::as_str).unwrap_or("sim/out"));
    std::fs::create_dir_all(&out_dir).unwrap();
    println!("neural noise suppression: {}", NoiseProcessor::standard(NoiseMode::Balanced).uses_neural());

    let mac = Timing { echo_path_s: 0.02, tap_delivery_s: 0.008, mic_processing_s: 0.006, tap_chunk: 512 };
    echo_scenario(&voices, &out_dir, &mac, "built-in speakers", true);
    let bluetooth = Timing { echo_path_s: 0.18, tap_delivery_s: 0.008, mic_processing_s: 0.006, tap_chunk: 512 };
    echo_scenario(&voices, &out_dir, &bluetooth, "Bluetooth speaker", false);
    let late_tap = Timing { echo_path_s: 0.012, tap_delivery_s: 0.025, mic_processing_s: 0.004, tap_chunk: 1024 };
    echo_scenario(&voices, &out_dir, &late_tap, "worst case: late, bursty tap", false);

    background_scenario(&voices, &out_dir, 18.0, true);
    background_scenario(&voices, &out_dir, 12.0, false);
    soft_voice_scenario(&voices);
    println!("\nWAVs written to {}", out_dir.display());
}
