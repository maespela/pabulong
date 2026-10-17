#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="${CONFIG:-debug}"
BIN_DIR="$ROOT/.build/arm64-apple-macosx/$CONFIG"
if [[ ! -d "$BIN_DIR" ]]; then
  BIN_DIR="$ROOT/.build/$CONFIG"
fi

swift build --product NoiselessApp -c "$CONFIG"
swift build --product NoiselessHelper -c "$CONFIG"

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
# Third-party licenses (RNNoise BSD-3-Clause requires its notice to ship with binaries).
cp "$ROOT/THIRD_PARTY_NOTICES.md" "$OUT/Contents/Resources/ThirdPartyNotices.md"
cp "$ROOT/LICENSE" "$OUT/Contents/Resources/LICENSE.txt"

cat > "$OUT/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIconFile</key>
  <string>Pabulong</string>
  <key>CFBundleDevelopmentRegion</key>
  <string>English</string>
  <key>CFBundleExecutable</key>
  <string>Pabulong</string>
  <key>CFBundleIdentifier</key>
  <string>com.nextmatterpro.pabulong</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Pabulong</string>
  <key>CFBundleDisplayName</key>
  <string>Pabulong</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSMinimumSystemVersion</key>
  <string>14.0</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSMicrophoneUsageDescription</key>
  <string>Pabulong cleans up your microphone and sends the result to the Pabulong Mic that your call app uses.</string>
  <key>NSAudioCaptureUsageDescription</key>
  <string>Pabulong listens to what your speakers play so it can remove that echo from your microphone. It records the other participants only if you turn on call recording. Audio stays on this Mac.</string>
  <key>NSHumanReadableCopyright</key>
  <string>© 2026 Next Matter Pro. MIT License. Includes RNNoise © Xiph.Org Foundation and contributors (BSD license).</string>
</dict>
</plist>
PLIST

# Signing.
#   - Local/dev (default): ad-hoc signature. Unstable identity across rebuilds,
#     so macOS re-prompts for microphone access every build.
#   - Stable local signing: set SIGN_IDENTITY to an "Apple Development: ..." or
#     self-signed identity. The signature is stable, so the mic grant persists.
#   - Distribution: set SIGN_IDENTITY to "Developer ID Application: ..." and
#     HARDENED=1 to sign with a hardened runtime, ready for notarization.
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
ENTITLEMENTS="$ROOT/Sources/NoiselessApp/Noiseless.entitlements"
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
