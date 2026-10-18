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

# In-app updates (Sparkle 2). The private half of SPARKLE_PUBLIC_ED_KEY is in the
# release Mac's login keychain under the account below; see docs/RELEASING.md.
UPDATE_FEED_URL="${UPDATE_FEED_URL:-https://vastarterhub.com/downloads/pabulong/appcast.xml}"
UPDATE_DOWNLOAD_BASE="${UPDATE_DOWNLOAD_BASE:-https://vastarterhub.com/downloads/pabulong}"
SPARKLE_PUBLIC_ED_KEY="1wHGLZ3n8UcZQ42SKogDMabWBKTIRZZg9k6yTj/aYy8="
SPARKLE_KEY_ACCOUNT="${SPARKLE_KEY_ACCOUNT:-pabulong}"
# SwiftPM downloads Sparkle (pinned in Package.swift) with its command-line tools.
SPARKLE_ARTIFACTS="$ROOT/.build/artifacts/sparkle/Sparkle"
SPARKLE_FRAMEWORK="$SPARKLE_ARTIFACTS/Sparkle.xcframework/macos-arm64_x86_64/Sparkle.framework"
SPARKLE_BIN="$SPARKLE_ARTIFACTS/bin"

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
  <key>SUFeedURL</key>
  <string>$UPDATE_FEED_URL</string>
  <key>SUPublicEDKey</key>
  <string>$SPARKLE_PUBLIC_ED_KEY</string>
  <key>SURequireSignedFeed</key>
  <true/>
  <key>SUVerifyUpdateBeforeExtraction</key>
  <true/>
  <key>SUEnableSystemProfiling</key>
  <false/>
  <key>SUSendProfileInfo</key>
  <false/>
  <key>SUAllowsAutomaticUpdates</key>
  <false/>
</dict>
</plist>
PLIST
}

# SUEnableAutomaticChecks is left out on purpose: Sparkle then asks on the second
# launch, and checks nothing until the user agrees. SUAllowsAutomaticUpdates is
# off because the pkg needs an admin password, so every install is a click.

# embed_sparkle <Pabulong.app>
# Copies Sparkle.framework (with its XPC services, Autoupdate and Updater.app)
# into Contents/Frameworks, and drops the build folder from the app's rpaths so
# the embedded copy is the one that loads.
embed_sparkle() {
  local app="$1" exe="$1/Contents/MacOS/Pabulong" rpath
  if [[ ! -d "$SPARKLE_FRAMEWORK" ]]; then
    echo "ERROR: $SPARKLE_FRAMEWORK is missing. Run: swift package resolve" >&2
    exit 1
  fi
  mkdir -p "$app/Contents/Frameworks"
  rm -rf "$app/Contents/Frameworks/Sparkle.framework"
  ditto "$SPARKLE_FRAMEWORK" "$app/Contents/Frameworks/Sparkle.framework"
  for rpath in $(otool -l "$exe" | awk '$1 == "cmd" && $2 == "LC_RPATH" { getline; getline; print $2 }'); do
    case "$rpath" in
      "$ROOT"/*) install_name_tool -delete_rpath "$rpath" "$exe" ;;
    esac
  done
}

# codesign_as <identity> <hardened 0|1> <path> [codesign options]
codesign_as() {
  local identity="$1" hardened="$2" path="$3"
  shift 3
  if [[ "$identity" == "-" ]]; then
    codesign --force --sign - "$@" "$path"
  elif [[ "$hardened" == "1" ]]; then
    codesign --force --options runtime --timestamp --sign "$identity" "$@" "$path"
  else
    codesign --force --sign "$identity" "$@" "$path"
  fi
}

# sign_sparkle <identity> <hardened 0|1> <Pabulong.app>
# Re-signs the embedded Sparkle with the app's identity, inside out, as
# Sparkle's documentation describes. The Downloader keeps its entitlements.
sign_sparkle() {
  local framework="$3/Contents/Frameworks/Sparkle.framework"
  local b="$framework/Versions/B"
  codesign_as "$1" "$2" "$b/XPCServices/Installer.xpc"
  codesign_as "$1" "$2" "$b/XPCServices/Downloader.xpc" --preserve-metadata=entitlements
  codesign_as "$1" "$2" "$b/Autoupdate"
  codesign_as "$1" "$2" "$b/Updater.app"
  codesign_as "$1" "$2" "$framework"
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
