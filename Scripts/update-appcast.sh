#!/usr/bin/env bash
# Adds a release to the update feed that Pabulong's "Check for Updates" reads.
#
#   ./Scripts/update-appcast.sh <Pabulong-x.y.z-arm64.pkg> <notes.txt>
#
# notes.txt holds the release notes, one short plain sentence per line; each
# line becomes a bullet in the update window.
#
# Signs the pkg with the EdDSA key in the login keychain (Sparkle's sign_update),
# puts the new item at the top of release/appcast.xml (keeping the last few),
# signs the feed itself, and checks both signatures. Upload the pkg and the
# appcast to UPDATE_DOWNLOAD_BASE afterwards; see docs/RELEASING.md.
#
# Environment:
#   APPCAST               feed file to update (default: release/appcast.xml)
#   UPDATE_DOWNLOAD_BASE  where the pkg is served (default in Scripts/common.sh)
#   SPARKLE_KEY_ACCOUNT   keychain account of the private key (default: pabulong)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PKG="${1:?usage: update-appcast.sh <pkg> <notes.txt>}"
NOTES="${2:?usage: update-appcast.sh <pkg> <notes.txt>}"
[[ -f "$PKG" ]] || { echo "ERROR: $PKG not found." >&2; exit 1; }
[[ -s "$NOTES" ]] || { echo "ERROR: release notes $NOTES are missing or empty." >&2; exit 1; }

# The version comes from the pkg's app, not the VERSION file, so the feed can't
# describe a different build than the one it points to.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pkgutil --expand-full "$PKG" "$WORK/x" >/dev/null
APP_PLIST="$(find "$WORK/x" -path '*/Payload/Applications/Pabulong.app/Contents/Info.plist' | head -1)"
[[ -n "$APP_PLIST" ]] || { echo "ERROR: no Pabulong.app in $PKG." >&2; exit 1; }
VERSION="$(plutil -extract CFBundleShortVersionString raw -o - "$APP_PLIST")"
BUILD_NUMBER="$(plutil -extract CFBundleVersion raw -o - "$APP_PLIST")"
MIN_SYSTEM="$(plutil -extract LSMinimumSystemVersion raw -o - "$APP_PLIST")"
source "$ROOT/Scripts/common.sh"

APPCAST="${APPCAST:-$ROOT/release/appcast.xml}"
SIGN_UPDATE="$SPARKLE_BIN/sign_update"
[[ -x "$SIGN_UPDATE" ]] || { echo "ERROR: $SIGN_UPDATE is missing. Run: swift package resolve" >&2; exit 1; }

# The pkg's public key must match the one the installed apps trust.
PKG_KEY="$(plutil -extract SUPublicEDKey raw -o - "$APP_PLIST" 2>/dev/null || true)"
[[ "$PKG_KEY" == "$SPARKLE_PUBLIC_ED_KEY" ]] \
  || { echo "ERROR: the pkg's SUPublicEDKey ('$PKG_KEY') is not the release key." >&2; exit 1; }
KEYCHAIN_KEY="$("$SPARKLE_BIN/generate_keys" --account "$SPARKLE_KEY_ACCOUNT" -p 2>/dev/null || true)"
[[ "$KEYCHAIN_KEY" == "$SPARKLE_PUBLIC_ED_KEY" ]] || {
  echo "ERROR: the keychain has no private key for SUPublicEDKey (account '$SPARKLE_KEY_ACCOUNT')." >&2
  echo "       Restore it from the backup; see docs/RELEASING.md." >&2
  exit 1
}

echo "Signing $(basename "$PKG") with the EdDSA key ..."
SIGNATURE="$("$SIGN_UPDATE" --account "$SPARKLE_KEY_ACCOUNT" -p "$PKG")"
"$SIGN_UPDATE" --account "$SPARKLE_KEY_ACCOUNT" --verify "$PKG" "$SIGNATURE" >/dev/null

mkdir -p "$(dirname "$APPCAST")"
/usr/bin/python3 -I - "$APPCAST" "$NOTES" <<PY
import html, re, sys
from email.utils import formatdate

appcast, notes_path = sys.argv[1], sys.argv[2]
version, build, min_system = "$VERSION", "$BUILD_NUMBER", "$MIN_SYSTEM"
url = "$UPDATE_DOWNLOAD_BASE/" + "$(basename "$PKG")"
length = "$(stat -f %z "$PKG")"
signature = "$SIGNATURE"
keep = 5

lines = [l.strip() for l in open(notes_path, encoding="utf-8") if l.strip()]
bullets = "\n".join(f"        <li>{html.escape(l)}</li>" for l in lines)
item = f"""    <item>
      <title>Pabulong {version}</title>
      <pubDate>{formatdate(usegmt=True)}</pubDate>
      <sparkle:version>{build}</sparkle:version>
      <sparkle:shortVersionString>{version}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>{min_system}</sparkle:minimumSystemVersion>
      <sparkle:hardwareRequirements>arm64</sparkle:hardwareRequirements>
      <description><![CDATA[
      <ul>
{bullets}
      </ul>
      ]]></description>
      <enclosure url="{html.escape(url)}" length="{length}" type="application/octet-stream"
                 sparkle:installationType="package" sparkle:edSignature="{signature}"/>
    </item>"""

items = []
try:
    old = open(appcast, encoding="utf-8").read()
    for block in re.findall(r"    <item>.*?</item>", old, re.S):
        v = re.search(r"<sparkle:shortVersionString>(.*?)</", block)
        if v and v.group(1) != version:
            items.append(block)
except FileNotFoundError:
    pass

body = "\n".join([item] + items[: keep - 1])
feed = f"""<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Pabulong</title>
    <link>https://vastarterhub.com/pabulong/</link>
    <description>Pabulong for Mac updates</description>
    <language>en</language>
{body}
  </channel>
</rss>
"""
open(appcast, "w", encoding="utf-8").write(feed)
PY

# Sign the feed too (SURequireSignedFeed), then check what the app will check.
"$SIGN_UPDATE" --account "$SPARKLE_KEY_ACCOUNT" --disable-signing-warning "$APPCAST" >/dev/null
"$SIGN_UPDATE" --account "$SPARKLE_KEY_ACCOUNT" --verify "$APPCAST" >/dev/null
xmllint --noout "$APPCAST"

echo "Appcast: $APPCAST"
echo "  Pabulong $VERSION -> $UPDATE_DOWNLOAD_BASE/$(basename "$PKG")"
echo "  $(stat -f %z "$PKG") bytes, EdDSA signature verified, feed signed."
