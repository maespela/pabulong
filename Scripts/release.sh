#!/usr/bin/env bash
# Builds the public release: Developer ID signed, notarized and stapled.
# One-time setup is in docs/RELEASING.md.
#
#   ./Scripts/release.sh
#
# Picks the Developer ID certificates from the keychain and uses the notarytool
# profile "pabulong-notary". Override with SIGN_IDENTITY, PKG_SIGN_IDENTITY or
# NOTARY_PROFILE.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

find_identity() {
  security find-identity -v | sed -n "s/.*\"\($1: .*\)\"/\1/p" | head -1
}

SIGN_IDENTITY="${SIGN_IDENTITY:-$(find_identity "Developer ID Application")}"
PKG_SIGN_IDENTITY="${PKG_SIGN_IDENTITY:-$(find_identity "Developer ID Installer")}"
NOTARY_PROFILE="${NOTARY_PROFILE:-pabulong-notary}"
[[ -n "$SIGN_IDENTITY" ]] || { echo "ERROR: no Developer ID Application certificate in the keychain. See docs/RELEASING.md." >&2; exit 1; }
[[ -n "$PKG_SIGN_IDENTITY" ]] || { echo "ERROR: no Developer ID Installer certificate in the keychain. See docs/RELEASING.md." >&2; exit 1; }

"$ROOT/Scripts/build-rust-core.sh"

HARDENED=1 NOTARIZE=1 \
SIGN_IDENTITY="$SIGN_IDENTITY" \
PKG_SIGN_IDENTITY="$PKG_SIGN_IDENTITY" \
NOTARY_PROFILE="$NOTARY_PROFILE" \
  "$ROOT/Scripts/package-offline-full-installer.sh"
