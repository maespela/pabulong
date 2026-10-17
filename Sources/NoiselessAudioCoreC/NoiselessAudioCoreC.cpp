#include "NoiselessAudioCoreC.h"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstddef>
#include <cstring>
#include <fcntl.h>
#include <memory>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
#include <vector>

#if defined(NL_HAS_RNNOISE)
#include "rnnoise.h"
#endif

struct NLRingBuffer {
    const uint32_t capacity;
    const uint32_t frameLength;
    std::vector<float> storage;
    std::atomic<uint32_t> readIndex;
    std::atomic<uint32_t> writeIndex;

    NLRingBuffer(uint32_t frameCapacity, uint32_t length)
        : capacity(frameCapacity + 1),
          frameLength(length),
          storage(static_cast<size_t>(frameCapacity + 1) * length, 0.0f),
          readIndex(0),
          writeIndex(0) {}
};

// Short-time Fourier transform parameters. 512-point FFT with 50% overlap and a
// square-root Hann window on both analysis and synthesis satisfies the
// constant-overlap-add condition (the windows multiply to a Hann, which sums to
// unity at 50% overlap), so unity spectral gain reconstructs the input exactly.
static constexpr uint32_t kNLFftSize = 512;
static constexpr uint32_t kNLFftHop = 256;
static constexpr uint32_t kNLFftBins = kNLFftSize / 2 + 1; // 257
static constexpr float kNLPi = 3.14159265358979323846f;

// In-place iterative radix-2 FFT. `inverse` computes the IFFT (with 1/N scaling).
static void nl_fft(std::vector<float> &re, std::vector<float> &im, bool inverse) {
    const int n = static_cast<int>(re.size());
    for (int i = 1, j = 0; i < n; ++i) {
        int bit = n >> 1;
        for (; j & bit; bit >>= 1) {
            j ^= bit;
        }
        j ^= bit;
        if (i < j) {
            std::swap(re[i], re[j]);
            std::swap(im[i], im[j]);
        }
    }
    for (int len = 2; len <= n; len <<= 1) {
        const float ang = 2.0f * kNLPi / len * (inverse ? 1.0f : -1.0f);
        const float wr = std::cos(ang);
        const float wi = std::sin(ang);
        for (int i = 0; i < n; i += len) {
            float cwr = 1.0f;
            float cwi = 0.0f;
            for (int k = 0; k < len / 2; ++k) {
                const float xr = re[i + k + len / 2] * cwr - im[i + k + len / 2] * cwi;
                const float xi = re[i + k + len / 2] * cwi + im[i + k + len / 2] * cwr;
                const float ur = re[i + k];
                const float ui = im[i + k];
                re[i + k] = ur + xr;
                im[i + k] = ui + xi;
                re[i + k + len / 2] = ur - xr;
                im[i + k + len / 2] = ui - xi;
                const float ncwr = cwr * wr - cwi * wi;
                cwi = cwr * wi + cwi * wr;
                cwr = ncwr;
            }
        }
    }
    if (inverse) {
        for (int i = 0; i < n; ++i) {
            re[i] /= n;
            im[i] /= n;
        }
    }
}

struct NLNoiseProcessor {
    NLNoiseMode mode;
    uint32_t sampleRate;
    uint32_t frameLength;

    // Streaming STFT state.
    std::vector<float> inFifo;     // pending input samples
    std::vector<float> outFifo;    // processed output awaiting emit
    std::vector<float> analysis;   // [N] sliding analysis window
    std::vector<float> ola;        // [N] overlap-add accumulator
    std::vector<float> window;     // [N] sqrt-Hann window
    std::vector<float> noiseMag;   // [BINS] estimated noise magnitude spectrum
    std::vector<float> gainSmooth; // [BINS] time-smoothed spectral gain
    std::vector<float> re;         // [N] FFT scratch (real)
    std::vector<float> im;         // [N] FFT scratch (imag)
    uint32_t framesSeen;
    float highPassPrevInput;
    float highPassPrevOutput;

#if defined(NL_HAS_RNNOISE)
    DenoiseState *rnnoise;         // neural suppressor (nullptr if unavailable/unsupported frame size)
    std::vector<float> rnIn;       // [frameLength] int16-scaled input scratch
    std::vector<float> rnOut;      // [frameLength] int16-scaled output scratch
    float rnGateGain;              // smoothed post-RNNoise silence expander gain
#endif

    float lastInputRMS;
    float lastOutputRMS;
    bool lastVoiceActive;

    NLNoiseProcessor(NLNoiseMode processorMode, uint32_t rate, uint32_t length)
        : mode(processorMode),
          sampleRate(rate),
          frameLength(length),
          analysis(kNLFftSize, 0.0f),
          ola(kNLFftSize, 0.0f),
          window(kNLFftSize, 0.0f),
          noiseMag(kNLFftBins, 0.0f),
          gainSmooth(kNLFftBins, 1.0f),
          re(kNLFftSize, 0.0f),
          im(kNLFftSize, 0.0f),
          framesSeen(0),
          highPassPrevInput(0.0f),
          highPassPrevOutput(0.0f),
#if defined(NL_HAS_RNNOISE)
          rnnoise(nullptr),
          rnIn(length, 0.0f),
          rnOut(length, 0.0f),
          rnGateGain(1.0f),
#endif
          lastInputRMS(0.0f),
          lastOutputRMS(0.0f),
          lastVoiceActive(false) {
        for (uint32_t i = 0; i < kNLFftSize; ++i) {
            const float hann = 0.5f * (1.0f - std::cos(2.0f * kNLPi * static_cast<float>(i) / static_cast<float>(kNLFftSize)));
            window[i] = std::sqrt(hann);
        }
    }
};

static float high_pass_sample(NLNoiseProcessor *processor, float sample) {
    // One-pole high-pass around 100 Hz at 48 kHz. This removes rumble before
    // suppression while preserving speech fundamentals well enough for a mic path.
    static constexpr float kHighPassAlpha = 0.98708f;
    const float output = kHighPassAlpha * (processor->highPassPrevOutput + sample - processor->highPassPrevInput);
    processor->highPassPrevInput = sample;
    processor->highPassPrevOutput = output;
    return output;
}

// Wet/dry mix for the neural backend, giving the suppression modes meaning
// (RNNoise itself is a single fixed model). 1.0 = fully denoised.
static float mode_wet_mix(NLNoiseMode mode) {
    switch (mode) {
        case NLNoiseModeLight: return 0.75f;
        case NLNoiseModeBalanced: return 1.0f;
        case NLNoiseModeMax: return 1.0f;
    }
    return 1.0f;
}

static float mode_vad_open_threshold(NLNoiseMode mode) {
    switch (mode) {
        case NLNoiseModeLight: return 0.25f;
        case NLNoiseModeBalanced: return 0.32f;
        case NLNoiseModeMax: return 0.38f;
    }
    return 0.32f;
}

static float mode_silence_gain(NLNoiseMode mode) {
    switch (mode) {
        case NLNoiseModeLight: return 0.55f;
        case NLNoiseModeBalanced: return 0.22f;
        case NLNoiseModeMax: return 0.08f;
    }
    return 0.22f;
}

// RNNoise operates on 480-sample (10 ms) frames at 48 kHz.
static constexpr uint32_t kNLRNNoiseFrame = 480;

struct NLSharedRingHeader {
    uint32_t magic;
    uint32_t capacity;
    uint32_t frameLength;
    uint32_t readIndex;
    uint32_t writeIndex;
    uint64_t underrunCount;
    uint64_t overrunCount;
    uint32_t producerAttached;
    uint32_t consumerRunningCount;
};

struct NLSharedRing {
    int fd;
    size_t byteSize;
    NLSharedRingHeader *header;
    float *storage;
};

static constexpr uint32_t NLSharedRingMagic = 0x4E4C5348; // NLSH

static uint32_t next_index(const NLRingBuffer *ring, uint32_t index) {
    return (index + 1) % ring->capacity;
}

// Spectral floor: the minimum per-bin gain, i.e. how much residual noise is
// left in. Lower = more aggressive suppression (but more risk of artifacts).
static float mode_spectral_floor(NLNoiseMode mode) {
    switch (mode) {
        case NLNoiseModeLight: return 0.30f;
        case NLNoiseModeBalanced: return 0.14f;
        case NLNoiseModeMax: return 0.05f;
    }
    return 0.14f;
}

// Over-subtraction factor: how much of the estimated noise magnitude to remove.
static float mode_oversubtraction(NLNoiseMode mode) {
    switch (mode) {
        case NLNoiseModeLight: return 1.0f;
        case NLNoiseModeBalanced: return 1.6f;
        case NLNoiseModeMax: return 2.4f;
    }
    return 1.6f;
}

NLRingBuffer *nl_ring_buffer_create(uint32_t frame_capacity, uint32_t frame_length) {
    if (frame_capacity == 0 || frame_length == 0) {
        return nullptr;
    }

    try {
        return new NLRingBuffer(frame_capacity, frame_length);
    } catch (...) {
        return nullptr;
    }
}

void nl_ring_buffer_destroy(NLRingBuffer *ring) {
    delete ring;
}

bool nl_ring_buffer_write(NLRingBuffer *ring, const float *samples, uint32_t frame_length) {
    if (!ring || !samples || frame_length != ring->frameLength) {
        return false;
    }

    const uint32_t write = ring->writeIndex.load(std::memory_order_relaxed);
    const uint32_t next = next_index(ring, write);
    const uint32_t read = ring->readIndex.load(std::memory_order_acquire);
    if (next == read) {
        return false;
    }

    std::memcpy(&ring->storage[static_cast<size_t>(write) * ring->frameLength], samples, sizeof(float) * ring->frameLength);
    ring->writeIndex.store(next, std::memory_order_release);
    return true;
}

bool nl_ring_buffer_read(NLRingBuffer *ring, float *samples, uint32_t frame_length) {
    if (!ring || !samples || frame_length != ring->frameLength) {
        return false;
    }

    const uint32_t read = ring->readIndex.load(std::memory_order_relaxed);
    const uint32_t write = ring->writeIndex.load(std::memory_order_acquire);
    if (read == write) {
        std::fill(samples, samples + frame_length, 0.0f);
        return false;
    }

    std::memcpy(samples, &ring->storage[static_cast<size_t>(read) * ring->frameLength], sizeof(float) * ring->frameLength);
    ring->readIndex.store(next_index(ring, read), std::memory_order_release);
    return true;
}

uint32_t nl_ring_buffer_available_read(const NLRingBuffer *ring) {
    if (!ring) {
        return 0;
    }
    const uint32_t read = ring->readIndex.load(std::memory_order_acquire);
    const uint32_t write = ring->writeIndex.load(std::memory_order_acquire);
    return write >= read ? write - read : ring->capacity - read + write;
}

uint32_t nl_ring_buffer_available_write(const NLRingBuffer *ring) {
    if (!ring) {
        return 0;
    }
    return ring->capacity - 1 - nl_ring_buffer_available_read(ring);
}

uint32_t nl_ring_buffer_capacity(const NLRingBuffer *ring) {
    return ring ? ring->capacity - 1 : 0;
}

void nl_ring_buffer_clear(NLRingBuffer *ring) {
    if (!ring) {
        return;
    }
    ring->readIndex.store(0, std::memory_order_release);
    ring->writeIndex.store(0, std::memory_order_release);
}

bool nl_rnnoise_available(void) {
#if defined(NL_HAS_RNNOISE)
    return true;
#else
    return false;
#endif
}

bool nl_noise_processor_uses_neural(const NLNoiseProcessor *processor) {
#if defined(NL_HAS_RNNOISE)
    return processor != nullptr && processor->rnnoise != nullptr;
#else
    (void)processor;
    return false;
#endif
}

NLNoiseProcessor *nl_noise_processor_create(NLNoiseMode mode, uint32_t sample_rate, uint32_t frame_length) {
    if (sample_rate == 0 || frame_length == 0) {
        return nullptr;
    }

    try {
        NLNoiseProcessor *processor = new NLNoiseProcessor(mode, sample_rate, frame_length);
#if defined(NL_HAS_RNNOISE)
        // RNNoise only supports 48 kHz / 480-sample frames; otherwise keep the
        // spectral fallback so the processor still works at other geometries.
        if (sample_rate == 48000 && frame_length == kNLRNNoiseFrame) {
            processor->rnnoise = rnnoise_create(nullptr);
        }
#endif
        return processor;
    } catch (...) {
        return nullptr;
    }
}

void nl_noise_processor_destroy(NLNoiseProcessor *processor) {
#if defined(NL_HAS_RNNOISE)
    if (processor && processor->rnnoise) {
        rnnoise_destroy(processor->rnnoise);
        processor->rnnoise = nullptr;
    }
#endif
    delete processor;
}

void nl_noise_processor_reset(NLNoiseProcessor *processor) {
    if (!processor) {
        return;
    }
    std::fill(processor->analysis.begin(), processor->analysis.end(), 0.0f);
    std::fill(processor->ola.begin(), processor->ola.end(), 0.0f);
    std::fill(processor->noiseMag.begin(), processor->noiseMag.end(), 0.0f);
    std::fill(processor->gainSmooth.begin(), processor->gainSmooth.end(), 1.0f);
    processor->inFifo.clear();
    processor->outFifo.clear();
    processor->framesSeen = 0;
    processor->highPassPrevInput = 0.0f;
    processor->highPassPrevOutput = 0.0f;
#if defined(NL_HAS_RNNOISE)
    // RNNoise has no reset entry point; recreate the state to clear its history.
    if (processor->rnnoise) {
        rnnoise_destroy(processor->rnnoise);
        processor->rnnoise = rnnoise_create(nullptr);
    }
    processor->rnGateGain = 1.0f;
#endif
    processor->lastInputRMS = 0.0f;
    processor->lastOutputRMS = 0.0f;
    processor->lastVoiceActive = false;
}

void nl_noise_processor_set_mode(NLNoiseProcessor *processor, NLNoiseMode mode) {
    if (processor) {
        processor->mode = mode;
    }
}

bool nl_noise_processor_process(NLNoiseProcessor *processor, const float *input, float *output, uint32_t frame_length) {
    if (!processor || !input || !output || frame_length != processor->frameLength) {
        return false;
    }

    processor->lastInputRMS = nl_frame_rms(input, frame_length);

#if defined(NL_HAS_RNNOISE)
    // Neural backend: RNNoise processes one 480-sample frame in/out with no
    // added latency. It expects float samples in int16 range, so scale by 32768.
    if (processor->rnnoise) {
        for (uint32_t i = 0; i < frame_length; ++i) {
            float sample = input[i];
            if (!std::isfinite(sample)) {
                sample = 0.0f;
            }
            sample = high_pass_sample(processor, sample);
            processor->rnIn[i] = sample * 32768.0f;
        }
        const float vad = rnnoise_process_frame(processor->rnnoise, processor->rnOut.data(), processor->rnIn.data());
        const float wet = mode_wet_mix(processor->mode);
        const float dry = 1.0f - wet;
        const float targetGate = vad >= mode_vad_open_threshold(processor->mode) ? 1.0f : mode_silence_gain(processor->mode);
        const float attack = 0.55f;
        const float release = 0.12f;
        const float smoothing = targetGate > processor->rnGateGain ? attack : release;
        processor->rnGateGain = processor->rnGateGain + smoothing * (targetGate - processor->rnGateGain);
        for (uint32_t i = 0; i < frame_length; ++i) {
            output[i] = processor->rnGateGain * ((wet * processor->rnOut[i] + dry * processor->rnIn[i]) / 32768.0f);
        }
        nl_limiter_process(output, frame_length, 0.98f);
        processor->lastOutputRMS = nl_frame_rms(output, frame_length);
        processor->lastVoiceActive = vad > 0.5f;
        return true;
    }
#endif

    // Enqueue sanitized input. A single NaN/Inf must not enter the FFT state.
    for (uint32_t index = 0; index < frame_length; ++index) {
        float sample = input[index];
        if (!std::isfinite(sample)) {
            sample = 0.0f;
        }
        processor->inFifo.push_back(high_pass_sample(processor, sample));
    }

    const uint32_t n = kNLFftSize;
    const uint32_t hop = kNLFftHop;
    const uint32_t bins = kNLFftBins;
    const float floorGain = mode_spectral_floor(processor->mode);
    const float oversub = mode_oversubtraction(processor->mode);

    bool voiceActive = false;

    // Process every hop that is now fully buffered.
    while (processor->inFifo.size() >= hop) {
        // Slide the analysis window by one hop and bring in `hop` fresh samples.
        std::memmove(processor->analysis.data(), processor->analysis.data() + hop, (n - hop) * sizeof(float));
        std::memcpy(processor->analysis.data() + (n - hop), processor->inFifo.data(), hop * sizeof(float));
        processor->inFifo.erase(processor->inFifo.begin(), processor->inFifo.begin() + hop);

        for (uint32_t i = 0; i < n; ++i) {
            processor->re[i] = processor->analysis[i] * processor->window[i];
            processor->im[i] = 0.0f;
        }
        nl_fft(processor->re, processor->im, false);

        // Voice-activity decision from the ratio of frame energy to noise energy.
        double frameEnergy = 0.0;
        double noiseEnergy = 0.0;
        for (uint32_t f = 0; f < bins; ++f) {
            const float mag = std::sqrt(processor->re[f] * processor->re[f] + processor->im[f] * processor->im[f]);
            frameEnergy += static_cast<double>(mag) * mag;
            noiseEnergy += static_cast<double>(processor->noiseMag[f]) * processor->noiseMag[f];
        }
        const bool priming = processor->framesSeen < 15;
        const double snr = frameEnergy / (noiseEnergy + 1e-9);
        const bool speech = !priming && snr > 3.0;
        if (speech) {
            voiceActive = true;
        }

        // Per-bin noise tracking and spectral-subtraction gain.
        for (uint32_t f = 0; f < bins; ++f) {
            const float mag = std::sqrt(processor->re[f] * processor->re[f] + processor->im[f] * processor->im[f]);
            if (priming) {
                processor->noiseMag[f] = 0.9f * processor->noiseMag[f] + 0.1f * mag;
            } else if (!speech) {
                processor->noiseMag[f] = 0.95f * processor->noiseMag[f] + 0.05f * mag;
            }

            float gain = 1.0f - oversub * (processor->noiseMag[f] / (mag + 1e-9f));
            if (gain < floorGain) {
                gain = floorGain;
            } else if (gain > 1.0f) {
                gain = 1.0f;
            }
            // Temporal smoothing curbs musical noise from bin-to-bin gain jumps.
            gain = 0.6f * processor->gainSmooth[f] + 0.4f * gain;
            processor->gainSmooth[f] = gain;

            processor->re[f] *= gain;
            processor->im[f] *= gain;
            // Mirror onto the conjugate-symmetric bin so the IFFT stays real.
            if (f > 0 && f < n / 2) {
                processor->re[n - f] *= gain;
                processor->im[n - f] *= gain;
            }
        }

        nl_fft(processor->re, processor->im, true);

        // Synthesis window + overlap-add, then emit one hop of finished output.
        for (uint32_t i = 0; i < n; ++i) {
            processor->ola[i] += processor->re[i] * processor->window[i];
        }
        for (uint32_t i = 0; i < hop; ++i) {
            processor->outFifo.push_back(processor->ola[i]);
        }
        std::memmove(processor->ola.data(), processor->ola.data() + hop, (n - hop) * sizeof(float));
        std::memset(processor->ola.data() + (n - hop), 0, hop * sizeof(float));

        if (processor->framesSeen < 0xffffffffu) {
            processor->framesSeen++;
        }
    }

    // Emit exactly one frame. During the initial fill (STFT latency) there is not
    // yet enough processed audio, so we output silence for those first frames.
    if (processor->outFifo.size() >= frame_length) {
        std::memcpy(output, processor->outFifo.data(), frame_length * sizeof(float));
        processor->outFifo.erase(processor->outFifo.begin(), processor->outFifo.begin() + frame_length);
    } else {
        std::fill(output, output + frame_length, 0.0f);
    }

    nl_limiter_process(output, frame_length, 0.98f);
    processor->lastOutputRMS = nl_frame_rms(output, frame_length);
    processor->lastVoiceActive = voiceActive;
    return true;
}

float nl_noise_processor_last_input_rms(const NLNoiseProcessor *processor) {
    return processor ? processor->lastInputRMS : 0.0f;
}

float nl_noise_processor_last_output_rms(const NLNoiseProcessor *processor) {
    return processor ? processor->lastOutputRMS : 0.0f;
}

bool nl_noise_processor_last_voice_active(const NLNoiseProcessor *processor) {
    return processor ? processor->lastVoiceActive : false;
}

float nl_frame_rms(const float *samples, uint32_t frame_length) {
    if (!samples || frame_length == 0) {
        return 0.0f;
    }
    float sumSquares = 0.0f;
    for (uint32_t index = 0; index < frame_length; ++index) {
        sumSquares += samples[index] * samples[index];
    }
    return std::sqrt(sumSquares / static_cast<float>(frame_length));
}

void nl_limiter_process(float *samples, uint32_t frame_length, float ceiling) {
    if (!samples || frame_length == 0 || ceiling <= 0.0f) {
        return;
    }

    float peak = 0.0f;
    for (uint32_t index = 0; index < frame_length; ++index) {
        peak = std::max(peak, std::fabs(samples[index]));
    }
    if (peak <= ceiling) {
        return;
    }

    const float gain = ceiling / peak;
    for (uint32_t index = 0; index < frame_length; ++index) {
        samples[index] *= gain;
    }
}

static size_t shared_ring_byte_size(uint32_t frame_capacity, uint32_t frame_length) {
    const uint32_t actualCapacity = frame_capacity + 1;
    return sizeof(NLSharedRingHeader) + (static_cast<size_t>(actualCapacity) * frame_length * sizeof(float));
}

static uint32_t shared_next_index(const NLSharedRingHeader *header, uint32_t index) {
    return (index + 1) % header->capacity;
}

static uint32_t shared_available_read(const NLSharedRingHeader *header) {
    const uint32_t read = __atomic_load_n(&header->readIndex, __ATOMIC_ACQUIRE);
    const uint32_t write = __atomic_load_n(&header->writeIndex, __ATOMIC_ACQUIRE);
    return write >= read ? write - read : header->capacity - read + write;
}

static uint32_t shared_available_write(const NLSharedRingHeader *header) {
    return header->capacity - 1 - shared_available_read(header);
}

NLSharedRing *nl_shared_ring_open(const char *name, bool create, uint32_t frame_capacity, uint32_t frame_length) {
    if (!name || frame_capacity == 0 || frame_length == 0) {
        return nullptr;
    }

    const int flags = create ? (O_CREAT | O_RDWR) : O_RDWR;
    const int fd = shm_open(name, flags, 0666);
    if (fd < 0) {
        return nullptr;
    }

    const size_t byteSize = shared_ring_byte_size(frame_capacity, frame_length);
    if (create) {
        // A POSIX shared-memory object can only be sized once on macOS; calling
        // ftruncate again on an already-sized segment fails with EINVAL. Only
        // grow it when it is smaller than required (i.e. freshly created), and
        // tolerate a concurrent opener having sized it first.
        struct stat info;
        if (fstat(fd, &info) != 0) {
            close(fd);
            return nullptr;
        }
        if (static_cast<size_t>(info.st_size) < byteSize) {
            if (ftruncate(fd, static_cast<off_t>(byteSize)) != 0) {
                if (fstat(fd, &info) != 0 || static_cast<size_t>(info.st_size) < byteSize) {
                    close(fd);
                    return nullptr;
                }
            }
        }
    }

    void *mapped = mmap(nullptr, byteSize, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (mapped == MAP_FAILED) {
        close(fd);
        return nullptr;
    }

    auto *ring = new (std::nothrow) NLSharedRing;
    if (!ring) {
        munmap(mapped, byteSize);
        close(fd);
        return nullptr;
    }

    ring->fd = fd;
    ring->byteSize = byteSize;
    ring->header = static_cast<NLSharedRingHeader *>(mapped);
    ring->storage = reinterpret_cast<float *>(static_cast<uint8_t *>(mapped) + sizeof(NLSharedRingHeader));

    // Initialize the ring only when it is uninitialized or its geometry does not
    // match what we expect. Do NOT wipe on every `create` open: a producer
    // reconnecting must not clobber a ring that a consumer is actively reading.
    // Callers that want a clean start should use `nl_shared_ring_clear`.
    if (ring->header->magic != NLSharedRingMagic || ring->header->frameLength != frame_length || ring->header->capacity != frame_capacity + 1) {
        std::memset(mapped, 0, byteSize);
        ring->header->magic = NLSharedRingMagic;
        ring->header->capacity = frame_capacity + 1;
        ring->header->frameLength = frame_length;
    }

    return ring;
}

void nl_shared_ring_close(NLSharedRing *ring) {
    if (!ring) {
        return;
    }
    munmap(ring->header, ring->byteSize);
    close(ring->fd);
    delete ring;
}

bool nl_shared_ring_write(NLSharedRing *ring, const float *samples, uint32_t frame_length) {
    if (!ring || !samples || frame_length != ring->header->frameLength) {
        return false;
    }

    const uint32_t write = __atomic_load_n(&ring->header->writeIndex, __ATOMIC_RELAXED);
    const uint32_t next = shared_next_index(ring->header, write);
    const uint32_t read = __atomic_load_n(&ring->header->readIndex, __ATOMIC_ACQUIRE);
    if (next == read) {
        __atomic_add_fetch(&ring->header->overrunCount, 1, __ATOMIC_RELAXED);
        return false;
    }

    std::memcpy(&ring->storage[static_cast<size_t>(write) * ring->header->frameLength], samples, sizeof(float) * ring->header->frameLength);
    __atomic_store_n(&ring->header->writeIndex, next, __ATOMIC_RELEASE);
    return true;
}

bool nl_shared_ring_write_latest(NLSharedRing *ring, const float *samples, uint32_t frame_length) {
    if (!ring || !samples || frame_length != ring->header->frameLength) {
        return false;
    }

    const uint32_t write = __atomic_load_n(&ring->header->writeIndex, __ATOMIC_RELAXED);
    const uint32_t next = shared_next_index(ring->header, write);
    const uint32_t read = __atomic_load_n(&ring->header->readIndex, __ATOMIC_ACQUIRE);
    if (next == read) {
        __atomic_store_n(&ring->header->readIndex, shared_next_index(ring->header, read), __ATOMIC_RELEASE);
        __atomic_add_fetch(&ring->header->overrunCount, 1, __ATOMIC_RELAXED);
    }

    std::memcpy(&ring->storage[static_cast<size_t>(write) * ring->header->frameLength], samples, sizeof(float) * ring->header->frameLength);
    __atomic_store_n(&ring->header->writeIndex, next, __ATOMIC_RELEASE);
    return true;
}

bool nl_shared_ring_read(NLSharedRing *ring, float *samples, uint32_t frame_length) {
    if (!ring || !samples || frame_length != ring->header->frameLength) {
        return false;
    }

    const uint32_t read = __atomic_load_n(&ring->header->readIndex, __ATOMIC_RELAXED);
    const uint32_t write = __atomic_load_n(&ring->header->writeIndex, __ATOMIC_ACQUIRE);
    if (read == write) {
        std::fill(samples, samples + frame_length, 0.0f);
        __atomic_add_fetch(&ring->header->underrunCount, 1, __ATOMIC_RELAXED);
        return false;
    }

    std::memcpy(samples, &ring->storage[static_cast<size_t>(read) * ring->header->frameLength], sizeof(float) * ring->header->frameLength);
    __atomic_store_n(&ring->header->readIndex, shared_next_index(ring->header, read), __ATOMIC_RELEASE);
    return true;
}

bool nl_shared_ring_read_latest(NLSharedRing *ring, float *samples, uint32_t frame_length, uint32_t max_buffered_frames) {
    if (!ring || !samples || frame_length != ring->header->frameLength) {
        return false;
    }

    if (max_buffered_frames > 0) {
        const uint32_t read = __atomic_load_n(&ring->header->readIndex, __ATOMIC_RELAXED);
        const uint32_t write = __atomic_load_n(&ring->header->writeIndex, __ATOMIC_ACQUIRE);
        const uint32_t available = write >= read ? write - read : ring->header->capacity - read + write;
        if (available > max_buffered_frames) {
            const uint32_t dropped = available - max_buffered_frames;
            const uint32_t trimmedRead = (read + dropped) % ring->header->capacity;
            __atomic_store_n(&ring->header->readIndex, trimmedRead, __ATOMIC_RELEASE);
            __atomic_add_fetch(&ring->header->overrunCount, dropped, __ATOMIC_RELAXED);
        }
    }

    return nl_shared_ring_read(ring, samples, frame_length);
}

void nl_shared_ring_clear(NLSharedRing *ring) {
    if (!ring) {
        return;
    }
    __atomic_store_n(&ring->header->readIndex, 0, __ATOMIC_RELEASE);
    __atomic_store_n(&ring->header->writeIndex, 0, __ATOMIC_RELEASE);
}

void nl_shared_ring_set_producer_attached(NLSharedRing *ring, bool attached) {
    if (!ring) {
        return;
    }
    __atomic_store_n(&ring->header->producerAttached, attached ? 1u : 0u, __ATOMIC_RELEASE);
}

void nl_shared_ring_set_consumer_running_count(NLSharedRing *ring, uint32_t count) {
    if (!ring) {
        return;
    }
    __atomic_store_n(&ring->header->consumerRunningCount, count, __ATOMIC_RELEASE);
}

NLSharedRingHealth nl_shared_ring_health(const NLSharedRing *ring) {
    NLSharedRingHealth health{};
    if (!ring) {
        return health;
    }

    health.available_read_frames = shared_available_read(ring->header);
    health.available_write_frames = shared_available_write(ring->header);
    health.underrun_count = __atomic_load_n(&ring->header->underrunCount, __ATOMIC_ACQUIRE);
    health.overrun_count = __atomic_load_n(&ring->header->overrunCount, __ATOMIC_ACQUIRE);
    health.producer_attached = __atomic_load_n(&ring->header->producerAttached, __ATOMIC_ACQUIRE) != 0;
    health.consumer_running_count = __atomic_load_n(&ring->header->consumerRunningCount, __ATOMIC_ACQUIRE);
    return health;
}

const char *nl_shared_ring_default_name(void) {
    return "/nlvmic";
}
