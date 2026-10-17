#!/usr/bin/env bash
# Builds the shared Rust DSP core (rust/noiseless-core: echo canceller, RNNoise,
# background-voice gate) — the same code the Windows app uses — and vendors the
# static library + header for SwiftPM. Package.swift auto-detects the result.
#
# Needs the Rust toolchain and an RNNoise build (Scripts/vendor-rnnoise.sh).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export RNNOISE_LIB_DIR="${RNNOISE_LIB_DIR:-$ROOT/Sources/CRNNoise/lib}"
if [[ ! -f "$RNNOISE_LIB_DIR/librnnoise.a" ]]; then
  echo "ERROR: $RNNOISE_LIB_DIR/librnnoise.a missing — run Scripts/vendor-rnnoise.sh first." >&2
  exit 1
fi
command -v cargo >/dev/null || { echo "ERROR: cargo not found (install Rust: https://rustup.rs)" >&2; exit 1; }

( cd "$ROOT/rust" && cargo build --release --offline -p noiseless-core --features rnnoise )

DEST="$ROOT/Sources/CNoiselessCore/lib"
mkdir -p "$DEST"
cp "$ROOT/rust/target/release/libnoiseless_core.a" "$DEST/libnoiseless_core.a"
cp "$ROOT/rust/noiseless-core/include/noiseless_core.h" "$ROOT/Sources/NoiselessAudioCoreC/include/noiseless_core.h"
echo "Vendored $(du -h "$DEST/libnoiseless_core.a" | cut -f1) static library into $DEST"
# SwiftPM caches the evaluated manifest; make it re-run the file detection.
touch "$ROOT/Package.swift"
