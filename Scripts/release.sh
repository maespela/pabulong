#!/usr/bin/env bash
# Builds a release: bumps VERSION, builds the installer, signs it for in-app
# updates and adds it to the update feed. One-time setup is in docs/RELEASING.md.
#
#   ./Scripts/release.sh <version> <notes.txt>               Developer ID, notarized
#   ./Scripts/release.sh --dev-signed <version> <notes.txt>  Apple Development (interim)
#
# notes.txt: the release notes, one short plain sentence per line.
#
# The Developer ID path picks both Developer ID certificates from the keychain
# and uses the notarytool profile "pabulong-notary". The --dev-signed path signs
# the app and driver with the Apple Development certificate and leaves the pkg
# unsigned, as the download page describes (Open Anyway). Override with
# SIGN_IDENTITY, PKG_SIGN_IDENTITY or NOTARY_PROFILE.
#
# Nothing is uploaded. The script prints what to upload and where.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

DEV_SIGNED=0
if [[ "${1:-}" == "--dev-signed" ]]; then
  DEV_SIGNED=1
  shift
fi
NEW_VERSION="${1:?usage: release.sh [--dev-signed] <version> <notes.txt>}"
NOTES="${2:?usage: release.sh [--dev-signed] <version> <notes.txt>}"
[[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "ERROR: version must look like 1.2.3." >&2; exit 1; }
[[ -s "$NOTES" ]] || { echo "ERROR: release notes $NOTES are missing or empty." >&2; exit 1; }
NOTES="$(cd "$(dirname "$NOTES")" && pwd)/$(basename "$NOTES")"

# The download folder is cached as immutable, so a version can only ship once.
APPCAST="${APPCAST:-$ROOT/release/appcast.xml}"
if [[ -f "$APPCAST" ]] && grep -qF "<sparkle:shortVersionString>$NEW_VERSION<" "$APPCAST"; then
  echo "ERROR: $NEW_VERSION is already in $APPCAST. Pick a new version." >&2
  exit 1
fi
LATEST=""
if [[ -f "$APPCAST" ]]; then
  LATEST="$(sed -n 's/.*<sparkle:shortVersionString>\(.*\)<.*/\1/p' "$APPCAST" | head -1)"
fi
if [[ -n "$LATEST" ]] && [[ "$(printf '%s\n%s\n' "$LATEST" "$NEW_VERSION" | sort -V | tail -1)" != "$NEW_VERSION" ]]; then
  echo "ERROR: $NEW_VERSION is not newer than $LATEST, the latest version in the feed." >&2
  exit 1
fi

find_identity() {
  security find-identity -v -p codesigning | sed -n "s/.*\"\($1: .*\)\"/\1/p" | head -1
}
find_installer_identity() {
  security find-identity -v | sed -n "s/.*\"\($1: .*\)\"/\1/p" | head -1
}

if [[ "$DEV_SIGNED" == "1" ]]; then
  SIGN_IDENTITY="${SIGN_IDENTITY:-$(find_identity "Apple Development")}"
  [[ -n "$SIGN_IDENTITY" ]] || { echo "ERROR: no Apple Development certificate in the keychain." >&2; exit 1; }
else
  SIGN_IDENTITY="${SIGN_IDENTITY:-$(find_identity "Developer ID Application")}"
  PKG_SIGN_IDENTITY="${PKG_SIGN_IDENTITY:-$(find_installer_identity "Developer ID Installer")}"
  NOTARY_PROFILE="${NOTARY_PROFILE:-pabulong-notary}"
  [[ -n "$SIGN_IDENTITY" ]] || { echo "ERROR: no Developer ID Application certificate in the keychain. See docs/RELEASING.md." >&2; exit 1; }
  [[ -n "$PKG_SIGN_IDENTITY" ]] || { echo "ERROR: no Developer ID Installer certificate in the keychain. See docs/RELEASING.md." >&2; exit 1; }
fi

# Check the update signing key before the long build.
swift package resolve >/dev/null
source "$ROOT/Scripts/common.sh"
KEYCHAIN_KEY="$("$SPARKLE_BIN/generate_keys" --account "$SPARKLE_KEY_ACCOUNT" -p 2>/dev/null || true)"
if [[ "$KEYCHAIN_KEY" != "$SPARKLE_PUBLIC_ED_KEY" ]]; then
  echo "ERROR: the update signing key (account '$SPARKLE_KEY_ACCOUNT') is not in the keychain." >&2
  echo "       Restore it from the backup; see docs/RELEASING.md. Without it, installed copies can't update." >&2
  exit 1
fi

echo "$NEW_VERSION" > "$ROOT/VERSION"
export VERSION="$NEW_VERSION"
BUILD_ID="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="$ROOT/outputs/offline-full/Pabulong-$NEW_VERSION-$BUILD_ID"
PKG="$OUT_DIR/Pabulong-$NEW_VERSION-arm64.pkg"

"$ROOT/Scripts/build-rust-core.sh"

if [[ "$DEV_SIGNED" == "1" ]]; then
  BUILD_ID="$BUILD_ID" HARDENED=1 NOTARIZE=0 \
  SIGN_IDENTITY="$SIGN_IDENTITY" \
    "$ROOT/Scripts/package-offline-full-installer.sh"
else
  BUILD_ID="$BUILD_ID" HARDENED=1 NOTARIZE=1 \
  SIGN_IDENTITY="$SIGN_IDENTITY" \
  PKG_SIGN_IDENTITY="$PKG_SIGN_IDENTITY" \
  NOTARY_PROFILE="$NOTARY_PROFILE" \
    "$ROOT/Scripts/package-offline-full-installer.sh"
fi

APPCAST="$APPCAST" "$ROOT/Scripts/update-appcast.sh" "$PKG" "$NOTES"
cp "$APPCAST" "$OUT_DIR/appcast.xml"

cat <<EOF

Pabulong $NEW_VERSION is ready. Nothing has been uploaded.

Upload to $UPDATE_DOWNLOAD_BASE/ (pkg first, then the feed):
  1. $PKG
  2. $OUT_DIR/appcast.xml   (as appcast.xml, replacing the old one)
Then update the download page's version, size and SHA-256 from $OUT_DIR/release.json,
and commit VERSION and release/appcast.xml.
EOF
