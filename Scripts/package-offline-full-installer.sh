#!/usr/bin/env bash
# Builds the public Mac installer, Pabulong-<version>-arm64.pkg: the app, the
# Pabulong Mic HAL driver, a welcome page, the license, and an uninstaller.
#
# Local QA build (ad-hoc signed):
#   ./Scripts/package-offline-full-installer.sh
#
# Public release (Developer ID signed, notarized, stapled): see docs/RELEASING.md
#   ./Scripts/release.sh
#
# Environment:
#   SIGN_IDENTITY       codesign identity for the app and driver ("-" = ad-hoc)
#   PKG_SIGN_IDENTITY   "Developer ID Installer: ..." to sign the .pkg
#   HARDENED=1          hardened runtime + secure timestamp (default)
#   NOTARIZE=1          notarize the app and driver, then the .pkg, and staple
#   NOTARY_PROFILE      notarytool keychain profile (xcrun notarytool store-credentials)
#   VERSION             overrides the VERSION file for a one-off build
set -euo pipefail
export COPYFILE_DISABLE=1

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/Scripts/common.sh"

CONFIG="${CONFIG:-release}"
PKG_ID="${PKG_ID:-com.nextmatterpro.pabulong.installer}"
BUILD_ID="${BUILD_ID:-$(date +%Y%m%d-%H%M%S)}"
OUT_DIR="$ROOT/outputs/offline-full/Pabulong-$VERSION-$BUILD_ID"
STAGE="$OUT_DIR/stage"
PAYLOAD="$STAGE/payload"
PKG_SCRIPTS="$STAGE/scripts"
PKG_RESOURCES="$STAGE/resources"
APP="$PAYLOAD/Applications/Pabulong.app"
DRIVER="$PAYLOAD/Library/Audio/Plug-Ins/HAL/PabulongHALPlugin.driver"
INSTALLER_SRC="$ROOT/Scripts/installer"
APP_RESOURCES="$ROOT/Sources/NoiselessApp/Resources"
RUST_CORE_LIB="$ROOT/Sources/CNoiselessCore/lib/libnoiseless_core.a"
PKG="$OUT_DIR/Pabulong-$VERSION-arm64.pkg"

APP_SIGN_IDENTITY="${APP_SIGN_IDENTITY:-${SIGN_IDENTITY:--}}"
PKG_SIGN_IDENTITY="${PKG_SIGN_IDENTITY:-}"
HARDENED="${HARDENED:-1}"
NOTARIZE="${NOTARIZE:-0}"
NOTARY_PROFILE="${NOTARY_PROFILE:-}"

fail() { echo "ERROR: $*" >&2; exit 1; }

for tool in swift pkgbuild productbuild codesign ditto shasum plutil; do
  command -v "$tool" >/dev/null 2>&1 || fail "required tool '$tool' was not found."
done
[[ "$(uname -m)" == "arm64" ]] || fail "build on an Apple silicon Mac; the installer is arm64 only."

# Without the Rust core the app silently loses echo cancellation and the
# background-voice gate, so a release build must not ship without it.
if [[ ! -f "$RUST_CORE_LIB" && "${ALLOW_NO_RUST_CORE:-0}" != "1" ]]; then
  fail "$RUST_CORE_LIB is missing. Run ./Scripts/build-rust-core.sh first."
fi

# Check release credentials before spending time on the build.
if [[ "$NOTARIZE" == "1" ]]; then
  [[ "$APP_SIGN_IDENTITY" == "Developer ID Application:"* ]] \
    || fail "NOTARIZE=1 needs SIGN_IDENTITY=\"Developer ID Application: ...\" (got '$APP_SIGN_IDENTITY')."
  [[ "$PKG_SIGN_IDENTITY" == "Developer ID Installer:"* ]] \
    || fail "NOTARIZE=1 needs PKG_SIGN_IDENTITY=\"Developer ID Installer: ...\"."
  [[ "$HARDENED" == "1" ]] || fail "notarization requires HARDENED=1."
  [[ -n "$NOTARY_PROFILE" ]] || fail "NOTARIZE=1 needs NOTARY_PROFILE=<notarytool keychain profile>."
  xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" >/dev/null 2>&1 \
    || fail "notarytool profile '$NOTARY_PROFILE' is missing or invalid. See docs/RELEASING.md."
fi
for identity in "$APP_SIGN_IDENTITY" "$PKG_SIGN_IDENTITY"; do
  [[ -z "$identity" || "$identity" == "-" ]] && continue
  security find-identity -v | grep -qF "\"$identity\"" \
    || fail "signing identity not found in the keychain: $identity"
done

echo "Building Pabulong $VERSION ($CONFIG) ..."
swift build --product NoiselessApp -c "$CONFIG"
swift build --product NoiselessHelper -c "$CONFIG"
swift build --product NoiselessHALPlugin -c "$CONFIG"
BIN_DIR="$(swift build -c "$CONFIG" --show-bin-path)"

rm -rf "$STAGE"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$DRIVER/Contents/MacOS" \
  "$PKG_SCRIPTS" "$PKG_RESOURCES"

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
cp "$INSTALLER_SRC/uninstall.sh" "$APP/Contents/Resources/uninstall.sh"
chmod 755 "$APP/Contents/Resources/uninstall.sh"
write_app_info_plist "$APP"

cp "$BIN_DIR/libNoiselessHALPlugin.dylib" "$DRIVER/Contents/MacOS/PabulongHALPlugin"
write_driver_info_plist "$DRIVER"

cp "$INSTALLER_SRC/preinstall" "$INSTALLER_SRC/postinstall" "$PKG_SCRIPTS/"
chmod 755 "$PKG_SCRIPTS/preinstall" "$PKG_SCRIPTS/postinstall"

# --- Code signing -------------------------------------------------------------

sign() {
  local path="$1"; shift
  if [[ "$APP_SIGN_IDENTITY" == "-" ]]; then
    codesign --force --sign - "$@" "$path"
  elif [[ "$HARDENED" == "1" ]]; then
    codesign --force --options runtime --timestamp --sign "$APP_SIGN_IDENTITY" "$@" "$path"
  else
    codesign --force --sign "$APP_SIGN_IDENTITY" "$@" "$path"
  fi
}

echo "Signing the app and driver with: $APP_SIGN_IDENTITY"
xattr -cr "$PAYLOAD" 2>/dev/null || true
# Inside out: the embedded helper first, then the app bundle that seals it.
sign "$APP/Contents/Resources/PabulongHelper"
if [[ "$APP_SIGN_IDENTITY" == "-" ]]; then
  sign "$APP"
else
  sign "$APP" --entitlements "$ENTITLEMENTS"
fi
sign "$DRIVER"
codesign --verify --deep --strict "$APP"
codesign --verify --deep --strict "$DRIVER"

# --- Notarize the app and driver ----------------------------------------------
# Notarizing them on their own, before packaging, lets us staple a ticket to each
# bundle, so Gatekeeper accepts them after install even when the Mac is offline.

notarize() {
  local file="$1" label="$2" result status id
  echo "Submitting $label for notarization (this usually takes a few minutes) ..."
  result="$(xcrun notarytool submit "$file" --keychain-profile "$NOTARY_PROFILE" \
    --wait --output-format plist)"
  status="$(plutil -extract status raw -o - - <<<"$result" 2>/dev/null || echo unknown)"
  id="$(plutil -extract id raw -o - - <<<"$result" 2>/dev/null || echo unknown)"
  xcrun notarytool log "$id" --keychain-profile "$NOTARY_PROFILE" \
    "$OUT_DIR/notary-$label.json" >/dev/null 2>&1 || true
  if [[ "$status" != "Accepted" ]]; then
    fail "notarization of $label returned '$status' (submission $id). Log: $OUT_DIR/notary-$label.json"
  fi
  echo "Notarized $label (submission $id)."
}

if [[ "$NOTARIZE" == "1" ]]; then
  BUNDLES_ZIP="$STAGE/Pabulong-bundles.zip"
  ( cd "$PAYLOAD" && ditto -c -k --sequesterRsrc . "$BUNDLES_ZIP" )
  notarize "$BUNDLES_ZIP" bundles
  xcrun stapler staple "$APP"
  # Gatekeeper also finds the driver's ticket online, so a staple failure here
  # is not fatal.
  xcrun stapler staple "$DRIVER" || echo "WARNING: could not staple the driver; its ticket is still online."
  rm -f "$BUNDLES_ZIP"
fi

# --- Package ------------------------------------------------------------------

# pkgbuild marks bundles relocatable by default, which makes Installer "upgrade"
# any other copy of Pabulong.app it finds instead of writing to /Applications.
# It also version-checks them, which silently keeps an installed bundle whose
# CFBundleVersion compares higher (earlier builds used "1"), so always install.
COMPONENT_PLIST="$STAGE/components.plist"
pkgbuild --analyze --root "$PAYLOAD" "$COMPONENT_PLIST" >/dev/null
i=0
while plutil -extract "$i" xml1 -o /dev/null "$COMPONENT_PLIST" >/dev/null 2>&1; do
  plutil -replace "$i.BundleIsRelocatable" -bool NO "$COMPONENT_PLIST"
  plutil -replace "$i.BundleIsVersionChecked" -bool NO "$COMPONENT_PLIST"
  i=$((i + 1))
done

echo "Building the installer package ..."
pkgbuild \
  --root "$PAYLOAD" \
  --component-plist "$COMPONENT_PLIST" \
  --scripts "$PKG_SCRIPTS" \
  --identifier "$PKG_ID" \
  --version "$VERSION" \
  --install-location / \
  --ownership recommended \
  "$STAGE/Pabulong-component.pkg" >/dev/null

sed "s/__VERSION__/$VERSION/g" "$INSTALLER_SRC/welcome.html" > "$PKG_RESOURCES/welcome.html"
cp "$INSTALLER_SRC/conclusion.html" "$PKG_RESOURCES/conclusion.html"
cat "$INSTALLER_SRC/license-preamble.txt" "$ROOT/LICENSE" > "$PKG_RESOURCES/license.txt"
cp "$INSTALLER_SRC/background.png" "$INSTALLER_SRC/background-dark.png" "$PKG_RESOURCES/"

cat > "$STAGE/distribution.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
  <title>Pabulong</title>
  <organization>com.nextmatterpro</organization>
  <welcome file="welcome.html" mime-type="text/html"/>
  <license file="license.txt" mime-type="text/plain"/>
  <conclusion file="conclusion.html" mime-type="text/html"/>
  <background file="background.png" alignment="bottomleft" scaling="none"/>
  <background-darkAqua file="background-dark.png" alignment="bottomleft" scaling="none"/>
  <options customize="never" require-scripts="false" hostArchitectures="arm64"/>
  <domains enable_localSystem="true" enable_currentUserHome="false" enable_anywhere="false"/>
  <volume-check>
    <allowed-os-versions>
      <os-version min="14.0"/>
    </allowed-os-versions>
  </volume-check>
  <choices-outline>
    <line choice="default">
      <line choice="$PKG_ID"/>
    </line>
  </choices-outline>
  <choice id="default"/>
  <choice id="$PKG_ID" visible="false">
    <pkg-ref id="$PKG_ID"/>
  </choice>
  <pkg-ref id="$PKG_ID" version="$VERSION" onConclusion="none">Pabulong-component.pkg</pkg-ref>
</installer-gui-script>
XML

PRODUCT_ARGS=(
  --distribution "$STAGE/distribution.xml"
  --resources "$PKG_RESOURCES"
  --package-path "$STAGE"
)
if [[ -n "$PKG_SIGN_IDENTITY" ]]; then
  PRODUCT_ARGS+=(--sign "$PKG_SIGN_IDENTITY" --timestamp)
fi
productbuild "${PRODUCT_ARGS[@]}" "$PKG"

if [[ "$NOTARIZE" == "1" ]]; then
  notarize "$PKG" pkg
  xcrun stapler staple "$PKG"
  xcrun stapler validate "$PKG"
fi

# --- Checksums and release facts ----------------------------------------------

PKG_NAME="$(basename "$PKG")"
( cd "$OUT_DIR" && shasum -a 256 "$PKG_NAME" > "$PKG_NAME.sha256" )
SHA256="$(cut -d' ' -f1 < "$PKG.sha256")"
SIZE_BYTES="$(stat -f %z "$PKG")"
SIZE_MB="$(awk -v b="$SIZE_BYTES" 'BEGIN { printf "%.1f MB", b / 1000000 }')"
if [[ "$NOTARIZE" == "1" ]]; then DISTRIBUTABLE=true; else DISTRIBUTABLE=false; fi

cat > "$OUT_DIR/release.json" <<JSON
{
  "version": "$VERSION",
  "file": "$PKG_NAME",
  "bytes": $SIZE_BYTES,
  "size": "$SIZE_MB",
  "sha256": "$SHA256",
  "appIdentity": "$APP_SIGN_IDENTITY",
  "pkgIdentity": "${PKG_SIGN_IDENTITY:-unsigned}",
  "notarized": $DISTRIBUTABLE,
  "built": "$BUILD_ID"
}
JSON

# The signed bundles are sealed inside the package. Keeping the staging copy
# makes Spotlight list it as another installed Pabulong.
if [[ "${KEEP_STAGING:-0}" != "1" ]]; then
  rm -rf "$STAGE"
fi

echo
if [[ "$NOTARIZE" == "1" ]]; then
  "$ROOT/Scripts/verify-installer.sh" "$PKG"
else
  # A QA build is expected to fail the Developer ID and Gatekeeper checks.
  "$ROOT/Scripts/verify-installer.sh" "$PKG" || true
fi
echo
echo "Installer: $PKG"
echo "Size:      $SIZE_MB ($SIZE_BYTES bytes)"
echo "SHA-256:   $SHA256"
if [[ "$DISTRIBUTABLE" != "true" ]]; then
  echo
  echo "NOTE: not Developer ID signed and notarized. Use it for local QA only."
fi
