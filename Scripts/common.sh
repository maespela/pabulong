# Shared by the packaging scripts. Source it after setting ROOT.
#
# The release version lives in one place: the VERSION file at the repo root.
# Override it for a one-off build with VERSION=x.y.z.

VERSION="${VERSION:-$(tr -d '[:space:]' < "$ROOT/VERSION")}"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]]; then
  echo "ERROR: VERSION must look like 1.2.3 (got '$VERSION')." >&2
  exit 1
fi
# CFBundleVersion must increase with every release macOS sees, so it tracks the
# marketing version unless a build number is given.
BUILD_NUMBER="${BUILD_NUMBER:-$VERSION}"

BUNDLE_ID="${BUNDLE_ID:-com.nextmatterpro.pabulong}"
DRIVER_BUNDLE_ID="${DRIVER_BUNDLE_ID:-com.nextmatterpro.pabulong.driver}"
ENTITLEMENTS="$ROOT/Sources/NoiselessApp/Noiseless.entitlements"

# write_app_info_plist <Pabulong.app>
write_app_info_plist() {
  cat > "$1/Contents/Info.plist" <<PLIST
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
  <string>$BUILD_NUMBER</string>
  <key>LSMinimumSystemVersion</key>
  <string>14.0</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>Pabulong cleans up your microphone and sends the result to the Pabulong Mic that your call app uses.</string>
  <key>NSAudioCaptureUsageDescription</key>
  <string>Pabulong listens to what your speakers play so it can remove that echo from your microphone. It records the other participants only if you turn on call recording. Audio stays on this Mac.</string>
  <key>NSHumanReadableCopyright</key>
  <string>© 2026 Next Matter Pro. MIT License. Includes RNNoise © Xiph.Org Foundation and contributors (BSD license).</string>
</dict>
</plist>
PLIST
}

# write_driver_info_plist <PabulongHALPlugin.driver>
write_driver_info_plist() {
  cat > "$1/Contents/Info.plist" <<PLIST
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
  <string>$BUILD_NUMBER</string>
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
}
