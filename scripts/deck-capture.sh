#!/usr/bin/env bash
# First-contact capture for a physical Gear VR Controller.
#
# Run this on the Steam Deck (not the development host), after the controller
# has been paired and trusted in BlueZ:
#
#   ./scripts/deck-capture.sh [device] [seconds]
#
# It scans, records the handshake, then reads the constant settings/calibration
# records. Two timestamped JSONL files land in ~/gearvr-captures/. Replay them
# with the gearvr-cli `replay` subcommand.

set -euo pipefail

DEVICE="${1:-Gear VR}"
DURATION="${2:-60}"
BIN="${GEARVR_BIN:-$HOME/gearvr-probe/gearvr-cli}"
OUTDIR="${GEARVR_OUTDIR:-$HOME/gearvr-captures}"
STAMP="$(date +%Y-%m-%d-%H%M%S)"

if [[ ! -x "$BIN" ]]; then
    echo "missing executable: $BIN" >&2
    exit 1
fi

mkdir -p "$OUTDIR"

echo "=== scan ==="
"$BIN" scan --seconds 5 || true

echo
echo "=== capture handshake (${DURATION}s) ==="
FIRST="$OUTDIR/$STAMP-first.jsonl"
"$BIN" capture "$DEVICE" --seconds "$DURATION" -o "$FIRST"

echo
echo "=== probe constant records (03 calibration, 05 settings) ==="
RECORDS="$OUTDIR/$STAMP-records.jsonl"
if ! "$BIN" probe "$DEVICE" -o "$RECORDS"; then
    echo "probe failed; the handshake capture is still valid" >&2
fi

echo
echo "=== first capture summary ==="
"$BIN" replay "$FIRST" --include-tx --summary --limit 15

if [[ -s "$RECORDS" ]]; then
    echo
    echo "=== records capture ==="
    "$BIN" replay "$RECORDS" --include-tx
fi

echo
echo "captures:"
echo "  $FIRST"
[[ -s "$RECORDS" ]] && echo "  $RECORDS"
