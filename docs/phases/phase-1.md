# Phase 1 — MVP: syscall latency tracker

Step-by-step runbook (inventory, architecture, subphases, open questions):
[phase-1-implementation-plan.md](phase-1-implementation-plan.md).

Evidence: [../testing/phase-1.tdd.md](../testing/phase-1.tdd.md).

## Checklist

- [x] Tracepoints `sys_enter/exit_connect` (and `accept4`)
- [x] HashMap entry on enter; latency + RingBuf event on exit
- [x] Socket metadata (sockaddr via `bpf_probe_read_user`; CO-RE deferred)
- [x] Tokio RingBuf consumer
- [x] Rolling 60s aggregates (p50/p95/p99, count, errors)
- [x] Ratatui live table + sparkline (TTY); headless for scripts
- [x] Overhead baseline under synthetic load → `docs/overhead.md`
- [x] Drop counter wired

## Milestone 1

Live CLI of connect/accept latency per remote endpoint from kernel probes only; measured overhead documented.

**Status:** Milestone 1 achieved for connect (smoke1 + overhead row). Accept4 programs attached; accept-only smoke still optional follow-up.
