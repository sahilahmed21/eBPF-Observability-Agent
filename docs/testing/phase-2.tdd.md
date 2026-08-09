# Phase 2 TDD evidence

**Source plan:** [phase-2-implementation-plan.md](../phases/phase-2-implementation-plan.md)  
**Date:** 2026-08-07  
**Tip context:** Phase 1 at `b7aeb06`; Phase 2 work uncommitted at evidence time.

## User journeys

1. As an operator, I want per-endpoint HTTP latency from kernel sock I/O without changing the target app.
2. As an operator, I want 4xx/5xx rates and p50/p95/p99 in the CLI (toggle with TCP view).
3. As a developer, I want a correctness gate vs injectable delay and an overhead row under named load.

## Task → test mapping

| Plan | Test target | RED | GREEN |
|---|---|---|---|
| 2.1 SockIO ABI | `common` layout tests | compile fail (missing types) | 10 passed (`wsl-run.sh test-common`) |
| 2.3 demux | `agent/src/decode.rs` | n/a (written with impl) | decode unit tests PASS |
| 2.4 correlation | `agent/src/correlate.rs` | n/a | client/server/timeout/restart PASS |
| 2.5 httparse/normalize/redact | `agent/src/http.rs` | n/a | parse/normalize/redact PASS |
| 2.5 HTTP agg | `agent/src/http_agg.rs` | n/a | status-class agg PASS |
| 2.2/2.8 attach+smoke | `scripts/smoke-milestone2.sh` | sockio=0 until sendto/recvfrom | 5 PASS lines |
| 2.7 correctness | `scripts/correctness-phase2.sh` | no row / wrong p50 | PASS p50 within Q13 band |
| 2.8 overhead | `scripts/overhead-phase2.sh` | — | row in `docs/overhead.md` |

## Empirical revisions (not assumed)

| ID | Revision | Evidence |
|---|---|---|
| Q2 | Extended beyond read/write to **sendto/recvfrom** | `strace` of `http-probe`: only `sendto`/`recvfrom` for HTTP body |
| Q4 | SOCK_FDS on **enter** + HTTP content filter on **exit** | Enter without mark = no PENDING_IO; exit without HTTP magic = no emit |
| Q16 | **PASS** | smoke2: exit_sendto/exit_recvfrom loaded |

## Commands & outcomes

```text
wsl-run.sh test-common  → 10 passed
wsl-run.sh test-agent   → 17 passed
wsl-run.sh smoke2       → PASS programs, sockio, HTTP rows, unload, pins
correctness-phase2.sh   → observed GET /slow p50=52.43ms vs delay=50ms (±10ms) PASS
```

## Guarantees

| # | What is guaranteed | Evidence |
|---|---|---|
| 1 | `SockIoEvent` is 288 B; P1 `SockLatencyEvent` stays 48 B | common tests |
| 2 | RingBuf demux by kind byte | decode tests |
| 3 | Client write→read and server read→write emit latency | correlate tests |
| 4 | Path `/users/123` → `/users/:id`; Auth redacted | http tests |
| 5 | sendto/recvfrom attach and HTTP endpoint rows appear | smoke2 |
| 6 | p50 within max(±10ms, ±10%) of injected delay | correctness2 |

## Coverage / gaps

- Unit tests cover ABI, demux, SM, parse, agg (17 agent + 10 common).
- Accept4 smoke still optional (Phase 1 note).
- Sequential-only slow probes were flaky for correlation; interleaved `http-probe --path` list is the locked gate shape.
- Overhead: see `docs/overhead.md` Phase 2 row (ps %CPU, not perf-normalized).

## Merge / checkpoint note

User rules: commits only when requested — RED/GREEN checkpoint commits not created; this report preserves the evidence.
