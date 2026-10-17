#!/usr/bin/env bash
set -euo pipefail
export COPYFILE_DISABLE=1

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="${CONFIG:-release}"
VERSION="${VERSION:-0.1.0}"
BUNDLE_ID="${BUNDLE_ID:-com.nextmatterpro.pabulong}"
DRIVER_BUNDLE_ID="${DRIVER_BUNDLE_ID:-com.nextmatterpro.pabulong.driver}"
PKG_ID="${PKG_ID:-com.nextmatterpro.pabulong.installer}"
BUILD_ID="${BUILD_ID:-$(date +%Y%m%d-%H%M%S)}"
OUT_DIR="$ROOT/outputs/offline-full/Pabulong-$VERSION-$BUILD_ID"
PAYLOAD="$OUT_DIR/payload"
PKG_SCRIPTS="$OUT_DIR/scripts"
APP="$PAYLOAD/Applications/Pabulong.app"
DRIVER="$PAYLOAD/Library/Audio/Plug-Ins/HAL/PabulongHALPlugin.driver"
ENTITLEMENTS="$ROOT/Sources/NoiselessApp/Noiseless.entitlements"
APP_RESOURCES="$ROOT/Sources/NoiselessApp/Resources"

APP_SIGN_IDENTITY="${APP_SIGN_IDENTITY:-${SIGN_IDENTITY:--}}"
PKG_SIGN_IDENTITY="${PKG_SIGN_IDENTITY:-}"
HARDENED="${HARDENED:-1}"
NOTARIZE="${NOTARIZE:-0}"
NOTARY_PROFILE="${NOTARY_PROFILE:-}"

for tool in swift pkgbuild codesign ditto shasum; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "ERROR: required tool '$tool' was not found." >&2
    exit 1
  fi
done

echo "Building Pabulong products ($CONFIG) without touching outputs/Pabulong.app ..."
swift build --product NoiselessApp -c "$CONFIG"
swift build --product NoiselessHelper -c "$CONFIG"
swift build --product NoiselessHALPlugin -c "$CONFIG"
BIN_DIR="$(swift build -c "$CONFIG" --show-bin-path)"

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$DRIVER/Contents/MacOS" "$PKG_SCRIPTS"

cp "$BIN_DIR/NoiselessApp" "$APP/Contents/MacOS/Pabulong"
cp "$BIN_DIR/NoiselessHelper" "$APP/Contents/Resources/PabulongHelper"

ICON="$APP_RESOURCES/Pabulong.icns"
if [[ ! -f "$ICON" ]]; then
  swift "$ROOT/Scripts/generate-app-icon.swift" >/dev/null
fi
cp "$ICON" "$APP/Contents/Resources/Pabulong.icns"
# Third-party licenses: RNNoise (BSD-3-Clause) requires its full notice with binaries.
cp "$ROOT/THIRD_PARTY_NOTICES.md" "$APP/Contents/Resources/ThirdPartyNotices.md"
cp "$ROOT/LICENSE" "$APP/Contents/Resources/LICENSE.txt"

cat > "$APP/Contents/Info.plist" <<PLIST
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
  <string>$BUNDLE_ID</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Pabulong</string>
  <key>CFBundleDisplayName</key>
  <string>Pabulong</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
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

cp "$BIN_DIR/libNoiselessHALPlugin.dylib" "$DRIVER/Contents/MacOS/PabulongHALPlugin"
cat > "$DRIVER/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>English</string>
  <key>CFBundleExecutable</key>
  <string>PabulongHALPlugin</string>
  <key>CFBundleIdentifier</key>
  <string>$DRIVER_BUNDLE_ID</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Pabulong Mic</string>
  <key>NSHumanReadableCopyright</key>
  <string>© 2026 Next Matter Pro. MIT License.</string>
  <key>CFBundlePackageType</key>
  <string>BNDL</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
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


cat > "$PKG_SCRIPTS/postinstall" <<'POSTINSTALL'
#!/bin/sh
set -e

# Remove the driver from builds named "Noiseless" so only one virtual mic exists.
/bin/rm -rf "/Library/Audio/Plug-Ins/HAL/NoiselessHALPlugin.driver"

# Reload CoreAudio so the newly installed HAL plug-in appears as "Pabulong Mic".
/usr/bin/killall coreaudiod >/dev/null 2>&1 || true

exit 0
POSTINSTALL
chmod 755 "$PKG_SCRIPTS/postinstall"

sign_item() {
  local path="$1"
  if [[ "$APP_SIGN_IDENTITY" == "-" ]]; then
    codesign --force --sign - "$path" >/dev/null
  elif [[ "$HARDENED" == "1" ]]; then
    codesign --force --options runtime --timestamp --sign "$APP_SIGN_IDENTITY" "$path" >/dev/null
  else
    codesign --force --sign "$APP_SIGN_IDENTITY" "$path" >/dev/null
  fi
}

sign_app_bundle() {
  sign_item "$APP/Contents/Resources/PabulongHelper"

  if [[ "$APP_SIGN_IDENTITY" == "-" ]]; then
    codesign --force --sign - "$APP" >/dev/null
  elif [[ "$HARDENED" == "1" ]]; then
    codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" --sign "$APP_SIGN_IDENTITY" "$APP" >/dev/null
  else
    codesign --force --entitlements "$ENTITLEMENTS" --sign "$APP_SIGN_IDENTITY" "$APP" >/dev/null
  fi
}

echo "Signing copied app and HAL driver ..."
sign_app_bundle
sign_item "$DRIVER"

xattr -cr "$PAYLOAD" 2>/dev/null || true
codesign --verify --deep --strict "$APP"
codesign --verify --strict "$DRIVER"

PKG="$OUT_DIR/Pabulong-$VERSION-$(uname -m).pkg"
PKG_ARGS=(
  --root "$PAYLOAD"
  --scripts "$PKG_SCRIPTS"
  --identifier "$PKG_ID"
  --version "$VERSION"
  --ownership recommended
)

if [[ -n "$PKG_SIGN_IDENTITY" ]]; then
  PKG_ARGS+=(--sign "$PKG_SIGN_IDENTITY")
fi

echo "Building offline full installer ..."
pkgbuild "${PKG_ARGS[@]}" "$PKG"
shasum -a 256 "$PKG" > "$PKG.sha256"

if [[ "$NOTARIZE" == "1" ]]; then
  if [[ -z "$NOTARY_PROFILE" ]]; then
    echo "ERROR: NOTARIZE=1 requires NOTARY_PROFILE=<notarytool keychain profile>." >&2
    exit 1
  fi
  xcrun notarytool submit "$PKG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$PKG"
fi

cat > "$OUT_DIR/README.txt" <<EOF
Pabulong Installer
Version: $VERSION
Architecture: $(uname -m)
Installer:
$PKG

This package installs:
- /Applications/Pabulong.app
- /Library/Audio/Plug-Ins/HAL/PabulongHALPlugin.driver

For public website distribution, build with:
  HARDENED=1 \\
  SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \\
  PKG_SIGN_IDENTITY="Developer ID Installer: Your Name (TEAMID)" \\
  NOTARIZE=1 NOTARY_PROFILE=<profile> \\
  ./Scripts/package-offline-full-installer.sh

Without Developer ID Application and Developer ID Installer certificates, this
package is useful for local QA but is not a Gatekeeper-ready public download.
EOF

# The signed app and driver are already sealed inside the installer. Keeping the
# extracted staging app makes Spotlight present it as another installed copy.
if [[ "${KEEP_STAGING:-0}" != "1" ]]; then
  rm -rf "$PAYLOAD" "$PKG_SCRIPTS"
fi

echo
echo "Offline full installer created:"
echo "$PKG"
echo
echo "Checksum:"
cat "$PKG.sha256"
echo
if [[ -z "$PKG_SIGN_IDENTITY" || "$APP_SIGN_IDENTITY" == "-" ]]; then
  echo "NOTE: This build is not Developer ID signed/notarized. Do not publish it as the final website download yet."
fi
