#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="${CONFIG:-debug}"
TRIPLE="$(swift build --show-bin-path | sed 's#^\(.*\)/debug$#\1#')"
BIN_DIR="$ROOT/.build/arm64-apple-macosx/$CONFIG"
if [[ ! -d "$BIN_DIR" ]]; then
  BIN_DIR="$ROOT/.build/$CONFIG"
fi

swift build --product NoiselessHALPlugin -c "$CONFIG"

OUT="$ROOT/outputs/PabulongHALPlugin.driver"
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS"

cp "$BIN_DIR/libNoiselessHALPlugin.dylib" "$OUT/Contents/MacOS/PabulongHALPlugin"
cat > "$OUT/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>English</string>
  <key>CFBundleExecutable</key>
  <string>PabulongHALPlugin</string>
  <key>CFBundleIdentifier</key>
  <string>com.nextmatterpro.pabulong.driver</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Pabulong Mic</string>
  <key>NSHumanReadableCopyright</key>
  <string>© 2026 Next Matter Pro. MIT License.</string>
  <key>CFBundlePackageType</key>
  <string>BNDL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleSignature</key>
  <string>????</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>CFPlugInFactories</key>
  <dict>
    <key>4F73716D-1E45-4E9D-A6BA-99AC8420502F</key>
    <string>Pabulong_Create</string>
  </dict>
  <key>CFPlugInTypes</key>
  <dict>
    <key>443ABAB8-E7B3-491A-B985-BEB9187030DB</key>
    <array>
      <string>4F73716D-1E45-4E9D-A6BA-99AC8420502F</string>
    </array>
  </dict>
</dict>
</plist>
PLIST

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
