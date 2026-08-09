# Phase 3 TDD evidence

**Source plan:** [phase-3-implementation-plan.md](../phases/phase-3-implementation-plan.md)  
**Date:** 2026-08-10

## User journeys

1. As an operator, I want OpenSSL HTTPS traffic to produce the same HTTP endpoint metrics as Phase 2 cleartext.
2. As an operator, I want missing libssl to leave cleartext HTTP working (soft-fail).
3. As a security reviewer, I want metrics-only UI and no raw prefix logging.

## Task → evidence

| Task | Command / test | Result |
|---|---|---|
| 3.1 TlsIo ABI | `wsl-run.sh test-common` | 12 passed (TlsIo twin, PendingTls 32 B) |
| 3.2/3.3 decode + wire | `wsl-run.sh test-agent` | 21 passed (`decodes_tls_io`, correlator reuse) |
| Q15 attach | `wsl-run.sh smoke3` | PASS `exit_ssl_write_ex` + `enter_ssl_set_fd` |
| TLS HTTP rows | smoke3 | PASS `tlsio=` + `GET /…` rows |
| Q10 correctness | `wsl-run.sh correctness3` | PASS p50≈50.95ms vs delay=50ms |
| P2 regression | `wsl-run.sh smoke2` | PASS |
| Q13 overhead | `overhead-phase3-quick.sh` | see `docs/overhead.md` |

## Empirical revision (recorded)

CPython 3.14 `_ssl` imports **`SSL_write_ex` / `SSL_read_ex`** (and `SSL_set_fd`), not `SSL_write` / `SSL_read`.
Milestone 3 attaches both classic and `_ex` symbols. Testdata is Python stdlib ssl (system libssl), not rustls.

## Guarantees

| # | Guarantee | Evidence |
|---|---|---|
| 1 | `EventKind::TlsIo=4`; layout twin of SockIo (288 B) | common tests |
| 2 | Decode demuxes TlsIo separately | `decode::tests::decodes_tls_io` |
| 3 | HTTPS OpenSSL traffic → HTTP agg rows | smoke3 / correctness3 |
| 4 | Soft-fail without libssl does not block cleartext path | attach warns; smoke2 still green |
| 5 | Redact helper still off hot path; metrics-only | http tests + UI copy |

## Gaps

- Dual-plane wire timing: out of M3 (Q2).
- Go/rustls/BoringSSL: unsupported.
- Full Q13 long load: not claimed; quick sample only.
- `SSL_free` map cleanup: not hooked (stale SSL_FD entries until map pressure).
