#!/usr/bin/env bash
# Milestone 0 gate: the kprobe loads, logs, and unloads without leaking.
#
# Run from the repo root, on the Linux target, after `scripts/preflight.sh` passes.
# See docs/phases/phase-0-implementation-plan.md
set -euo pipefail

BIN="${CARGO_TARGET_DIR:-target}/release/obsagent"
PROG=smoke_probe          # eBPF program name == the Rust probe fn name (kernel truncates at 15 chars)
LOG=$(mktemp)

# Loading BPF needs root. `env` is a no-op prefix for when we are already root.
if [ "$(id -u)" -eq 0 ]; then PRIV=(env); else PRIV=(sudo -E); fi
loaded() { "${PRIV[@]}" bpftool prog list | grep -c " name ${PROG} " || true; }

# This script verifies; it does not build. Keeps `cargo` off the privileged path.
[ -x "$BIN" ] || { echo "FAIL: $BIN not built - run: cargo build --release"; exit 1; }

[ "$(loaded)" -eq 0 ] || { echo "FAIL: ${PROG} already loaded (leak from an earlier run)"; exit 1; }

# try_to_wake_up fires constantly, so 8s is generous. SIGINT exercises the real shutdown path.
RUST_LOG=info timeout --signal=INT 8 "${PRIV[@]}" "$BIN" >"$LOG" 2>&1 &
runner=$!

sleep 3
[ "$(loaded)" -eq 1 ] || { echo "FAIL: program not loaded"; cat "$LOG"; exit 1; }
echo "PASS: loaded"

wait "$runner" || true

grep -q "kprobe called" "$LOG" || { echo "FAIL: no aya-log output"; cat "$LOG"; exit 1; }
echo "PASS: aya-log output visible"

[ "$(loaded)" -eq 0 ] || { echo "FAIL: still loaded after exit"; exit 1; }
echo "PASS: clean unload"

[ -z "$(ls -A /sys/fs/bpf 2>/dev/null)" ] || { echo "FAIL: leftover pins in /sys/fs/bpf"; exit 1; }
echo "PASS: no leaked pins"
