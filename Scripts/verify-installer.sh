#!/usr/bin/env bash
# Checks a built Pabulong installer without installing it: package signature,
# Gatekeeper verdicts, notarization tickets, and the signatures, entitlements,
# versions and dependencies of the app and driver inside.
#
#   ./Scripts/verify-installer.sh outputs/offline-full/.../Pabulong-0.1.0-arm64.pkg
#
# Exits non-zero if any release requirement fails. A dev-signed build is
# expected to fail the Developer ID, Gatekeeper and notarization checks.
set -uo pipefail

PKG="${1:?usage: verify-installer.sh <Pabulong.pkg>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXPECTED_VERSION="$(tr -d '[:space:]' < "$ROOT/VERSION")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0
pass() { printf '  PASS  %s\n' "$1"; }
fail() { printf '  FAIL  %s\n' "$1"; [[ -n "${2:-}" ]] && printf '        %s\n' "$2"; failures=$((failures + 1)); }
check() { local label="$1"; shift; local out; if out="$("$@" 2>&1)"; then pass "$label"; else fail "$label" "$(echo "$out" | head -3 | tr '\n' ' ')"; fi; }

echo "Verifying $(basename "$PKG")"

echo "Package"
sig="$(pkgutil --check-signature "$PKG" 2>&1)"
if grep -q "Developer ID Installer:" <<<"$sig"; then
  pass "signed with Developer ID Installer"
else
  fail "signed with Developer ID Installer" "$(echo "$sig" | sed -n 2p | sed 's/^ *//')"
fi
check "Gatekeeper accepts the package (spctl -t install)" spctl -a -vv -t install "$PKG"
check "notarization ticket stapled to the package" xcrun stapler validate -q "$PKG"

pkgutil --expand-full "$PKG" "$WORK/x" >/dev/null 2>&1 || { fail "package expands"; exit 1; }
COMPONENT="$(find "$WORK/x" -maxdepth 1 -name '*.pkg' | head -1)"
DIST="$WORK/x/Distribution"
grep -q 'hostArchitectures="arm64"' "$DIST" && pass "installs only on Apple silicon" || fail "installs only on Apple silicon"
grep -q '<license ' "$DIST" && grep -q '<welcome ' "$DIST" && pass "shows welcome and license" || fail "shows welcome and license"
# An empty <relocate/> means no bundle can be redirected to another copy on disk.
if grep -q '<relocate>' "$COMPONENT/PackageInfo"; then fail "bundles are not relocatable"; else pass "bundles are not relocatable"; fi
[[ -x "$COMPONENT/Scripts/postinstall" ]] && pass "postinstall present" || fail "postinstall present"

APP="$COMPONENT/Payload/Applications/Pabulong.app"
DRIVER="$COMPONENT/Payload/Library/Audio/Plug-Ins/HAL/PabulongHALPlugin.driver"

for bundle in "$APP" "$DRIVER"; do
  name="$(basename "$bundle")"
  echo "$name"
  [[ -d "$bundle" ]] || { fail "present in payload"; continue; }
  check "codesign --verify --deep --strict" codesign --verify --deep --strict "$bundle"
  info="$(codesign -dv --verbose=4 "$bundle" 2>&1)"
  grep -q "Authority=Developer ID Application:" <<<"$info" \
    && pass "signed with Developer ID Application" \
    || fail "signed with Developer ID Application" "$(echo "$info" | grep -m1 '^Authority=')"
  grep -q "flags=.*runtime" <<<"$info" && pass "hardened runtime" || fail "hardened runtime"
  grep -q "^Timestamp=" <<<"$info" && pass "secure timestamp" || fail "secure timestamp"
  version="$(plutil -extract CFBundleShortVersionString raw -o - "$bundle/Contents/Info.plist")"
  [[ "$version" == "$EXPECTED_VERSION" ]] && pass "version $version" || fail "version $version (VERSION file says $EXPECTED_VERSION)"
  check "notarization ticket stapled" xcrun stapler validate -q "$bundle"
  for bin in "$bundle"/Contents/MacOS/* "$bundle"/Contents/Resources/PabulongHelper; do
    [[ -f "$bin" ]] || continue
    archs="$(lipo -archs "$bin" 2>/dev/null)"
    [[ "$archs" == "arm64" ]] && pass "$(basename "$bin") is arm64" || fail "$(basename "$bin") is arm64" "$archs"
    foreign="$(otool -L "$bin" | tail -n +2 | awk '{print $1}' | grep -v -E '^(/usr/lib/|/System/|@rpath/libNoiselessHALPlugin)')"
    [[ -z "$foreign" ]] && pass "$(basename "$bin") links only system libraries" || fail "$(basename "$bin") links only system libraries" "$foreign"
  done
done

echo "Pabulong.app"
ents="$(codesign -d --entitlements :- "$APP" 2>/dev/null)"
grep -q "com.apple.security.device.audio-input" <<<"$ents" && pass "microphone entitlement" || fail "microphone entitlement"
check "Gatekeeper accepts the app (spctl -t exec)" spctl -a -vv -t exec "$APP"
symbols="$(nm -U "$APP/Contents/MacOS/Pabulong" 2>/dev/null)"
grep -q " _nlr_pipeline_create" <<<"$symbols" \
  && pass "Rust core linked (echo canceller, voice gate)" || fail "Rust core linked (echo canceller, voice gate)"
[[ -x "$APP/Contents/Resources/uninstall.sh" ]] && pass "uninstaller included" || fail "uninstaller included"
[[ -f "$APP/Contents/Resources/ThirdPartyNotices.md" && -f "$APP/Contents/Resources/LICENSE.txt" ]] \
  && pass "license and third-party notices included" || fail "license and third-party notices included"

echo
if [[ $failures -eq 0 ]]; then
  echo "All checks passed. This package is ready to publish."
else
  echo "$failures check(s) failed."
fi
exit $(( failures > 0 ))
