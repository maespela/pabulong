# noiseless-core

Portable real-time voice processing for Pabulong. One implementation for macOS and
Windows: 48 kHz mono, 480-sample (10 ms) frames in, cleaner frames out.

- `NoiseProcessor` — high-pass → RNNoise (feature `rnnoise`) or spectral subtraction → limiter
- `RingBuffer` — lock-free SPSC frame ring
- C ABI (`include/noiseless_core.h`, `nlr_*`) for the Swift app and a Windows host
- No third-party Rust dependencies

Platform I/O (virtual mic driver, system-audio capture, the cross-process shared ring)
is intentionally **not** here — it differs per OS.

## Build and test

    cargo test                       # spectral fallback
    cargo test --features rnnoise    # + RNNoise (uses ../Sources/CRNNoise/lib by default)
    RNNOISE_LIB_DIR=/path cargo build --release --features rnnoise

## Parity with the macOS C++ implementation

    ./parity/run.sh

Builds the original C++ processor, runs it and the Rust port over the same signal in all
three modes, and fails if any output sample differs by more than 1e-4. Current result:
~2e-7 (spectral) and ~1e-9 (RNNoise), i.e. float rounding.

`rnnoise` on Windows needs an MSVC/x86_64 build of RNNoise; the vendored library is
macOS arm64 only.
