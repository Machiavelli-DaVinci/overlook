#!/usr/bin/env bash
# Records the Google OAuth verification demo: full screen with microphone, Overlook on a
# fresh profile so the sign-in flow is shown from the start. Quit Overlook to stop.
#
#   scripts/record-demo.sh [out.mp4] [--keep-profile]
#
# Needs gpu-screen-recorder (Omarchy ships it). The browser window used for Google's consent
# screen is part of the recording, so keep it on this monitor.
set -euo pipefail

out=${1:-"$HOME/Videos/overlook-oauth-demo-$(date +%Y%m%d-%H%M%S).mp4"}
keep=${2:-}
profile=$(mktemp -d "${XDG_RUNTIME_DIR:-/tmp}/overlook-demo.XXXXXX")
bin=${OVERLOOK_BIN:-"$(dirname "$0")/../target/release/overlook"}
[[ -x $bin ]] || bin="$(dirname "$0")/../target/debug/overlook"
[[ -x $bin ]] || { echo "build Overlook first: cargo build --release -p overlook" >&2; exit 1; }
command -v gpu-screen-recorder >/dev/null || { echo "gpu-screen-recorder not installed" >&2; exit 1; }
mkdir -p "$(dirname "$out")"

echo "Recording to $out  (profile: $profile)"
echo "Close the Overlook window to stop."
gpu-screen-recorder -w screen -f 30 -q high -a default_input -o "$out" &
rec=$!
sleep 1

# Fresh profile: the sample account only, no Google accounts, so the sign-in is demonstrated.
OVERLOOK_DATA_DIR="$profile" OVERLOOK_SAMPLE_ONLINE=1 "$bin" || true

kill -INT "$rec" 2>/dev/null || true
wait "$rec" 2>/dev/null || true
[[ $keep == --keep-profile ]] || rm -rf "$profile"
echo "Saved $out"
