#!/usr/bin/env bash
# Milestone 1 gate: connect latency events appear; clean unload; smoke_probe still present while run.
set -euo pipefail

BIN="${CARGO_TARGET_DIR:-target}/release/obsagent"
LOG=$(mktemp)
PROG_CONNECT=exit_connect

if [ "$(id -u)" -eq 0 ]; then PRIV=(env); else PRIV=(sudo -E); fi
loaded_named() { "${PRIV[@]}" bpftool prog list | grep -c " name ${1} " || true; }

[ -x "$BIN" ] || { echo "FAIL: $BIN not built - run: cargo build --release"; exit 1; }

[ "$(loaded_named "$PROG_CONNECT")" -eq 0 ] || {
  echo "FAIL: ${PROG_CONNECT} already loaded"; exit 1;
}

OBSAGENT_HEADLESS=1 RUST_LOG=warn timeout --signal=INT 12 "${PRIV[@]}" env OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
runner=$!

sleep 2
[ "$(loaded_named "$PROG_CONNECT")" -ge 1 ] || {
  echo "FAIL: ${PROG_CONNECT} not loaded"; cat "$LOG"; exit 1;
}
[ "$(loaded_named smoke_probe)" -ge 1 ] || {
  echo "FAIL: smoke_probe not loaded (Q9)"; cat "$LOG"; exit 1;
}
echo "PASS: programs loaded"

# Generate IPv4 connect(s).
curl -s --max-time 3 http://1.1.1.1 >/dev/null 2>&1 || true
curl -s --max-time 3 http://example.com >/dev/null 2>&1 || true
sleep 2

grep -q 'events_60s=[1-9]' "$LOG" || {
  echo "FAIL: no connect events observed"; cat "$LOG"; exit 1;
}
echo "PASS: connect events observed"

wait "$runner" || true

[ "$(loaded_named "$PROG_CONNECT")" -eq 0 ] || {
  echo "FAIL: still loaded after exit"; exit 1;
}
echo "PASS: clean unload"

[ -z "$(ls -A /sys/fs/bpf 2>/dev/null)" ] || {
  echo "FAIL: leftover pins in /sys/fs/bpf"; exit 1;
}
echo "PASS: no leaked pins"
