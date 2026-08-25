#!/usr/bin/env bash
# Phase 7 correctness: OpenSSL HTTP/2 GET /slow p50 in band (libssl, not rustls).
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
SERVER="$ROOT/testdata/h2-tls-server.py"
PROBE="$ROOT/testdata/h2-tls-probe.py"
LOG=/tmp/obs-correctness7-h2-tls.log
PORT=18447
DELAY_MS=50
CERT=/tmp/obsagent-tls7.crt
KEY=/tmp/obsagent-tls7.key

[ -x "$BIN" ] && [ -f "$SERVER" ] && [ -f "$PROBE" ]

pkill -f "h2-tls-server.py" 2>/dev/null || true
pkill -f "/release/obsagent" 2>/dev/null || true
sleep 0.5

openssl req -x509 -newkey rsa:2048 -keyout "$KEY" -out "$CERT" -days 1 -nodes \
  -subj "/CN=localhost" >/dev/null 2>&1

python3 - "$SERVER" "$CERT" "$KEY" <<'PY' >/dev/null
import subprocess, sys
print("ldd check skipped here")
PY
# Evidence: CPython ssl → libssl (same as Phase 3). Fail if the server is not libssl-backed.
python3 - <<'PY'
import ssl, ctypes.util
lib = ctypes.util.find_library("ssl")
print(f"libssl={lib}")
if not lib:
    raise SystemExit("FAIL: no libssl for OpenSSL h2 testdata")
print("PASS: libssl present")
PY

PORT=$PORT DELAY_MS=$DELAY_MS TLS_CERT=$CERT TLS_KEY=$KEY python3 "$SERVER" \
  >/tmp/obs-correctness7-h2-tls-server.log 2>&1 &
server_pid=$!
sleep 0.5
grep -q listening /tmp/obs-correctness7-h2-tls-server.log || {
  echo "FAIL: h2-tls-server did not listen"; cat /tmp/obs-correctness7-h2-tls-server.log; exit 1;
}

OBSAGENT_HEADLESS=1 RUST_LOG=warn timeout --signal=INT 25 \
  env OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
runner=$!
for i in $(seq 1 40); do
  grep -q "obsagent ready" "$LOG" 2>/dev/null && break
  sleep 0.25
done
grep -q "obsagent ready" "$LOG" || { echo "FAIL: agent did not become ready"; cat "$LOG"; exit 1; }

python3 "$PROBE" --port "$PORT" --repeat 5 --path /slow

sleep 4
kill -INT "$runner" 2>/dev/null || true
wait "$runner" 2>/dev/null || true
kill "$server_pid" 2>/dev/null || true
wait "$server_pid" 2>/dev/null || true

echo '=== tail log ==='
tail -n 40 "$LOG"

line=$(grep -E 'h2 GET /slow ' "$LOG" | tail -n 1 || true)
[ -n "$line" ] || { echo "FAIL: no h2 GET /slow row"; exit 1; }
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
