#include "noiseless_core.h"
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
int main(void) {
    NoiseProcessor *p = nlr_noise_processor_create(1, 48000, 480);
    if (!p) { puts("create failed"); return 1; }
    printf("rnnoise_available=%d uses_neural=%d\n", nlr_rnnoise_available(), nlr_noise_processor_uses_neural(p));
    float in[480], out[480];
    unsigned s = 1;
    const int frames = 48000; // 8 minutes of audio
    struct timespec a, b; clock_gettime(CLOCK_MONOTONIC, &a);
    for (int f = 0; f < frames; ++f) {
        for (int i = 0; i < 480; ++i) { s = s * 1664525u + 1013904223u; in[i] = ((float)(s >> 8) / 16777216.0f - 0.5f) * 0.1f; }
        if (!nlr_noise_processor_process(p, in, out, 480)) { puts("process failed"); return 1; }
    }
    clock_gettime(CLOCK_MONOTONIC, &b);
    double secs = (b.tv_sec - a.tv_sec) + (b.tv_nsec - a.tv_nsec) / 1e9;
    printf("processed %.0f s of audio in %.3f s  (%.0fx realtime, %.1f us/frame)\n", frames * 0.01, secs, frames * 0.01 / secs, secs / frames * 1e6);
    printf("in_rms=%.4f out_rms=%.4f\n", nlr_noise_processor_last_input_rms(p), nlr_noise_processor_last_output_rms(p));
    printf("wrong length rejected: %d\n", !nlr_noise_processor_process(p, in, out, 100));
    printf("null safe: %d\n", !nlr_noise_processor_process(NULL, in, out, 480));
    nlr_noise_processor_destroy(p); nlr_noise_processor_destroy(NULL);
    return 0;
}
