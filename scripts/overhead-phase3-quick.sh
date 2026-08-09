#!/usr/bin/env bash
# Phase 3 quick overhead sample (Q13). Honest ps %CPU — not host-normalized &lt;2% proof.
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
SERVER="$ROOT/testdata/https-latency-server.py"
PROBE="$ROOT/testdata/https-probe.py"
PORT=18445
CERT=/tmp/obsagent-oh3.crt
KEY=/tmp/obsagent-oh3.key
LOG=/tmp/obs-overhead3.log

openssl req -x509 -newkey rsa:2048 -keyout "$KEY" -out "$CERT" -days 1 -nodes \
  -subj "/CN=localhost" >/dev/null 2>&1

pkill -f "https-latency-server.py" 2>/dev/null || true
pkill -f "/release/obsagent" 2>/dev/null || true
sleep 0.5

PORT=$PORT TLS_CERT=$CERT TLS_KEY=$KEY python3 "$SERVER" >/tmp/obs-oh3-server.log 2>&1 &
server_pid=$!
sleep 0.5

OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
agent_pid=$!
sleep 2

samples=()
for i in $(seq 1 8); do
  python3 "$PROBE" --port "$PORT" --repeat 3 \
    --path /fast --path /users/1 --path '/slow?delay_ms=10' >/dev/null || true
  cpu=$(ps -p "$agent_pid" -o %cpu= | tr -d ' ')
  rss=$(ps -p "$agent_pid" -o rss= | tr -d ' ')
  samples+=("$cpu")
  echo "round=$i cpu=${cpu}% rss_kib=$rss"
  sleep 0.5
done

kill -INT "$agent_pid" 2>/dev/null || true
wait "$agent_pid" 2>/dev/null || true
kill "$server_pid" 2>/dev/null || true

python3 - "${samples[@]}" <<'PY'
import sys
vals = [float(x) for x in sys.argv[1:]]
avg = sum(vals) / len(vals)
print(f"samples={vals}")
print(f"avg_ps_cpu={avg:.2f}%")
print("NOTE: ps %CPU under short HTTPS burst; not full Q13 32x30s; not host-normalized <2% claim")
PY
grep -E 'tlsio=|drops=' "$LOG" | tail -n 3 || true
