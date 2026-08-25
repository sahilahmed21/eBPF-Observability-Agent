#!/usr/bin/env bash
# Phase 12: profile join on GET /slow — prof_hit >= 50% of N slow requests (N=20).
set -euo pipefail

BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
SERVER_BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/latency-server"
PROBE="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/http-probe"
LOG=/tmp/obs-correctness12.log
PORT=18088
N=20
DELAY_MS=50

[ -x "$BIN" ] && [ -x "$SERVER_BIN" ] && [ -x "$PROBE" ]

pkill -f "latency-server" 2>/dev/null || true
pkill -f "/release/obsagent" 2>/dev/null || true
sleep 0.5

PORT=$PORT "$SERVER_BIN" >/tmp/obs-correctness12-server.log 2>&1 &
server_pid=$!
sleep 0.5
grep -q listening /tmp/obs-correctness12-server.log || {
  echo "FAIL: server did not listen"; cat /tmp/obs-correctness12-server.log; exit 1;
}

OBSAGENT_HEADLESS=1 OBSAGENT_PROFILE=1 OBSAGENT_TRACE_SAMPLE=1 RUST_LOG=info \
  timeout --signal=INT 60 \
  env OBSAGENT_HEADLESS=1 OBSAGENT_PROFILE=1 OBSAGENT_TRACE_SAMPLE=1 RUST_LOG=info \
  stdbuf -oL -eL "$BIN" >"$LOG" 2>&1 &
runner=$!
# Wait for ready (profiles attach can take a few seconds)
for _ in $(seq 1 30); do
  if grep -q 'obsagent ready\|PROFILE=1: perf_event\|perf attach failed' "$LOG" 2>/dev/null; then
    break
  fi
  sleep 0.5
done
if grep -q 'perf attach failed' "$LOG" 2>/dev/null; then
  echo "FAIL: perf attach failed (see $LOG)"
  tail -n 40 "$LOG"
  kill -INT "$runner" 2>/dev/null || true
  exit 1
fi
if ! grep -q 'obsagent ready\|PROFILE=1: perf_event' "$LOG" 2>/dev/null; then
  echo "FAIL: agent did not become ready (empty/buffered log?)"
  tail -n 40 "$LOG" || true
  kill -INT "$runner" 2>/dev/null || true
  exit 1
fi
sleep 2

for _ in $(seq 1 "$N"); do
  "$PROBE" --port "$PORT" --repeat 1 --path "/slow?delay_ms=${DELAY_MS}" >/dev/null
done

sleep 8
kill -INT "$runner" 2>/dev/null || true
# Give line-buffered process time to flush final ticks
sleep 1
wait "$runner" 2>/dev/null || true
kill "$server_pid" 2>/dev/null || true
wait "$server_pid" 2>/dev/null || true

echo '=== tail log ==='
tail -n 40 "$LOG"

hits=$(grep -oE 'prof_hit=[0-9]+' "$LOG" | sed 's/prof_hit=//' | sort -n | tail -1 || true)
hits=${hits:-0}
echo "profile_join_hits=$hits"

python3 - "$hits" "$N" <<'PY'
import sys
hits = int(sys.argv[1])
n = int(sys.argv[2])
need = (n + 1) // 2
print(f"need>={need} profile join hits for N={n}")
if hits < need:
    raise SystemExit(f"FAIL: profile join hit rate {hits}/{n} < 50%")
print(f"PASS: profile join hit rate {hits}/{n} >= 50%")
PY

line=$(grep -E 'GET /slow ' "$LOG" | tail -n 1 || true)
[ -n "$line" ] || { echo "FAIL: no GET /slow row"; exit 1; }
echo "observed: $line"
