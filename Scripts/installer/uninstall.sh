#!/bin/bash
# Removes Pabulong and the Pabulong Mic audio driver.
#
#   sudo /Applications/Pabulong.app/Contents/Resources/uninstall.sh
#   sudo .../uninstall.sh --delete-data   # also delete settings and recordings
#
# Recordings in ~/Library/Application Support/Pabulong are kept unless you pass
# --delete-data.
set -euo pipefail

DELETE_DATA=0
for arg in "$@"; do
  case "$arg" in
    --delete-data) DELETE_DATA=1 ;;
    -h|--help) sed -n '2,8p' "$0"; exit 0 ;;
    *) echo "Unknown option: $arg" >&2; exit 2 ;;
  esac
done

if [[ $EUID -ne 0 ]]; then
  exec sudo "$0" "$@"
fi

# The person who ran sudo, or whoever is logged in at the screen.
TARGET_USER="${SUDO_USER:-$(stat -f %Su /dev/console)}"
TARGET_HOME="$(dscl . -read "/Users/$TARGET_USER" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"

/usr/bin/pkill -x Pabulong 2>/dev/null || true
/usr/bin/pkill -x PabulongHelper 2>/dev/null || true

/bin/rm -rf "/Applications/Pabulong.app"
/bin/rm -rf "/Library/Audio/Plug-Ins/HAL/PabulongHALPlugin.driver"
/bin/rm -rf "/Library/Audio/Plug-Ins/HAL/NoiselessHALPlugin.driver"
/usr/sbin/pkgutil --forget com.nextmatterpro.pabulong.installer >/dev/null 2>&1 || true

if [[ -n "$TARGET_HOME" ]]; then
  /bin/rm -f "$TARGET_HOME/Library/LaunchAgents/com.nextmatterpro.pabulong.helper.plist"
  if [[ "$DELETE_DATA" == "1" ]]; then
    /bin/rm -rf "$TARGET_HOME/Library/Application Support/Pabulong"
    /usr/bin/sudo -u "$TARGET_USER" /usr/bin/defaults delete com.nextmatterpro.pabulong >/dev/null 2>&1 || true
  fi
fi

# Reload the audio system so Pabulong Mic disappears from the device list.
/usr/bin/killall coreaudiod >/dev/null 2>&1 || true

echo "Pabulong is uninstalled."
if [[ "$DELETE_DATA" != "1" && -n "$TARGET_HOME" && -d "$TARGET_HOME/Library/Application Support/Pabulong" ]]; then
  echo "Your settings and recordings are still in $TARGET_HOME/Library/Application Support/Pabulong."
fi
