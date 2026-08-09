#!/usr/bin/env bash
# Phase 3 correctness: HTTPS p50 within max(±10ms, ±10%) of injected delay (Q10).
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
SERVER="$ROOT/testdata/https-latency-server.py"
PROBE="$ROOT/testdata/https-probe.py"
LOG=/tmp/obs-correctness3.log
PORT=18444
DELAY_MS=50
CERT=/tmp/obsagent-tls3.crt
KEY=/tmp/obsagent-tls3.key

[ -x "$BIN" ] && [ -f "$SERVER" ] && [ -f "$PROBE" ]

pkill -f "https-latency-server.py" 2>/dev/null || true
pkill -f "/release/obsagent" 2>/dev/null || true
sleep 0.5

openssl req -x509 -newkey rsa:2048 -keyout "$KEY" -out "$CERT" -days 1 -nodes \
  -subj "/CN=localhost" >/dev/null 2>&1

PORT=$PORT TLS_CERT=$CERT TLS_KEY=$KEY python3 "$SERVER" >/tmp/obs-correctness3-server.log 2>&1 &
server_pid=$!
sleep 0.5
grep -q listening /tmp/obs-correctness3-server.log || {
  echo "FAIL: server did not listen"; cat /tmp/obs-correctness3-server.log; exit 1;
}

OBSAGENT_HEADLESS=1 RUST_LOG=warn timeout --signal=INT 25 \
  env OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
runner=$!
sleep 3

python3 "$PROBE" --port "$PORT" --repeat 5 \
  --path /fast --path /users/1 --path "/slow?delay_ms=${DELAY_MS}"

sleep 4
kill -INT "$runner" 2>/dev/null || true
wait "$runner" 2>/dev/null || true
kill "$server_pid" 2>/dev/null || true
wait "$server_pid" 2>/dev/null || true

echo '=== tail log ==='
tail -n 30 "$LOG"

line=$(grep -E 'GET /slow ' "$LOG" | tail -n 1 || true)
[ -n "$line" ] || { echo "FAIL: no GET /slow row"; exit 1; }
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
    raise SystemExit("FAIL: p50 outside Q10 tolerance")
print("PASS: p50 within Q10 tolerance")
PY
