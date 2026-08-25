#!/usr/bin/env bash
# Milestone 7: h2c gRPC row appears in headless output.
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
SERVER_BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/grpc-slow"
PROBE="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/grpc-probe"
LOG=$(mktemp)
PORT=18098

if [ "$(id -u)" -eq 0 ]; then PRIV=(env); else PRIV=(sudo -E); fi

[ -x "$BIN" ] || { echo "FAIL: $BIN not built"; exit 1; }
[ -x "$SERVER_BIN" ] || { echo "FAIL: $SERVER_BIN not built"; exit 1; }
[ -x "$PROBE" ] || { echo "FAIL: $PROBE not built"; exit 1; }

PORT=$PORT "$SERVER_BIN" >/tmp/obs-grpc-slow.log 2>&1 &
server_pid=$!
cleanup() {
  kill "$server_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true
}
trap cleanup EXIT
sleep 0.5

OBSAGENT_HEADLESS=1 RUST_LOG=warn timeout --signal=INT 20 "${PRIV[@]}" env OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
runner=$!
for i in $(seq 1 40); do
  grep -q "obsagent ready" "$LOG" 2>/dev/null && break
  sleep 0.25
done
grep -q "obsagent ready" "$LOG" || { echo "FAIL: agent did not become ready"; cat "$LOG"; exit 1; }

"$PROBE" --port "$PORT" --repeat 3 --delay-ms 20 || {
  echo "FAIL: grpc-probe errors"; cat /tmp/obs-grpc-slow.log; exit 1;
}
sleep 3
kill -INT "$runner" 2>/dev/null || true
wait "$runner" 2>/dev/null || true

grep -E 'grpc POST /slow.Slow/Sleep' "$LOG" >/dev/null || {
  echo "FAIL: no grpc row"; tail -n 40 "$LOG"; exit 1;
}
echo "PASS: milestone 7 grpc row present"
