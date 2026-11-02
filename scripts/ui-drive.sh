#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Drives the decel UI under a private Xvfb with software Vulkan (lavapipe)
# and takes screenshots. Used to check UI changes from the devcontainer.
#
# Usage: scripts/ui-drive.sh <out-dir> <binary> [args...] -- <step>...
# Steps: key:<keys> type:<text> click:<x>,<y> dbl:<x>,<y> move:<x>,<y>
#        sleep:<s> shot:<name>
set -euo pipefail
out=$1; shift
cmd=()
while [ "$1" != "--" ]; do cmd+=("$1"); shift; done; shift
mkdir -p "$out"
Xvfb :99 -screen 0 1400x900x24 >/dev/null 2>&1 & XPID=$!
trap 'kill $XPID 2>/dev/null || true' EXIT
sleep 1
export DISPLAY=:99
env -u WAYLAND_DISPLAY VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json "${cmd[@]}" > "$out/app.log" 2>&1 & APID=$!
sleep 5
for step in "$@"; do
  case "$step" in
    key:*) xdotool key --delay 80 ${step#key:} ;;
    type:*) xdotool type --delay 60 "${step#type:}" ;;
    click:*) IFS=, read -r x y <<< "${step#click:}"; xdotool mousemove "$x" "$y" click 1 ;;
    dbl:*) IFS=, read -r x y <<< "${step#dbl:}"; xdotool mousemove "$x" "$y" click --repeat 2 --delay 80 1 ;;
    move:*) IFS=, read -r x y <<< "${step#move:}"; xdotool mousemove "$x" "$y" ;;
    sleep:*) sleep "${step#sleep:}" ;;
    shot:*) sleep 0.6; import -display :99 -window root "$out/${step#shot:}.png" ;;
  esac
  sleep 0.3
done
kill $APID 2>/dev/null || true; sleep 0.5; kill -9 $APID 2>/dev/null || true
