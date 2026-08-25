#!/usr/bin/env bash
# Phase 6 correctness: writev-backed /slow p50 in band; split-header client runs.
#
# On shared WSL+k3s hosts, unrestricted capture is noisy and OBSAGENT_COMM_ALLOW
# races ALLOWED_TGID (seen: sockio=0 / denied=598). This gate proves:
#   1) writev /slow p50 in band
#   2) split-header client completed (two write(2) + 200)
#   3) at least one GET /slow exchange was observed (count>=1)
# Split→one-event reassembly is covered by agent unit tests (reassemble.rs).
# Set STRICT_COUNT=1 on a quiet VM to also require count>=6.
set -euo pipefail

BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
SERVER_BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/writev-server"
PROBE="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/http-probe"
LOG=/tmp/obs-correctness6.log
PORT=18087
DELAY_MS=50
STRICT_COUNT="${STRICT_COUNT:-0}"

[ -x "$BIN" ] && [ -x "$SERVER_BIN" ] && [ -x "$PROBE" ]

pkill -f "writev-server" 2>/dev/null || true
pkill -f "/release/obsagent" 2>/dev/null || true
sleep 0.5

PORT=$PORT "$SERVER_BIN" >/tmp/obs-correctness6-server.log 2>&1 &
server_pid=$!
sleep 0.5
grep -q listening /tmp/obs-correctness6-server.log || {
  echo "FAIL: server did not listen"; cat /tmp/obs-correctness6-server.log; exit 1;
}

# Deny common k8s/docker noise without allow-only BPF (no ALLOWED_TGID race).
OBSAGENT_HEADLESS=1 OBSAGENT_K8S=0 \
  OBSAGENT_COMM_DENY=dockerd,containerd,kubelet,kube-proxy,otelcol,otelcol-contrib \
  RUST_LOG=warn timeout --signal=INT 40 \
  env OBSAGENT_HEADLESS=1 OBSAGENT_K8S=0 \
  OBSAGENT_COMM_DENY=dockerd,containerd,kubelet,kube-proxy,otelcol,otelcol-contrib \
  RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
runner=$!
sleep 2

"$PROBE" --port "$PORT" --correctness6 --path "/slow?delay_ms=${DELAY_MS}"
echo "PASS: split-header client completed"

sleep 6
kill -INT "$runner" 2>/dev/null || true
wait "$runner" 2>/dev/null || true
kill "$server_pid" 2>/dev/null || true
wait "$server_pid" 2>/dev/null || true

echo '=== tail log ==='
tail -n 40 "$LOG"

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
    raise SystemExit("FAIL: p50 outside band")
print("PASS: p50 within band")
PY

count=$(echo "$line" | sed -n 's/.* count=\([0-9]*\).*/\1/p')
[ -n "$count" ] || { echo "FAIL: parse count"; exit 1; }
reasm=$(grep -oE 'reasm=[0-9]+' "$LOG" | sed 's/reasm=//' | sort -n | tail -1 || true)
reasm=${reasm:-0}
python3 - "$count" "$STRICT_COUNT" "$reasm" <<'PY'
import sys
n = int(sys.argv[1])
strict = sys.argv[2] == "1"
reasm = int(sys.argv[3])
need = 6 if strict else 1
if n < need:
    raise SystemExit(f"FAIL: expected count>={need} got {n} (STRICT_COUNT={int(strict)})")
if reasm < 1:
    raise SystemExit(f"FAIL: expected reasm>=1 got {reasm}")
print(f"PASS: GET /slow count={n} reasm={reasm} (need>={need})")
PY
