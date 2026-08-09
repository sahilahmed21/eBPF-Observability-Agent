# Phase 3 — TLS interception

**Plan + locked Qs:** [phase-3-implementation-plan.md](phase-3-implementation-plan.md) (Q1–Q12 locked 2026-08-10; Q13–Q15 filled in execution).  
**Evidence:** [phase-3.tdd.md](../testing/phase-3.tdd.md)

## Scope (Milestone 3)

HTTP over **HTTPS/OpenSSL** only: `SSL_set_fd` → `SSL*`→fd → `SSL_read`/`SSL_write` **and** `_ex` variants →
`TlsIo` → **existing** correlator / httparse / aggregator / CLI.

**TLS-only latency** (Q2). Not a general TLS↔syscall correlation engine.

**Out of Phase 3:** Go/rustls/BoringSSL-first-class, HTTP/2, OTLP, Kubernetes, DB, dual-plane
wire timing, nested syscalls, custom BIO recovery.

## Checklist

- [x] Resolve + attach `SSL_write` / `SSL_read` **and** `SSL_write_ex` / `SSL_read_ex` on `libssl.so.3` / `1.1` (try-attach; soft-fail)
- [x] `SSL_set_fd` (+ rfd/wfd) → `SSL*`→fd map; drop TLS emit if fd unknown
- [x] Plaintext prefix via `bpf_probe_read_user` (256 B); enter-stash / exit-emit
- [x] `EventKind::TlsIo` (SockIo twin) → existing correlator → HTTP agg / CLI
- [x] Skip Phase 2 sock I/O on TLS-marked fds
- [x] Redaction helper documented; metrics-only UI; never log raw prefixes
- [x] HTTPS test service via Python stdlib ssl → system OpenSSL (self-signed; not rustls)
- [x] Security section reviewed (`docs/security.md`)
- [x] Overhead row for Phase 3
- [x] Cleartext Phase 2 still green when libssl present (`smoke2`)

## Milestone 3

Same per-endpoint breakdown as Phase 2 over OpenSSL HTTPS, with documented redaction policy.
Missing libssl must not break cleartext HTTP.
