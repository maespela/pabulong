// Runs the ORIGINAL C++ noise processor over a deterministic signal and dumps
// raw float32 output, so the Rust port can be compared sample-for-sample.
// usage: parity <mode 0|1|2> <frames> <out.f32>
#include "NoiselessAudioCoreC.h"

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <vector>

static uint32_t lcg_state = 12345;
static float lcg_noise(float amp) {
    lcg_state = lcg_state * 1664525u + 1013904223u;
    return (static_cast<float>(lcg_state >> 8) / 16777216.0f - 0.5f) * 2.0f * amp;
}

// Tone bursts over a noise floor: exercises priming, speech detect, and gating.
static float signal_at(uint64_t t) {
    const double seconds = static_cast<double>(t) / 48000.0;
    const bool burst = std::fmod(seconds, 1.0) > 0.5;
    float s = lcg_noise(0.03f);
    if (burst) {
        s += static_cast<float>(0.3 * std::sin(2.0 * M_PI * 440.0 * static_cast<double>(t) / 48000.0));
    }
    return s;
}

int main(int argc, char **argv) {
    if (argc < 4) return 2;
    const NLNoiseMode mode = static_cast<NLNoiseMode>(std::atoi(argv[1]));
    const int frames = std::atoi(argv[2]);
    NLNoiseProcessor *p = nl_noise_processor_create(mode, 48000, 480);
    if (!p) return 3;
    std::printf("neural=%d\n", nl_noise_processor_uses_neural(p) ? 1 : 0);
    FILE *out = std::fopen(argv[3], "wb");
    std::vector<float> in(480), o(480);
    for (int f = 0; f < frames; ++f) {
        for (int i = 0; i < 480; ++i) in[i] = signal_at(static_cast<uint64_t>(f) * 480 + i);
        nl_noise_processor_process(p, in.data(), o.data(), 480);
        std::fwrite(o.data(), sizeof(float), 480, out);
    }
    std::fclose(out);
    nl_noise_processor_destroy(p);
    return 0;
}
