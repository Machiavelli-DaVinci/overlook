#!/usr/bin/env bash
# Launch Overlook against a profile directory, optionally drive it with keystrokes, and
# capture the window on Hyprland. Used to refresh branding/previews/overlook-native-*.png.
#
#   scripts/capture-window.sh <data_dir> <out.png> [bash snippet run after focus]
#
# Example (reply while offline, then send):
#   scripts/capture-window.sh /tmp/ol-demo shot.png \
#     "wtype -M ctrl -s 200 r -m ctrl; sleep 1; wtype 'Hello'; wtype -M ctrl -P Return -p Return -m ctrl"
#
# Requires: hyprctl, grim, and wtype for keystrokes. Environment is passed through, so
# OVERLOOK_SAMPLE_ONLINE=1 starts the sample connection connected.
set -u
DATA="$1"; OUT="$2"; KEYS="${3:-}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/debug/overlook"
[ -x "$BIN" ] || cargo build -p overlook --manifest-path "$ROOT/Cargo.toml"
pkill -f "^$BIN" 2>/dev/null; sleep 0.5
OVERLOOK_DATA_DIR="$DATA" setsid nohup "$BIN" > /tmp/overlook-capture.log 2>&1 < /dev/null & disown
for _ in $(seq 1 40); do
  sleep 0.25
  hyprctl clients -j | grep -q '"class": "ai.lastrefuge.Overlook"' && break
done
sleep 1.5
hyprctl dispatch focuswindow class:ai.lastrefuge.Overlook > /dev/null
sleep 0.4
[ -n "$KEYS" ] && bash -c "$KEYS"
sleep 1.2
GEOM=$(hyprctl clients -j | python3 -c "
import json,sys
for c in json.load(sys.stdin):
    if c['class']=='ai.lastrefuge.Overlook':
        x,y=c['at']; w,h=c['size']; print(f'{x},{y} {w}x{h}')")
ACTIVE=$(hyprctl activewindow -j | python3 -c "import json,sys; print(json.load(sys.stdin).get('class',''))")
if [ "$ACTIVE" != "ai.lastrefuge.Overlook" ]; then echo "skipped: Overlook is not the active window (active=$ACTIVE)"; exit 2; fi
grim -g "$GEOM" "$OUT" && echo "captured $OUT ($GEOM)"
