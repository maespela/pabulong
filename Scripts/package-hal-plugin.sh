#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/Scripts/common.sh"
CONFIG="${CONFIG:-debug}"
BIN_DIR="$ROOT/.build/arm64-apple-macosx/$CONFIG"
if [[ ! -d "$BIN_DIR" ]]; then
  BIN_DIR="$ROOT/.build/$CONFIG"
fi

swift build --product NoiselessHALPlugin -c "$CONFIG"

OUT="$ROOT/outputs/PabulongHALPlugin.driver"
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS"

cp "$BIN_DIR/libNoiselessHALPlugin.dylib" "$OUT/Contents/MacOS/PabulongHALPlugin"
write_driver_info_plist "$OUT"

# Signing. Set SIGN_IDENTITY to a "Developer ID Application: ..." name for
# distribution (hardened runtime + secure timestamp); defaults to ad-hoc.
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
if [[ "$SIGN_IDENTITY" == "-" ]]; then
  codesign --force --sign - "$OUT" >/dev/null
else
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$OUT" >/dev/null
  echo "Signed with: $SIGN_IDENTITY"
fi
echo "$OUT"
