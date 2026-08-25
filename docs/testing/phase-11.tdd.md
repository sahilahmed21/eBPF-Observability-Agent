# Phase 11 TDD evidence

**Source plan:** [phase-11-implementation-plan.md](../phases/phase-11-implementation-plan.md)  
**Status:** unit tests green; **11.4 headline measured** (WSL2 k3s 2026-08-25). &lt;2% wording **illegal**.

## Task → test mapping

| Plan | Test target | GREEN |
|---|---|---|
| 11.2 keep_io | `sample::keep_io_n10_is_exact_tenth_on_sequential_rnd` | PASS (`cargo test -p obsagent` 151 tests, 2026-08-20) |
| 11.2 sticky model | `sample::many_fds_n10_keep_about_one_tenth` | PASS — many fds, not 10× fewer I/O on one fd |
| 11.2 auto N | `sample::auto_doubles_on_drops_and_caps` / `auto_halves_after_30_quiet_ticks` | PASS |
| 11.2 flags | `obsagent_common::sock_meta_sample_flags_do_not_clear_addr` | PASS (`cargo test -p obsagent-common --features user`) |
| 11.2 unmarked | `keep_from_lookup(None) == Unmarked`; merge preserves sample bits | same test |
| 11.2 gauge | `export::bpf_sample_n_gauge_is_independent_of_trace_sample` | PASS |
| 11.2 pin parse | `sample::env_pin_rejects_zero` (`parse_bpf_sample_n`) | PASS |
| 11.2b deny proc | `filter::denied_tgids_from_fake_proc` (leaders + `status` Tgid) | PASS |
| 11.2b allow proc | `filter::allowed_tgids_from_fake_proc_allow_only` | PASS |
| 11.2b delta | `filter::tgid_map_delta_remove_then_insert` | PASS |
| 11.3 overload | `scripts/overload-vision95.sh` | **PASS** 2026-08-25 — `drops=68068` `sample_n=2` RSS flat; log `../handoff/artifacts/logs/overload-vision95.log` |
| 11.4 headline | `perf stat` × 3 | **PASS** pin 500+200; mean ~87% of one core |

## Results

| Run | Agent CPU % (one core) | RSS | drops | sample_n | Date |
|---|---|---|---|---|---|
| 1 | **90%** (0.9 CPUs utilized) | — | — | 1 (`PROFILE=0`) | 2026-08-25 — `overhead-run1.log` |
| 2 | **80%** (0.8) | — | — | 1 | 2026-08-25 — `overhead-run2.log` |
| 3 | **90%** (0.9) | — | — | 1 | 2026-08-25 — `overhead-run3.log` |
| **mean** | **~87%** | — | — | — | |

**Load:** ClusterIP `api` + `api-grpc`; `api` replicas=3; vegeta WORKERS=32; HTTP 500/s 100% success; ghz ~200 RPS.

**2% wording:** **illegal**. Replacement number: **~87% of one core** (agent `perf -p` only; eBPF probe time on syscall CPUs excluded).

