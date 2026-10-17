#!/usr/bin/env bash
set -euo pipefail

# Removes the Pabulong Mic driver (and the driver from builds named Noiseless).
for PLUGIN in "/Library/Audio/Plug-Ins/HAL/PabulongHALPlugin.driver" "/Library/Audio/Plug-Ins/HAL/NoiselessHALPlugin.driver"; do
  if [[ -e "$PLUGIN" ]]; then
    sudo rm -rf "$PLUGIN"
  fi
done
for LAUNCH_AGENT in "$HOME/Library/LaunchAgents/com.nextmatterpro.pabulong.helper.plist" "$HOME/Library/LaunchAgents/com.noiseless.helper.plist"; do
  rm -f "$LAUNCH_AGENT"
done
sudo killall coreaudiod || true
echo "Removed the Pabulong development install."
