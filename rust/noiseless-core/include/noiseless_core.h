#pragma once
// C interface to the portable Rust noise-suppression core (noiseless-core).
// Link against libnoiseless_core.a (or the cdylib). Frames are 48 kHz mono float.

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NlrNoiseProcessor NlrNoiseProcessor;

// mode: 0 = light, 1 = balanced, 2 = max.
NlrNoiseProcessor *nlr_noise_processor_create(uint32_t mode, uint32_t sample_rate, uint32_t frame_length);
void nlr_noise_processor_destroy(NlrNoiseProcessor *processor);
void nlr_noise_processor_reset(NlrNoiseProcessor *processor);
void nlr_noise_processor_set_mode(NlrNoiseProcessor *processor, uint32_t mode);
// Returns false on a null pointer or a frame_length that differs from creation.
bool nlr_noise_processor_process(NlrNoiseProcessor *processor, const float *input, float *output, uint32_t frame_length);
float nlr_noise_processor_last_input_rms(const NlrNoiseProcessor *processor);
float nlr_noise_processor_last_output_rms(const NlrNoiseProcessor *processor);
bool nlr_noise_processor_last_voice_active(const NlrNoiseProcessor *processor);
bool nlr_noise_processor_uses_neural(const NlrNoiseProcessor *processor);
// True when the library was built with --features rnnoise.
bool nlr_rnnoise_available(void);

// ---------------------------------------------------------------------------
// Full mic chain: echo canceller -> noise suppressor -> background-voice gate.
// The same code runs on macOS and Windows.
//
// Threading: call nlr_pipeline_process from ONE audio thread. push_reference,
// the setters and nlr_pipeline_metrics may be called from any thread.
// Host times are nanoseconds on one monotonic clock shared by mic and speaker
// timestamps (mach host time on macOS, QPC on Windows).

typedef struct NlrPipeline NlrPipeline;

typedef struct NlrPipelineMetrics {
    float input_rms;
    float output_rms;
    float voice_probability;
    float echo_reduction_db;     // echo removed by the canceller, far end talking alone
    float echo_delay_ms;         // echo path delay found by the canceller
    float reference_lookback_ms; // how far behind the mic the speaker reference is read
    float voice_gate_gain;       // background-voice gate gain (1 = open)
    bool far_end_active;
} NlrPipelineMetrics;

// mode: 0 light, 1 balanced, 2 max. isolation: 0 off, 1 light, 2 balanced, 3 strong.
NlrPipeline *nlr_pipeline_create(uint32_t mode, bool echo_cancellation, uint32_t isolation);
void nlr_pipeline_destroy(NlrPipeline *handle);
// One 480-sample 48 kHz mono frame whose first sample was captured at host_time_ns.
bool nlr_pipeline_process(const NlrPipeline *handle, const float *mic, float *out, uint32_t frame_length, uint64_t host_time_ns);
// Speaker output (48 kHz mono) whose first sample was played at host_time_ns.
void nlr_pipeline_push_reference(const NlrPipeline *handle, const float *samples, uint32_t count, uint64_t host_time_ns);
void nlr_pipeline_set_mode(const NlrPipeline *handle, uint32_t mode);
void nlr_pipeline_set_voice_isolation(const NlrPipeline *handle, uint32_t isolation);
void nlr_pipeline_set_echo_cancellation(const NlrPipeline *handle, bool enabled);
void nlr_pipeline_reset(const NlrPipeline *handle);
bool nlr_pipeline_uses_neural(const NlrPipeline *handle);
NlrPipelineMetrics nlr_pipeline_metrics(const NlrPipeline *handle);

#ifdef __cplusplus
}
#endif
