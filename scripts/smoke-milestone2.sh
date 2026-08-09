#!/usr/bin/env bash
# Milestone 2 gate: read/write TPs attach; SockIO + HTTP endpoint rows; clean unload.
set -euo pipefail

# wsl-run may copy this script to /tmp; pin the repo root.
ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
BIN="${CARGO_TARGET_DIR:-target}/release/obsagent"
SERVER_BIN="${CARGO_TARGET_DIR:-target}/release/latency-server"
PROBE="${CARGO_TARGET_DIR:-target}/release/http-probe"
LOG=$(mktemp)
PORT=18080

if [ "$(id -u)" -eq 0 ]; then PRIV=(env); else PRIV=(sudo -E); fi
loaded_named() { "${PRIV[@]}" bpftool prog list | grep -c " name ${1} " || true; }

[ -x "$BIN" ] || { echo "FAIL: $BIN not built - run: cargo build --release -p obsagent"; exit 1; }
[ -x "$SERVER_BIN" ] || { echo "FAIL: $SERVER_BIN not built - run: cargo build --release -p latency-server"; exit 1; }
[ -x "$PROBE" ] || { echo "FAIL: $PROBE not built - run: cargo build --release -p http-probe"; exit 1; }

[ "$(loaded_named exit_sendto)" -eq 0 ] || {
  echo "FAIL: exit_sendto already loaded"; exit 1;
}

PORT=$PORT "$SERVER_BIN" >/tmp/obs-latency-server.log 2>&1 &
server_pid=$!
cleanup() {
  kill "$server_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true
}
trap cleanup EXIT
sleep 0.5

OBSAGENT_HEADLESS=1 OBSAGENT_SMOKE_PROBE=1 RUST_LOG=warn timeout --signal=INT 20 "${PRIV[@]}" env OBSAGENT_HEADLESS=1 OBSAGENT_SMOKE_PROBE=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
runner=$!

sleep 2
[ "$(loaded_named exit_sendto)" -ge 1 ] || {
  echo "FAIL: exit_sendto not loaded (Q16/Q2)"; cat "$LOG"; exit 1;
}
[ "$(loaded_named exit_recvfrom)" -ge 1 ] || {
  echo "FAIL: exit_recvfrom not loaded (Q16/Q2)"; cat "$LOG"; exit 1;
}
[ "$(loaded_named smoke_probe)" -ge 1 ] || {
  echo "FAIL: smoke_probe not loaded (Q15)"; cat "$LOG"; exit 1;
}
echo "PASS: programs loaded (Q16 attach OK)"

"$PROBE" --port "$PORT" --repeat 5 \
  --path /fast --path /users/123 --path '/slow?delay_ms=20' || {
  echo "FAIL: http_probe errors"; cat /tmp/obs-latency-server.log; exit 1;
}
sleep 3

grep -q 'sockio=[1-9]' "$LOG" || {
  echo "FAIL: no sockio events"; cat "$LOG"; exit 1;
}
echo "PASS: sockio events observed"

grep -E -q 'GET /(fast|users/:id|slow)' "$LOG" || {
  echo "FAIL: no HTTP endpoint rows"; cat "$LOG"; exit 1;
}
echo "PASS: HTTP endpoint rows observed"

wait "$runner" || true

[ "$(loaded_named exit_sendto)" -eq 0 ] || {
  echo "FAIL: still loaded after exit"; exit 1;
}
echo "PASS: clean unload"

[ -z "$(ls -A /sys/fs/bpf 2>/dev/null)" ] || {
  echo "FAIL: leftover pins in /sys/fs/bpf"; exit 1;
}
echo "PASS: no leaked pins"
