#!/usr/bin/env bash
# Fetches and builds Xiph RNNoise, then vendors the static library + public
# header into Sources/CRNNoise/. Package.swift auto-detects the result and
# compiles the neural suppressor into NoiselessAudioCore.
#
# Requires network access plus the autotools toolchain:
#   brew install autoconf automake libtool
#
# After running this, rebuild and repackage:
#   swift build && ./Scripts/package-app.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/Sources/CRNNoise"
REPO="${RNNOISE_REPO:-https://github.com/xiph/rnnoise}"
MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export MACOSX_DEPLOYMENT_TARGET
# autoconf only adds its default `-g -O2` when CFLAGS is *unset*. Because this
# script sets CFLAGS (for the deployment target), the optimisation level must be
# given explicitly — without it RNNoise is built at -O0 and costs ~13% of a core
# instead of ~1-2%. Override with RNNOISE_OPT_CFLAGS if needed.
RNNOISE_OPT_CFLAGS="${RNNOISE_OPT_CFLAGS:--O2}"
export CFLAGS="${CFLAGS:-} $RNNOISE_OPT_CFLAGS -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET"
export CXXFLAGS="${CXXFLAGS:-} -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET"
export LDFLAGS="${LDFLAGS:-} -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET"

for tool in git autoconf automake libtool make cc; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "ERROR: '$tool' is required. Install the autotools chain:" >&2
    echo "  brew install autoconf automake libtool" >&2
    exit 1
  fi
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "Cloning $REPO ..."
git clone --depth 1 "$REPO" "$WORK/rnnoise"
cd "$WORK/rnnoise"

# RNNoise's download_model.sh fetches the trained weights with `wget`, which
# macOS does not ship. Provide a curl-backed `wget` shim so no extra dependency
# is required.
if ! command -v wget >/dev/null 2>&1; then
  SHIM="$WORK/shim"
  mkdir -p "$SHIM"
  cat > "$SHIM/wget" <<'SH'
#!/bin/sh
# minimal wget -> curl shim (handles bare URL and -O <file>)
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -O) out="$2"; shift 2 ;;
    -O*) out="${1#-O}"; shift ;;
    -q|--quiet|-c|--continue) shift ;;
    --) shift ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
if [ -n "$out" ]; then exec curl -fL -o "$out" "$url"; else exec curl -fLO "$url"; fi
SH
  chmod +x "$SHIM/wget"
  export PATH="$SHIM:$PATH"
  echo "wget not found — using a curl-based shim for the model download."
fi

echo "Building RNNoise for macOS $MACOSX_DEPLOYMENT_TARGET (downloads the trained model) ..."
./autogen.sh          # runs download_model.sh to fetch the trained weights
./configure --disable-examples --disable-doc
make -j"$(sysctl -n hw.ncpu 2>/dev/null || echo 4)"

LIB="$(find "$WORK/rnnoise" -name 'librnnoise.a' -print -quit)"
HEADER="$WORK/rnnoise/include/rnnoise.h"
if [[ -z "$LIB" || ! -f "$HEADER" ]]; then
  echo "ERROR: build did not produce librnnoise.a / rnnoise.h" >&2
  exit 1
fi

echo "Vendoring into $DEST ..."
mkdir -p "$DEST/lib" "$DEST/include"
cp "$LIB" "$DEST/lib/librnnoise.a"
cp "$HEADER" "$DEST/include/rnnoise.h"

# Verify the architecture matches this machine so linking will succeed.
echo "Vendored library architectures:"
lipo -archs "$DEST/lib/librnnoise.a" 2>/dev/null || file "$DEST/lib/librnnoise.a"

# SwiftPM caches the evaluated manifest by Package.swift *content* in a shared
# cache outside .build, so it won't re-run the filesystem detection just because
# librnnoise.a now exists. Purge that cache so the next build detects the
# library. (`touch` alone is not enough — the content is unchanged.)
touch "$ROOT/Package.swift"
( cd "$ROOT" && swift package purge-cache >/dev/null 2>&1 ) || true
echo "Cleared SwiftPM manifest cache so the next build detects RNNoise."

cat <<EOF

RNNoise vendored successfully.
Next:
  swift build                 # Package.swift now defines NL_HAS_RNNOISE
  swift run NoiselessCoreChecks
  ./Scripts/package-app.sh    # rebuild the app with the neural backend
EOF
