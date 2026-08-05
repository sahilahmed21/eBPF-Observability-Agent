# Phase 1 — TDD evidence report

- **Source plan:** [phase-1-implementation-plan.md](../phases/phase-1-implementation-plan.md)
- **Executed:** 2026-08-06
- **Target:** WSL2 Ubuntu (`wsl -d Ubuntu`), kernel `6.6.114.1-microsoft-standard-WSL2`

## Locked decisions (user)

Q4=A IPv4 · Q5=B remote+direction · Q6=A hdrhistogram · Q7=A enter→exit · Q8=A 256KiB/8192 ·
Q9=B keep smoke_probe · Q11=A `wsl -d Ubuntu -u root`

Resolved by experiment: **Q1 PASS** (tracepoints attach/fire). **Q3** = sockaddr read (not CO-RE).

## User journeys

1. As an operator, I load the agent, generate connects, and see per-endpoint latency (TUI or headless).
2. As an operator, I see drop counts when the RingBuf cannot reserve.
3. As a developer, `cargo test` covers ABI layout + rolling aggregates; smoke scripts cover kernel path.

## Task report

### ABI (`obsagent-common`)

GREEN: `scripts/wsl-run.sh test-common` → 4 passed (`SockLatencyEvent` 48 bytes, kinds, Q8 constants).

### Aggregator (`agent/src/agg.rs`)

GREEN: `scripts/wsl-run.sh test-agent` → 5 passed (direction keying, errors, 60s window, percentiles, labels).

### Kernel + agent

Build: `scripts/wsl-run.sh build` → `Finished release`.

Milestone 0 still green: `wsl -d Ubuntu -u root` + `smoke0` → four PASS lines.

Milestone 1: `smoke1` →

```
PASS: programs loaded
PASS: connect events observed
PASS: clean unload
PASS: no leaked pins
```

## Test specification

| # | What is guaranteed | Test / command | Type | Result |
|---|---|---|---|---|
| 1 | Event ABI is 48-byte `repr(C)` | `test-common` | unit | PASS |
| 2 | Aggregates by remote+direction; 60s window; hdrhistogram p50/p95/p99 | `test-agent` | unit | PASS |
| 3 | smoke_probe still loads (Q9) | `smoke0` / `smoke1` | integration | PASS |
| 4 | connect enter/exit TPs load, emit events, unload clean | `smoke1` | integration | PASS |
| 5 | Drop map exists and is scraped | code path in agent; forced overflow not automated yet | partial | code wired |

## Coverage and gaps

- BPF programs: no unit coverage (verifier + smoke only) — intentional.
- Accept4: programs attached; smoke1 currently triggers **connect** only. Manual accept check deferred / follow-up.
- Overhead row: `scripts/overhead-phase1.sh` → ~1.6% CPU (`ps`), ~22.5 MiB RSS under `connect-load.sh 50`; drops=0 (see `docs/overhead.md`).
- Ratatui: exercised when stdout is a TTY; smoke uses headless (`OBSAGENT_HEADLESS` or non-TTY auto).

## Merge evidence

RED/GREEN for userspace: unit tests above. Kernel GREEN: `smoke-milestone1.sh`.
