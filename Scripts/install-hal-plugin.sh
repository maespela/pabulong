#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DRIVER="$ROOT/outputs/PabulongHALPlugin.driver"
DEST="/Library/Audio/Plug-Ins/HAL/PabulongHALPlugin.driver"
STAGED="/tmp/PabulongHALPlugin.driver"
# Driver installed by builds named Noiseless; removed so only one virtual mic exists.
LEGACY="/Library/Audio/Plug-Ins/HAL/NoiselessHALPlugin.driver"

if [[ ! -d "$DRIVER" ]]; then
  "$ROOT/Scripts/package-hal-plugin.sh"
fi

rm -rf "$STAGED"
/usr/bin/ditto "$DRIVER" "$STAGED"

sudo rm -rf "$DEST" "$LEGACY"
sudo /usr/bin/ditto "$STAGED" "$DEST"
sudo chown -R root:wheel "$DEST"
sudo killall coreaudiod || true

echo "Installed $DEST"

# Warn early if the bundle is only ad-hoc signed: coreaudiod on Apple Silicon
# frequently refuses to load such plug-ins, so the device silently never appears.
if codesign -dv --verbose=2 "$DEST" 2>&1 | grep -q "adhoc"; then
  echo "WARNING: driver is ad-hoc signed. If it does not appear below, re-sign it"
  echo "         with a Developer ID: SIGN_IDENTITY=\"Developer ID Application: ...\" ./Scripts/package-hal-plugin.sh"
fi

# Give coreaudiod a moment to rescan, then verify the device actually loaded.
echo "Waiting for coreaudiod to rescan..."
sleep 2
if system_profiler SPAudioDataType 2>/dev/null | grep -qi "Pabulong Mic"; then
  echo "SUCCESS: 'Pabulong Mic' is now visible to CoreAudio."
  echo "Select it in System Settings > Sound or your meeting app."
else
  echo "FAILED: 'Pabulong Mic' did not appear. coreaudiod did not load the plug-in."
  echo "  Most likely cause: code signature. Re-sign with a Developer ID and reinstall,"
  echo "  or check the log:  log show --last 2m --predicate 'process == \"coreaudiod\"' | grep -i pabulong"
fi
