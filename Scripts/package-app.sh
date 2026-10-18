#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/Scripts/common.sh"
CONFIG="${CONFIG:-debug}"
BIN_DIR="$ROOT/.build/arm64-apple-macosx/$CONFIG"
if [[ ! -d "$BIN_DIR" ]]; then
  BIN_DIR="$ROOT/.build/$CONFIG"
fi

# Package.swift links the Rust core only if its library exists, so never reuse
# a manifest evaluated before ./Scripts/build-rust-core.sh ran.
swift build --manifest-cache none --product NoiselessApp -c "$CONFIG"
swift build --manifest-cache none --product NoiselessHelper -c "$CONFIG"

OUT="$ROOT/outputs/Pabulong.app"
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"

cp "$BIN_DIR/NoiselessApp" "$OUT/Contents/MacOS/Pabulong"
cp "$BIN_DIR/NoiselessHelper" "$OUT/Contents/Resources/PabulongHelper"

ICON="$ROOT/Sources/NoiselessApp/Resources/Pabulong.icns"
if [[ ! -f "$ICON" ]]; then
  swift "$ROOT/Scripts/generate-app-icon.swift" >/dev/null
fi
cp "$ICON" "$OUT/Contents/Resources/Pabulong.icns"
# Brand fonts (SIL OFL 1.1, notices in THIRD_PARTY_NOTICES.md), registered at launch.
mkdir -p "$OUT/Contents/Resources/Fonts"
cp "$ROOT/Sources/NoiselessApp/Resources/Fonts/"*.ttf "$OUT/Contents/Resources/Fonts/"
# Third-party licenses (RNNoise BSD-3-Clause requires its notice to ship with binaries).
cp "$ROOT/THIRD_PARTY_NOTICES.md" "$OUT/Contents/Resources/ThirdPartyNotices.md"
cp "$ROOT/LICENSE" "$OUT/Contents/Resources/LICENSE.txt"

write_app_info_plist "$OUT"
embed_sparkle "$OUT"

# Signing.
#   - Local/dev (default): ad-hoc signature. Unstable identity across rebuilds,
#     so macOS re-prompts for microphone access every build.
#   - Stable local signing: set SIGN_IDENTITY to an "Apple Development: ..." or
#     self-signed identity. The signature is stable, so the mic grant persists.
#   - Distribution: set SIGN_IDENTITY to "Developer ID Application: ..." and
#     HARDENED=1 to sign with a hardened runtime, ready for notarization.
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
sign_sparkle "$SIGN_IDENTITY" "${HARDENED:-0}" "$OUT" >/dev/null
if [[ "$SIGN_IDENTITY" == "-" ]]; then
  codesign --force --sign - "$OUT/Contents/Resources/PabulongHelper" >/dev/null
  codesign --force --sign - "$OUT" >/dev/null
  echo "Ad-hoc signed (mic permission will re-prompt on each rebuild)."
else
  # Sign the embedded helper first, then the app bundle with its entitlements.
  # (macOS bash 3.2 is unfriendly to empty arrays under `set -u`, so branch.)
  if [[ "${HARDENED:-0}" == "1" ]]; then
    codesign --force --options runtime --timestamp \
      --sign "$SIGN_IDENTITY" "$OUT/Contents/Resources/PabulongHelper" >/dev/null
    codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" \
      --sign "$SIGN_IDENTITY" "$OUT" >/dev/null
  else
    codesign --force \
      --sign "$SIGN_IDENTITY" "$OUT/Contents/Resources/PabulongHelper" >/dev/null
    codesign --force --entitlements "$ENTITLEMENTS" \
      --sign "$SIGN_IDENTITY" "$OUT" >/dev/null
  fi
  echo "Signed with: $SIGN_IDENTITY"
  if [[ "${HARDENED:-0}" == "1" ]]; then
    echo "Notarize with: xcrun notarytool submit <zip> --keychain-profile <profile> --wait"
    echo "Then staple with: xcrun stapler staple \"$OUT\""
  fi
fi
echo "$OUT"
