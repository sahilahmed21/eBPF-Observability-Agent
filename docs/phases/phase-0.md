# Phase 0 — Foundations

**Gate for everything else.** If hello kprobe doesn’t load, stop.

Step-by-step runbook: [phase-0-implementation-plan.md](phase-0-implementation-plan.md).
Execution evidence: [../testing/phase-0.tdd.md](../testing/phase-0.tdd.md).

## Checklist

- [x] Dev target is Linux 5.15+ (VM/cloud preferred over WSL2) — WSL2 kernel 6.6.114.1, BTF present
- [x] `ls /sys/kernel/btf/vmlinux` succeeds
- [x] Rust stable + nightly (`rust-src`) installed
- [x] `bpf-linker` installed
- [x] `bpftool` installed — every load/unload/leak check below is a `bpftool` command
- [x] Aya template scaffolded into `ebpf/` + `agent/` + `common/` workspace
- [x] Default template kprobe loads
- [x] `aya-log` output visible
- [x] Clean unload works
- [x] `scripts/preflight.sh` and `scripts/smoke-milestone0.sh` both pass

## Milestone 0

**Achieved 2026-07-28.** Load trivial Aya kprobe → log → unload. Proves compiler → verifier →
kernel → userspace path.

## Notes

- Don’t burn a week on WSL2 BTF; switch to a VM. (Current Microsoft WSL2 kernels *do* ship BTF —
  check before assuming you need a VM.)
- Read [Aya book](https://aya-rs.dev/book/) before custom probes.
- `aya-tool` is deferred to Phase 1 — it only matters once we read kernel structs (CO-RE bindings).
- On WSL2: build as the normal user, run privileged steps via `wsl -u root`, and keep
  `CARGO_TARGET_DIR` off `/tmp` (wiped when the VM idles out).
