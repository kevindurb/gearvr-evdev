#!/usr/bin/env bash
# Hardware-free Steam Deck probe for the virtual evdev gamepad.
#
# Run this on the Deck (not on the development host). It assumes Steam is
# running and that the binary has been copied to ~/gearvr-probe/gearvr-cli.
#
#   ./scripts/deck-probe.sh [duration-seconds]
#
# The duration must cover the two `evtest` runs and `jstest`; 25 seconds is the
# default. It prints the evdev/jstest view and any new Steam controller-log
# lines.

set -euo pipefail

DURATION="${1:-25}"
BIN="${GEARVR_BIN:-$HOME/gearvr-probe/gearvr-cli}"
OUTDIR="${GEARVR_OUTDIR:-$HOME/gearvr-probe}"
STEAM_LOG="$HOME/.local/share/Steam/logs/controller.txt"

if [[ ! -x "$BIN" ]]; then
    echo "missing executable: $BIN" >&2
    exit 1
fi
if [[ ! -r "$STEAM_LOG" ]]; then
    echo "warning: Steam controller log not found: $STEAM_LOG" >&2
fi

mkdir -p "$OUTDIR"
BEFORE=0
if [[ -r "$STEAM_LOG" ]]; then
    BEFORE=$(wc -l < "$STEAM_LOG")
fi

"$BIN" synth --duration "$DURATION" --rate 68 --samples 3 --backend evdev --motion \
    >"$OUTDIR/synth.log" 2>&1 &
PID=$!
cleanup() {
    kill "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
}
trap cleanup EXIT

sleep 2
echo "=== CLI log ==="
cat "$OUTDIR/synth.log"

EV=$(grep -l 'Gear VR Controller$' /sys/class/input/event*/device/name 2>/dev/null \
    | head -1 | sed 's#/sys/class/input/##; s#/device/name##' || true)
MOV=$(grep -l 'Gear VR Controller Motion$' /sys/class/input/event*/device/name 2>/dev/null \
    | head -1 | sed 's#/sys/class/input/##; s#/device/name##' || true)

if [[ -z "$EV" ]]; then
    echo "gamepad node not found" >&2
    exit 1
fi

echo
echo "=== nodes ==="
echo "gamepad: /dev/input/$EV"
[[ -n "$MOV" ]] && echo "motion:  /dev/input/$MOV"
grep -A8 -B1 'Gear VR Controller' /proc/bus/input/devices || true

echo
echo "=== gamepad capabilities ==="
timeout 2 evtest "/dev/input/$EV" 2>&1 | sed -n '1,30p' || true

echo
echo "=== gamepad events (buttons + axes, 6s) ==="
timeout 6 evtest "/dev/input/$EV" >"$OUTDIR/evtest-gamepad.txt" 2>&1 || true
grep -E 'BTN_|ABS_X|ABS_Y|SYN_REPORT' "$OUTDIR/evtest-gamepad.txt" | head -40 || true

JS=$(grep -A8 -B1 'N: Name="Gear VR Controller"' /proc/bus/input/devices \
    | grep -o 'js[0-9]*' | head -1 || true)
if [[ -n "$JS" ]] && command -v jstest >/dev/null; then
    echo
    echo "=== jstest /dev/input/$JS (4s) ==="
    timeout 4 jstest --event "/dev/input/$JS" 2>&1 | head -25 || true
fi

if [[ -n "$MOV" ]]; then
    echo
    echo "=== motion axes ==="
    timeout 2 evtest "/dev/input/$MOV" >"$OUTDIR/evtest-motion.txt" 2>&1 || true
    if grep -q "Permission denied" "$OUTDIR/evtest-motion.txt"; then
        echo "motion node is not readable by this user; install packaging/udev/70-gearvr-evdev.rules"
    else
        sed -n '1,30p' "$OUTDIR/evtest-motion.txt"
    fi
fi

sleep 2
if [[ -r "$STEAM_LOG" ]]; then
    echo
    echo "=== new Steam controller log ==="
    tail -n +$((BEFORE + 1)) "$STEAM_LOG" \
        | grep -iE '1209|Gear|Local Device|controller device opened|configset|SDL Mapping' \
        | tail -30 || true
fi

echo
echo "done; raw captures in $OUTDIR"
