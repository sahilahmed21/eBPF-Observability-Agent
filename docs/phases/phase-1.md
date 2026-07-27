# Phase 1 — MVP: syscall latency tracker

## Checklist

- [ ] Tracepoints `sys_enter/exit_connect` (and `accept4`)
- [ ] HashMap entry on enter; latency + RingBuf event on exit
- [ ] Socket metadata (prefer sock-layer CO-RE; else `sockaddr` via `bpf_probe_read_user`)
- [ ] Tokio RingBuf consumer
- [ ] Rolling 60s aggregates (p50/p95/p99, count, errors)
- [ ] Ratatui live table + sparkline
- [ ] Overhead baseline under synthetic load → `docs/overhead.md`
- [ ] Drop counter wired

## Milestone 1

Live CLI of connect/accept latency per remote endpoint from kernel probes only; measured overhead documented.
