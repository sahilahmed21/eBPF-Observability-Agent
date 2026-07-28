#!/usr/bin/env bash
# Phase 0 environment gate. Exits non-zero if anything required is missing.
#
# Reused as the Phase 4 DaemonSet init check and as the CI environment assertion.
# See docs/phases/phase-0-implementation-plan.md
set -uo pipefail

fail=0
pass() { printf 'PASS  %s\n' "$1"; }
warn() { printf 'WARN  %s\n' "$1"; }
bad()  { printf 'FAIL  %s\n' "$1"; fail=1; }

# --- kernel ---
kver=$(uname -r)
if [ "$(printf '%s\n5.15\n' "${kver%%-*}" | sort -V | head -1)" = "5.15" ]; then
  pass "kernel $kver (>= 5.15)"
else
  bad "kernel $kver is below the 5.15 floor"
fi

# --- hard gate: BTF ---
if [ -r /sys/kernel/btf/vmlinux ]; then
  pass "BTF at /sys/kernel/btf/vmlinux"
else
  bad "no /sys/kernel/btf/vmlinux - CO-RE and aya-log cannot work. Change environment."
fi

# --- attach surface ---
if [ -r /sys/bus/event_source/devices/kprobe/type ]; then
  pass "perf-based kprobe attach available"
else
  warn "no perf kprobe PMU; aya will fall back to legacy tracefs kprobe_events (leaks on crash)"
fi

# --- things that turn into EPERM later ---
lockdown=$(cat /sys/kernel/security/lockdown 2>/dev/null || echo "none")
case "$lockdown" in
  *"[confidentiality]"*) bad "kernel lockdown=confidentiality blocks BPF" ;;
  none) pass "kernel lockdown not enforced" ;;
  *)    warn "kernel lockdown: $lockdown" ;;
esac
warn "perf_event_paranoid=$(sysctl -n kernel.perf_event_paranoid 2>/dev/null || echo '?')"
warn "unprivileged_bpf_disabled=$(sysctl -n kernel.unprivileged_bpf_disabled 2>/dev/null || echo '?')"

# --- toolchain ---
for tool in bpf-linker bpftool cargo rustup; do
  if command -v "$tool" >/dev/null 2>&1; then pass "$tool on PATH"; else bad "$tool missing"; fi
done
rustup toolchain list 2>/dev/null | grep -q '^stable'  && pass "stable toolchain"  || bad "stable toolchain missing"
rustup toolchain list 2>/dev/null | grep -q '^nightly' && pass "nightly toolchain" || bad "nightly toolchain missing"
if rustup component list --toolchain nightly 2>/dev/null | grep -q 'rust-src.*installed'; then
  pass "nightly rust-src (required by -Z build-std=core)"
else
  bad "nightly rust-src missing: rustup component add rust-src --toolchain nightly"
fi

[ "$fail" -eq 0 ] && echo "preflight OK" || echo "preflight FAILED"
exit "$fail"
