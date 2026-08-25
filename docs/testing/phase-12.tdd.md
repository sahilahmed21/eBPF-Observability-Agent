# Phase 12 TDD evidence

**Source plan:** [phase-12-implementation-plan.md](../phases/phase-12-implementation-plan.md)  
**Status:** complete — claim lock 10/10 (2026-08-25).

## User journeys

1. As an operator with `OBSAGENT_PROFILE=1` and no OTLP endpoint, profile join runs and `prof_hit` increments.
2. As an operator with trace sampling skipping a span, profile join still runs for slow spans.
3. As an operator, perf attach failure disables profiles without killing the agent.
4. As CI, `correctness12` validates join hit rate via headless `prof_hit=` counter.

## Task → test mapping

| Plan / fix | Test target | GREEN |
|---|---|---|
| 12.3 join decoupled from OTLP | `export::tests::profile_join_runs_without_otlp_export` | PASS |
| 12.3 join decoupled from trace sampling | `export::tests::profile_join_runs_when_trace_not_sampled` | PASS |
| 12.3 frames on exported span | `export::tests::profile_join_attaches_frames_to_exported_span` | PASS |
| 12.1 stacks | sample rate with PROFILE=1 | ~99 Hz (integration, VM) |
| 12.2 symbols | `slow_handler_sleep` | integration gate |
| 12.3 join | `wsl-run.sh correctness12` | **PASS** `prof_hit=11/20` |
| 12.4 verifier | `docs/verifier-rejection-log.md` | **PASS** — 2 stack-limit pastes (Q18) |
| 12.5 claim lock | `docs/handoff/SESSION-VISION-95.md` | **10/10 PASS** |

## Test specification

| # | What is guaranteed | Test file or command | Test type | Result | Evidence |
|---|--------------------|----------------------|-----------|--------|----------|
| 1 | Profile join runs when OTLP export is disabled | `export::tests::profile_join_runs_without_otlp_export` | unit | PASS | `wsl-run.sh test-agent` |
| 2 | Profile join runs when trace sampling skips the span | `export::tests::profile_join_runs_when_trace_not_sampled` | unit | PASS | `wsl-run.sh test-agent` |
| 3 | Joined frames appear on exported spans when OTLP+sampled | `export::tests::profile_join_attaches_frames_to_exported_span` | unit | PASS | `wsl-run.sh test-agent` |
| 4 | Existing profile store join math unchanged | `profile::tests::*` | unit | PASS | `wsl-run.sh test-agent` |
| 5 | OTLP profile event JSON shape | `trace_export::tests::span_json_includes_profile_event` | unit | PASS | `wsl-run.sh test-agent` |
| 6 | Stack sample layout stable | `common` `stack_sample_event_layout` | unit | PASS | `wsl-run.sh test-common` |
| 7 | End-to-end join hit rate on `/slow` | `wsl-run.sh correctness12` | integration | **PASS** | `prof_hit=11/20`; log `../handoff/artifacts/logs/correctness12.log` |

## RED → GREEN (review fixes)

| Stage | Command | Outcome |
|---|---|---|
| RED | `wsl-run.sh test-agent` (new profile join tests) | **FAIL** — `profile_join_hits` stayed 0 (join gated on OTLP `enabled`) |
| GREEN | same after `try_profile_join` refactor | **PASS** — 160 agent tests |
| GREEN | `wsl-run.sh test-common` | **PASS** — 21 tests |

## Results

| Command | Result | Date |
|---|---|---|
| `wsl-run.sh test-common` | PASS — 21 tests | 2026-08-20 |
| `wsl-run.sh test-agent` | PASS — 160 tests (+3 review-fix tests) | 2026-08-20 |
| `wsl-run.sh build` | PASS (prior run) | 2026-08-20 |
| `wsl-run.sh correctness12` | **PASS** — `prof_hit=11`/20; busy-wait `slow_handler_sleep` (sleep was off-CPU) | 2026-08-25 |
| verifier Q18 force | **PASS** — `vrej-build-stack600.log` + `vrej-build-getstack600.log` | 2026-08-25 |
| Claim lock rows 1–10 | **10/10 PASS** — [SESSION-VISION-95.md](../handoff/SESSION-VISION-95.md) | 2026-08-25 |

## Coverage and known gaps

- Unit tests cover profile join wiring decoupled from OTLP and trace sampling.
- Not covered in unit tests: perf attach soft-fail (requires BPF runtime), blazesym symbolization, BPF-side stack sampling rate.
- Integration gate `correctness12` updated to parse `prof_hit=` from headless output (removed demo-specific `PROFILE_JOIN hit` grep).

## Claim lock

**10/10 PASS** — [SESSION-VISION-95.md](../handoff/SESSION-VISION-95.md). Overhead wording: **~87% of one core**.
