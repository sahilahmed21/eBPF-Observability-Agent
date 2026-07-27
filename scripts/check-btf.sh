#!/usr/bin/env bash
# Verify the host is usable for Aya CO-RE / BTF development (Phase 0 gate).
set -euo pipefail

echo "==> Kernel: $(uname -r)"
echo "==> OS: $(. /etc/os-release 2>/dev/null && echo "${PRETTY_NAME:-unknown}" || echo unknown)"

BTF_PATH="/sys/kernel/btf/vmlinux"
if [[ -e "$BTF_PATH" ]]; then
  echo "==> BTF: OK ($BTF_PATH)"
else
  echo "==> BTF: MISSING ($BTF_PATH not found)"
  echo "    eBPF CO-RE development needs CONFIG_DEBUG_INFO_BTF=y."
  echo "    Prefer a native Linux / cloud VM over WSL2 unless you ship a custom WSL kernel."
  exit 1
fi

if [[ -r /proc/kallsyms ]]; then
  echo "==> /proc/kallsyms: readable"
else
  echo "==> /proc/kallsyms: not readable (may need privileges later)"
fi

if command -v rustc >/dev/null 2>&1; then
  echo "==> rustc: $(rustc --version)"
else
  echo "==> rustc: not found"
fi

if command -v bpf-linker >/dev/null 2>&1; then
  echo "==> bpf-linker: $(command -v bpf-linker)"
else
  echo "==> bpf-linker: not found (install in Phase 0: cargo install bpf-linker)"
fi

echo "==> check-btf: passed"
