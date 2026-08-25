# Phase 6 — Capture completeness

Plan: [phase-6-implementation-plan.md](phase-6-implementation-plan.md).  
North star: [VISION-95.md](VISION-95.md) (Q8, Q9, Q13, Q17).  
Evidence: [../testing/phase-6.tdd.md](../testing/phase-6.tdd.md) *(fill at close)*.

**Buys:** HTTP/1.1 from real servers (`writev`/`sendmsg`), split-header reassembly, k8s noise deny-list, IPv6 peers on the service map. Required before HTTP/2.

**Do not start Phase 7 until Milestone 6 is green.**

## Checklist

- [x] Tracepoints: `readv` / `writev` / `recvmsg` / `sendmsg` (first iovec, 256 B)
- [x] HTTP-magic OR INFLIGHT emit gate (BPF sets INFLIGHT; userspace clears on half-flush)
- [x] Userspace reassembly buffer (8 KiB / 60 s) + split-header gate
- [x] `OBSAGENT_COMM_DENY` + k8s default deny list (userspace)
- [x] IPv6 `sockaddr_in6` → expanded `SockMeta` (latency event stays 48 B / IPv4)
- [x] Testdata: writev-backed `/slow` server
- [x] `wsl-run.sh smoke6` + `correctness6`
- [x] Overhead row in `docs/overhead.md` (not the &lt;2% claim)
- [x] `wsl-run.sh` arms for `smoke6` / `correctness6`

## Milestone 6

`correctness6-writev`: writev-backed `/slow` 50 ms → p50 in band `max(±10 ms, ±10%)`. Split-header fixture produces **one** exchange. Deny-list evidence in the Phase 6 handoff.

**Status:** complete (2026-08-18). Emit gate is **HTTP-magic OR INFLIGHT** (not emit-all on SOCK_META). Reassembly key is `(tgid, fd, dir)`; flush on `\r\n\r\n`.
