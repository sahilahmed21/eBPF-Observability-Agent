# Phase 8 — Dual-plane TLS + handshake

Plan: [phase-8-implementation-plan.md](phase-8-implementation-plan.md).  
North star: [VISION-95.md](VISION-95.md) Q5, Q6, Q7, Q19.  
Evidence: [../testing/phase-8.tdd.md](../testing/phase-8.tdd.md).

**Buys:** original v3 — TLS **content** from OpenSSL uprobes, **wire timing** from syscalls on the same `(tgid,fd)`, plus handshake duration. OpenSSL 1.1/3.x only.

**Blocked on:** Milestone 7 for h2-over-TLS dual-plane; HTTP/1.1 dual-plane can start after M6 but **ship M8 after M7** so one correctness gate covers both.

## Checklist

- [x] TLS fds emit **timing-only** sock events (`SockIoTimes`, no ciphertext prefix)
- [x] Content stays `TlsIo`; never merge two HTTP parses
- [x] Join: preceding sock stamps in 5 ms; metrics content + **labeled** wire (HTTP/1.1)
- [x] `correctness8-dual`: content p50 in band; join **hit** rate &gt; 90%; handshake required
- [x] Uprobe `SSL_do_handshake` (demo hits it; `SSL_connect`/`SSL_accept` not attached)
- [x] Histogram `tls.handshake.duration`
- [x] `tls_unmapped` counter; OpenSSL demo stays 0
- [x] Optional `OBSAGENT_TLS_SERVER=1` server-side pairing
- [x] `docs/security.md` updated (syscall timing on TLS fds)
- [x] Overhead row (no `&lt;2%` claim)

## Milestone 8

`correctness8-dual` prints both clocks; content p50 in band; join **hit** rate &gt; 90% (fallback is not a hit). Handshake p50 required in `(0, content p50)`. Wire p50 is on the `GET /slow` row, not a process-wide mix.

**Status:** PASS (2026-08-19 post-review). Uncommitted.
