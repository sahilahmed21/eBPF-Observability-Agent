#!/usr/bin/env bash
# Milestone 6: vectored TPs loaded; sockio against writev-server; HTTP rows.
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
BIN="${CARGO_TARGET_DIR:-target}/release/obsagent"
SERVER_BIN="${CARGO_TARGET_DIR:-target}/release/writev-server"
PROBE="${CARGO_TARGET_DIR:-target}/release/http-probe"
LOG=$(mktemp)
PORT=18086

if [ "$(id -u)" -eq 0 ]; then PRIV=(env); else PRIV=(sudo -E); fi
loaded_named() { "${PRIV[@]}" bpftool prog list | grep -c " name ${1} " || true; }

[ -x "$BIN" ] || { echo "FAIL: $BIN not built"; exit 1; }
[ -x "$SERVER_BIN" ] || { echo "FAIL: $SERVER_BIN not built"; exit 1; }
[ -x "$PROBE" ] || { echo "FAIL: $PROBE not built"; exit 1; }

PORT=$PORT "$SERVER_BIN" >/tmp/obs-writev-server.log 2>&1 &
server_pid=$!
cleanup() {
  kill "$server_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true
}
trap cleanup EXIT
sleep 0.5

OBSAGENT_HEADLESS=1 RUST_LOG=warn timeout --signal=INT 20 "${PRIV[@]}" env OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
runner=$!

sleep 2
[ "$(loaded_named enter_writev)" -ge 1 ] || { echo "FAIL: enter_writev not loaded"; cat "$LOG"; exit 1; }
[ "$(loaded_named exit_writev)" -ge 1 ] || { echo "FAIL: exit_writev not loaded"; cat "$LOG"; exit 1; }
[ "$(loaded_named enter_sendmsg)" -ge 1 ] || { echo "FAIL: enter_sendmsg not loaded"; cat "$LOG"; exit 1; }
[ "$(loaded_named enter_readv)" -ge 1 ] || { echo "FAIL: enter_readv not loaded"; cat "$LOG"; exit 1; }
[ "$(loaded_named enter_recvmsg)" -ge 1 ] || { echo "FAIL: enter_recvmsg not loaded"; cat "$LOG"; exit 1; }
echo "PASS: vectored programs loaded"

"$PROBE" --port "$PORT" --repeat 5 --path /fast --path '/slow?delay_ms=20' || {
  echo "FAIL: http_probe errors"; cat /tmp/obs-writev-server.log; exit 1;
}
sleep 3

grep -q 'sockio=[1-9]' "$LOG" || { echo "FAIL: no sockio events"; cat "$LOG"; exit 1; }
echo "PASS: sockio events observed"

grep -E -q 'GET /(fast|slow)' "$LOG" || { echo "FAIL: no HTTP endpoint rows"; cat "$LOG"; exit 1; }
echo "PASS: HTTP endpoint rows observed"

wait "$runner" || true
echo "PASS: smoke6"
