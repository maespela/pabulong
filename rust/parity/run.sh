#!/usr/bin/env bash
# Builds the original C++ processor, dumps its output on a fixed signal, and
# checks the Rust core reproduces it (spectral fallback and RNNoise backends).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${NL_PARITY_DIR:-$(mktemp -d)}"
FRAMES="${FRAMES:-600}"   # 6 s: priming, several tone bursts
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
INC="$ROOT/Sources/NoiselessAudioCoreC/include"
SRC="$ROOT/Sources/NoiselessAudioCoreC/NoiselessAudioCoreC.cpp"
RN_INC="$ROOT/Sources/CRNNoise/include"
RN_LIB="$ROOT/Sources/CRNNoise/lib"

echo "== building C++ reference (spectral) =="
clang++ -std=c++17 -O2 -I"$INC" "$ROOT/rust/parity/parity.cpp" "$SRC" -o "$OUT/parity-spectral"
echo "== building C++ reference (RNNoise) =="
clang++ -std=c++17 -O2 -DNL_HAS_RNNOISE -I"$INC" -I"$RN_INC" "$ROOT/rust/parity/parity.cpp" "$SRC" "$RN_LIB/librnnoise.a" -o "$OUT/parity-neural"

for m in 0 1 2; do
  "$OUT/parity-spectral" $m "$FRAMES" "$OUT/spectral-$m.f32" >/dev/null
  "$OUT/parity-neural"   $m "$FRAMES" "$OUT/neural-$m.f32" | grep -q "neural=1" || { echo "C++ reference is not using RNNoise"; exit 1; }
done

cd "$ROOT/rust"
export NL_PARITY_DIR="$OUT"
echo "== Rust vs C++: spectral =="
cargo test --offline --test parity -- --nocapture 2>&1 | grep -E "max \||test result|panicked|error" || true
echo "== Rust vs C++: RNNoise =="
cargo test --offline --features rnnoise --test parity -- --nocapture 2>&1 | grep -E "max \||test result|panicked|error" || true
