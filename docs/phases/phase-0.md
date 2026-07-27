# Phase 0 — Foundations

**Gate for everything else.** If hello kprobe doesn’t load, stop.

## Checklist

- [ ] Dev target is Linux 5.15+ (VM/cloud preferred over WSL2)
- [ ] `ls /sys/kernel/btf/vmlinux` succeeds
- [ ] Rust stable + nightly (`rust-src`) installed
- [ ] `bpf-linker` installed; `aya-tool` available
- [ ] Aya template scaffolded into `ebpf/` + `agent/` + `common/` workspace
- [ ] Default template kprobe loads
- [ ] `aya-log` output visible
- [ ] Clean unload works

## Milestone 0

Load trivial Aya kprobe → log → unload. Proves compiler → verifier → kernel → userspace path.

## Notes

- Don’t burn a week on WSL2 BTF; switch to a VM.
- Read [Aya book](https://aya-rs.dev/book/) before custom probes.
