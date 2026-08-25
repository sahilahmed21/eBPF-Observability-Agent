#!/usr/bin/env bash
# Phase 8: HTTPS content p50 in band, join rate > 90%, wire and handshake clocks.
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
SERVER="$ROOT/testdata/https-latency-server.py"
PROBE="$ROOT/testdata/https-probe.py"
LOG=/tmp/obs-correctness8.log
PORT=18448
DELAY_MS=50
CERT=/tmp/obsagent-tls8.crt
KEY=/tmp/obsagent-tls8.key

[ -x "$BIN" ] && [ -f "$SERVER" ] && [ -f "$PROBE" ]

pkill -f "https-latency-server.py" 2>/dev/null || true
pkill -f "/release/obsagent" 2>/dev/null || true
sleep 0.5

openssl req -x509 -newkey rsa:2048 -keyout "$KEY" -out "$CERT" -days 1 -nodes \
  -subj "/CN=localhost" >/dev/null 2>&1

PORT=$PORT TLS_CERT=$CERT TLS_KEY=$KEY python3 "$SERVER" >/tmp/obs-correctness8-server.log 2>&1 &
server_pid=$!
sleep 0.5
grep -q listening /tmp/obs-correctness8-server.log || {
  echo "FAIL: server did not listen"; cat /tmp/obs-correctness8-server.log; exit 1;
}

OBSAGENT_HEADLESS=1 RUST_LOG=warn timeout --signal=INT 30 \
  env OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
runner=$!
for i in $(seq 1 40); do
  grep -q "obsagent ready" "$LOG" 2>/dev/null && break
  sleep 0.25
done
grep -q "obsagent ready" "$LOG" || { echo "FAIL: agent did not become ready"; cat "$LOG"; exit 1; }

python3 "$PROBE" --port "$PORT" --repeat 5 \
  --path "/slow?delay_ms=${DELAY_MS}"

sleep 4
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

summary=$(grep -E 'join_hit=' "$LOG" | tail -n 1 || true)
[ -n "$summary" ] || { echo "FAIL: no join summary"; exit 1; }
echo "summary: $summary"

python3 - "$DELAY_MS" "$p50" "$summary" "$line" <<'PY'
import re, sys
delay = float(sys.argv[1])
p50 = float(sys.argv[2])
summary = sys.argv[3]
line = sys.argv[4]
tol = max(10.0, delay * 0.10)
lo, hi = delay - tol, delay + tol
print(f"delay={delay}ms content_p50={p50}ms band=[{lo},{hi}]")
if not (lo <= p50 <= hi):
    raise SystemExit("FAIL: content p50 outside band")

def grab(name):
    m = re.search(rf"{name}=([0-9.]+)", summary)
    if not m:
        raise SystemExit(f"FAIL: missing {name}")
    return m.group(1)

hit = int(grab("join_hit"))
miss = int(grab("join_miss"))
fb = int(float(grab("join_fb"))) if "join_fb=" in summary else 0
total = hit + fb + miss
if total == 0:
    raise SystemExit("FAIL: no join attempts")
rate = hit / total
print(f"join hit={hit} fb={fb} miss={miss} hit_rate={rate:.0%}")
if rate <= 0.90:
    raise SystemExit("FAIL: join hit rate <= 90%")

if "wire_p50=-" in line or "wire_p50=0.00ms" in line:
    raise SystemExit("FAIL: wire_p50 missing on GET /slow row")
wm = re.search(r"wire_p50=([0-9.]+)ms", line)
if not wm:
    raise SystemExit("FAIL: parse wire_p50 on GET /slow row")
wire = float(wm.group(1))
if wire <= 0:
    raise SystemExit("FAIL: wire_p50 <= 0")
if wire > p50 + 20:
    raise SystemExit(f"FAIL: wire {wire}ms > content {p50}+20")

if "tls_unmapped=" in summary:
    um = int(grab("tls_unmapped"))
    if um != 0:
        raise SystemExit(f"FAIL: tls_unmapped={um} want 0")

if "hs_p50=-" in summary:
    raise SystemExit("FAIL: handshake p50 missing")
hs_m = re.search(r"hs_p50=([0-9.]+)ms", summary)
if not hs_m:
    raise SystemExit("FAIL: parse hs_p50")
hs = float(hs_m.group(1))
print(f"handshake p50={hs}ms")
if hs <= 0 or hs >= p50:
    raise SystemExit("FAIL: handshake p50 not in (0, content p50)")

print("PASS: content p50 in band, join hit > 90%, wire on /slow, handshake present")
PY
