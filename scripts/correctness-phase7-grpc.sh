#!/usr/bin/env bash
# Phase 7 correctness: tonic h2c gRPC /slow.Slow/Sleep p50 in band.
set -euo pipefail

BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
SERVER_BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/grpc-slow"
PROBE="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/grpc-probe"
LOG=/tmp/obs-correctness7-grpc.log
PORT=18097
DELAY_MS=50

[ -x "$BIN" ] && [ -x "$SERVER_BIN" ] && [ -x "$PROBE" ]

pkill -f "/release/grpc-slow" 2>/dev/null || true
pkill -f "/release/obsagent" 2>/dev/null || true
sleep 0.5

PORT=$PORT "$SERVER_BIN" >/tmp/obs-correctness7-grpc-server.log 2>&1 &
server_pid=$!
sleep 0.5
grep -q listening /tmp/obs-correctness7-grpc-server.log || {
  echo "FAIL: grpc-slow did not listen"; cat /tmp/obs-correctness7-grpc-server.log; exit 1;
}

OBSAGENT_HEADLESS=1 RUST_LOG=warn timeout --signal=INT 25 \
  env OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
runner=$!
for i in $(seq 1 40); do
  grep -q "obsagent ready" "$LOG" 2>/dev/null && break
  sleep 0.25
done
grep -q "obsagent ready" "$LOG" || { echo "FAIL: agent did not become ready"; cat "$LOG"; exit 1; }

"$PROBE" --port "$PORT" --repeat 5 --delay-ms "$DELAY_MS"

sleep 4
kill -INT "$runner" 2>/dev/null || true
wait "$runner" 2>/dev/null || true
kill "$server_pid" 2>/dev/null || true
wait "$server_pid" 2>/dev/null || true

echo '=== tail log ==='
tail -n 40 "$LOG"

line=$(grep -E 'grpc POST /slow.Slow/Sleep' "$LOG" | tail -n 1 || true)
[ -n "$line" ] || { echo "FAIL: no grpc POST /slow.Slow/Sleep row"; exit 1; }
echo "observed: $line"

p50=$(echo "$line" | sed -n 's/.* p50=\([0-9.]*\)ms.*/\1/p')
[ -n "$p50" ] || { echo "FAIL: parse p50"; exit 1; }

python3 - "$DELAY_MS" "$p50" <<'PY'
import sys
delay = float(sys.argv[1])
p50 = float(sys.argv[2])
tol = max(10.0, delay * 0.10)
lo, hi = delay - tol, delay + tol
print(f"delay={delay}ms p50={p50}ms tol=±{tol}ms band=[{lo},{hi}]")
if not (lo <= p50 <= hi):
    raise SystemExit("FAIL: p50 outside band")
print("PASS: p50 within band")
PY
