//! C ABI so the Swift (macOS) and Windows apps call the same core.
//!
//! All functions tolerate null pointers. Panics abort (the release profile uses
//! `panic = "abort"`), so nothing unwinds across the boundary.

use crate::mode::NoiseMode;
use crate::pipeline::VoicePipeline;
use crate::processor::NoiseProcessor;
use crate::reference::ReferenceQueue;
use crate::voice_gate::VoiceIsolation;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// ---------------------------------------------------------------- pipeline

const NO_CHANGE: u32 = u32::MAX;

/// Full mic chain (echo canceller → noise suppressor → background-voice gate).
///
/// Threading contract:
/// - `nlr_pipeline_process` must be called from one thread at a time (the
///   audio processing thread).
/// - `nlr_pipeline_push_reference`, the setters and the metric getters may be
///   called from any thread. Setters take effect at the start of the next frame.
pub struct PipelineHandle {
    dsp: UnsafeCell<VoicePipeline>,
    reference: ReferenceQueue,
    pending_mode: AtomicU32,
    pending_isolation: AtomicU32,
    pending_echo: AtomicU32,
    reset_requested: AtomicBool,
    uses_neural: bool,
    // Published metrics (f32 bits).
    m_input_rms: AtomicU32,
    m_output_rms: AtomicU32,
    m_vad: AtomicU32,
    m_erle: AtomicU32,
    m_delay: AtomicU32,
    m_gate: AtomicU32,
    m_lookback: AtomicU32,
    m_far_active: AtomicBool,
}

// SAFETY: `dsp` is only touched inside `nlr_pipeline_process`, which the
// contract restricts to one thread at a time; everything else is atomic or
// internally locked (ReferenceQueue).
unsafe impl Sync for PipelineHandle {}
unsafe impl Send for PipelineHandle {}

#[repr(C)]
#[derive(Default)]
pub struct NlrPipelineMetrics {
    pub input_rms: f32,
    pub output_rms: f32,
    pub voice_probability: f32,
    /// Echo removed by the linear canceller (dB), far end talking alone.
    pub echo_reduction_db: f32,
    /// Echo path delay found by the canceller (ms), after the lookback.
    pub echo_delay_ms: f32,
    /// How far behind the mic the speaker reference is read (ms).
    pub reference_lookback_ms: f32,
    /// Gain the background-voice gate is applying (1 = open).
    pub voice_gate_gain: f32,
    pub far_end_active: bool,
}

fn store(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Ordering::Relaxed);
}
fn load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

/// mode: 0 light, 1 balanced, 2 max. isolation: 0 off, 1 light, 2 balanced, 3 strong.
#[no_mangle]
pub extern "C" fn nlr_pipeline_create(mode: u32, echo_cancellation: bool, isolation: u32) -> *mut PipelineHandle {
    let dsp = VoicePipeline::new(NoiseMode::from_u32(mode), echo_cancellation, VoiceIsolation::from_u32(isolation));
    let uses_neural = dsp.uses_neural();
    Box::into_raw(Box::new(PipelineHandle {
        dsp: UnsafeCell::new(dsp),
        reference: ReferenceQueue::new(crate::SAMPLE_RATE),
        pending_mode: AtomicU32::new(NO_CHANGE),
        pending_isolation: AtomicU32::new(NO_CHANGE),
        pending_echo: AtomicU32::new(NO_CHANGE),
        reset_requested: AtomicBool::new(false),
        uses_neural,
        m_input_rms: AtomicU32::new(0),
        m_output_rms: AtomicU32::new(0),
        m_vad: AtomicU32::new(0),
        m_erle: AtomicU32::new(0),
        m_delay: AtomicU32::new(0),
        m_gate: AtomicU32::new(1.0_f32.to_bits()),
        m_lookback: AtomicU32::new(0),
        m_far_active: AtomicBool::new(false),
    }))
}

/// # Safety
/// `handle` must be null or a pointer from `nlr_pipeline_create`, not used
/// concurrently and not already destroyed.
#[no_mangle]
pub unsafe extern "C" fn nlr_pipeline_destroy(handle: *mut PipelineHandle) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

/// Processes one 480-sample 48 kHz mono frame whose first sample was captured
/// at `host_time_ns`. Returns false on bad arguments.
///
/// # Safety
/// `mic`/`out` must point to `frame_length` floats; `handle` must be live; at
/// most one thread may be inside this function for a given handle.
#[no_mangle]
pub unsafe extern "C" fn nlr_pipeline_process(
    handle: *const PipelineHandle,
    mic: *const f32,
    out: *mut f32,
    frame_length: u32,
    host_time_ns: u64,
) -> bool {
    let Some(h) = handle.as_ref() else { return false };
    if mic.is_null() || out.is_null() {
        return false;
    }
    let dsp = &mut *h.dsp.get();
    if h.reset_requested.swap(false, Ordering::AcqRel) {
        dsp.reset();
        h.reference.clear();
    }
    let mode = h.pending_mode.swap(NO_CHANGE, Ordering::AcqRel);
    if mode != NO_CHANGE {
        dsp.set_mode(NoiseMode::from_u32(mode));
    }
    let isolation = h.pending_isolation.swap(NO_CHANGE, Ordering::AcqRel);
    if isolation != NO_CHANGE {
        dsp.set_voice_isolation(VoiceIsolation::from_u32(isolation));
    }
    let echo = h.pending_echo.swap(NO_CHANGE, Ordering::AcqRel);
    if echo != NO_CHANGE {
        dsp.set_echo_cancellation(echo == 1);
    }

    let n = frame_length as usize;
    let mic = std::slice::from_raw_parts(mic, n);
    let out = std::slice::from_raw_parts_mut(out, n);
    if dsp.process(mic, out, Some(&h.reference), host_time_ns).is_err() {
        return false;
    }
    let m = dsp.metrics();
    store(&h.m_input_rms, m.noise.input_rms);
    store(&h.m_output_rms, m.noise.output_rms);
    store(&h.m_vad, m.noise.vad);
    store(&h.m_erle, m.echo.erle_db);
    store(&h.m_delay, m.echo.delay_ms);
    store(&h.m_gate, m.voice_gate_gain);
    store(&h.m_lookback, h.reference.lookback_ms());
    h.m_far_active.store(m.echo.far_end_active, Ordering::Relaxed);
    true
}

/// Supplies speaker-output audio (48 kHz mono) whose first sample was played
/// at `host_time_ns`. Any thread.
///
/// # Safety
/// `samples` must point to `count` floats; `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn nlr_pipeline_push_reference(handle: *const PipelineHandle, samples: *const f32, count: u32, host_time_ns: u64) {
    let Some(h) = handle.as_ref() else { return };
    if samples.is_null() || count == 0 {
        return;
    }
    h.reference.push(std::slice::from_raw_parts(samples, count as usize), host_time_ns);
}

/// # Safety
/// `handle` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_pipeline_set_mode(handle: *const PipelineHandle, mode: u32) {
    if let Some(h) = handle.as_ref() {
        h.pending_mode.store(mode.min(2), Ordering::Release);
    }
}

/// # Safety
/// `handle` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_pipeline_set_voice_isolation(handle: *const PipelineHandle, isolation: u32) {
    if let Some(h) = handle.as_ref() {
        h.pending_isolation.store(isolation.min(3), Ordering::Release);
    }
}

/// # Safety
/// `handle` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_pipeline_set_echo_cancellation(handle: *const PipelineHandle, enabled: bool) {
    if let Some(h) = handle.as_ref() {
        h.pending_echo.store(u32::from(enabled), Ordering::Release);
    }
}

/// Clears all adaptive state at the start of the next frame.
///
/// # Safety
/// `handle` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_pipeline_reset(handle: *const PipelineHandle) {
    if let Some(h) = handle.as_ref() {
        h.reset_requested.store(true, Ordering::Release);
    }
}

/// # Safety
/// `handle` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_pipeline_uses_neural(handle: *const PipelineHandle) -> bool {
    handle.as_ref().is_some_and(|h| h.uses_neural)
}

/// # Safety
/// `handle` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_pipeline_metrics(handle: *const PipelineHandle) -> NlrPipelineMetrics {
    let Some(h) = handle.as_ref() else { return NlrPipelineMetrics::default() };
    NlrPipelineMetrics {
        input_rms: load(&h.m_input_rms),
        output_rms: load(&h.m_output_rms),
        voice_probability: load(&h.m_vad),
        echo_reduction_db: load(&h.m_erle),
        echo_delay_ms: load(&h.m_delay),
        reference_lookback_ms: load(&h.m_lookback),
        voice_gate_gain: load(&h.m_gate),
        far_end_active: h.m_far_active.load(Ordering::Relaxed),
    }
}

// ---------------------------------------------------- bare noise processor

#[no_mangle]
pub extern "C" fn nlr_noise_processor_create(mode: u32, sample_rate: u32, frame_length: u32) -> *mut NoiseProcessor {
    match NoiseProcessor::new(NoiseMode::from_u32(mode), sample_rate, frame_length as usize) {
        Some(processor) => Box::into_raw(Box::new(processor)),
        None => std::ptr::null_mut(),
    }
}

/// # Safety
/// `processor` must be null or a pointer from `nlr_noise_processor_create`
/// that has not already been destroyed.
#[no_mangle]
pub unsafe extern "C" fn nlr_noise_processor_destroy(processor: *mut NoiseProcessor) {
    if !processor.is_null() {
        drop(Box::from_raw(processor));
    }
}

/// # Safety
/// `processor` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_noise_processor_reset(processor: *mut NoiseProcessor) {
    if let Some(p) = processor.as_mut() {
        p.reset();
    }
}

/// # Safety
/// `processor` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_noise_processor_set_mode(processor: *mut NoiseProcessor, mode: u32) {
    if let Some(p) = processor.as_mut() {
        p.set_mode(NoiseMode::from_u32(mode));
    }
}

/// # Safety
/// `input`/`output` must point to `frame_length` floats; `processor` null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_noise_processor_process(
    processor: *mut NoiseProcessor,
    input: *const f32,
    output: *mut f32,
    frame_length: u32,
) -> bool {
    let Some(p) = processor.as_mut() else { return false };
    if input.is_null() || output.is_null() {
        return false;
    }
    let n = frame_length as usize;
    p.process(std::slice::from_raw_parts(input, n), std::slice::from_raw_parts_mut(output, n)).is_ok()
}

/// # Safety
/// `processor` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_noise_processor_last_input_rms(processor: *const NoiseProcessor) -> f32 {
    processor.as_ref().map_or(0.0, |p| p.metrics().input_rms)
}

/// # Safety
/// `processor` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_noise_processor_last_output_rms(processor: *const NoiseProcessor) -> f32 {
    processor.as_ref().map_or(0.0, |p| p.metrics().output_rms)
}

/// # Safety
/// `processor` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_noise_processor_last_voice_active(processor: *const NoiseProcessor) -> bool {
    processor.as_ref().is_some_and(|p| p.metrics().voice_active)
}

/// # Safety
/// `processor` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn nlr_noise_processor_uses_neural(processor: *const NoiseProcessor) -> bool {
    processor.as_ref().is_some_and(|p| p.uses_neural())
}

#[no_mangle]
pub extern "C" fn nlr_rnnoise_available() -> bool {
    NoiseProcessor::neural_available()
}
