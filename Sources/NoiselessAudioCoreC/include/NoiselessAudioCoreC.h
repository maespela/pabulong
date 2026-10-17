#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum NLNoiseMode {
    NLNoiseModeLight = 0,
    NLNoiseModeBalanced = 1,
    NLNoiseModeMax = 2
} NLNoiseMode;

typedef struct NLRingBuffer NLRingBuffer;
typedef struct NLNoiseProcessor NLNoiseProcessor;
typedef struct NLSharedRing NLSharedRing;

typedef struct NLSharedRingHealth {
    uint32_t available_read_frames;
    uint32_t available_write_frames;
    uint64_t underrun_count;
    uint64_t overrun_count;
    bool producer_attached;
    uint32_t consumer_running_count;
} NLSharedRingHealth;

NLRingBuffer *nl_ring_buffer_create(uint32_t frame_capacity, uint32_t frame_length);
void nl_ring_buffer_destroy(NLRingBuffer *ring);
bool nl_ring_buffer_write(NLRingBuffer *ring, const float *samples, uint32_t frame_length);
bool nl_ring_buffer_read(NLRingBuffer *ring, float *samples, uint32_t frame_length);
uint32_t nl_ring_buffer_available_read(const NLRingBuffer *ring);
uint32_t nl_ring_buffer_available_write(const NLRingBuffer *ring);
uint32_t nl_ring_buffer_capacity(const NLRingBuffer *ring);
void nl_ring_buffer_clear(NLRingBuffer *ring);

// True when the library was built with the RNNoise neural suppressor vendored
// in (see Scripts/vendor-rnnoise.sh). When false, processors fall back to the
// built-in spectral-subtraction suppressor.
bool nl_rnnoise_available(void);

// True when this specific processor instance is running the neural backend.
bool nl_noise_processor_uses_neural(const NLNoiseProcessor *processor);

NLNoiseProcessor *nl_noise_processor_create(NLNoiseMode mode, uint32_t sample_rate, uint32_t frame_length);
void nl_noise_processor_destroy(NLNoiseProcessor *processor);
void nl_noise_processor_reset(NLNoiseProcessor *processor);
void nl_noise_processor_set_mode(NLNoiseProcessor *processor, NLNoiseMode mode);
bool nl_noise_processor_process(NLNoiseProcessor *processor, const float *input, float *output, uint32_t frame_length);
float nl_noise_processor_last_input_rms(const NLNoiseProcessor *processor);
float nl_noise_processor_last_output_rms(const NLNoiseProcessor *processor);
bool nl_noise_processor_last_voice_active(const NLNoiseProcessor *processor);

float nl_frame_rms(const float *samples, uint32_t frame_length);
void nl_limiter_process(float *samples, uint32_t frame_length, float ceiling);

NLSharedRing *nl_shared_ring_open(const char *name, bool create, uint32_t frame_capacity, uint32_t frame_length);
void nl_shared_ring_close(NLSharedRing *ring);
bool nl_shared_ring_write(NLSharedRing *ring, const float *samples, uint32_t frame_length);
bool nl_shared_ring_write_latest(NLSharedRing *ring, const float *samples, uint32_t frame_length);
bool nl_shared_ring_read(NLSharedRing *ring, float *samples, uint32_t frame_length);
bool nl_shared_ring_read_latest(NLSharedRing *ring, float *samples, uint32_t frame_length, uint32_t max_buffered_frames);
void nl_shared_ring_clear(NLSharedRing *ring);
void nl_shared_ring_set_producer_attached(NLSharedRing *ring, bool attached);
void nl_shared_ring_set_consumer_running_count(NLSharedRing *ring, uint32_t count);
NLSharedRingHealth nl_shared_ring_health(const NLSharedRing *ring);
const char *nl_shared_ring_default_name(void);

#ifdef __cplusplus
}
#endif

// Shared Rust DSP core (echo canceller, RNNoise, background-voice gate).
#include "noiseless_core.h"
