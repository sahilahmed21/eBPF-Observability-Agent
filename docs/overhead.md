# Overhead log

Record the **same** load script every milestone (`benches/` once it exists).

| Phase | Date | Load | Agent CPU % | Agent RSS | Notes |
|---|---|---|---|---|---|
| 0 | 2026-07-28 | n/a | — | — | hello kprobe only |
| 1 | 2026-08-06 | `benches/connect-load.sh 50` (~5.9s) | 1.6% (`ps`) | ~22.5 MiB | `scripts/overhead-phase1.sh` on WSL2 Ubuntu; headless; drops=0; ~60 events/60s window. `ps` %CPU is not host-normalized — re-check with `perf` before claiming &lt;2% under heavier load. |
| 2 | 2026-08-07 | `scripts/overhead-phase2-quick.sh` (8 rounds × `http-probe` 3 paths) | 10.7% (`ps` avg) | ~22.6 MiB | drops=0; http_60s≈42 under sample. **Not** the full Q12 32×30s harness — aspirational &lt;2% unmet on this burst. Post-review: SOCK_FDS enforced on enter + `smoke_probe` opt-in (default off); re-sample before claiming improvement. `ps` %CPU not host-normalized. |
| 3 | 2026-08-10 | `scripts/overhead-phase3-quick.sh` (8 rounds × Python HTTPS probe 3 paths) | 3.5% (`ps` avg) | ~22.6 MiB | drops=0; tlsio under sample; sockio=0 on TLS fds (Q8). Quick burst only — not host-normalized &lt;2% proof. |
| 4 | | | | | + map + OTLP |
| 8 | 2026-08-19 | `correctness8-dual` (5× HTTPS `/slow` 50 ms) | not sampled (`ps`) | ~same | `socktimes=180` `tlsio=21` `join=100%` `hs_p50=2.93ms`. Extra RingBuf records vs M3. **Not** Vision-95 `&lt;2%`. |
| 11 | 2026-08-25 | vegeta **500 RPS** HTTP/1.1 + ghz **200 RPS** gRPC, 60 s × 3; ClusterIP; `api`×3; WORKERS=32; `OBSAGENT_PROFILE=0`; WSL2 k3s | **~87% of one core (RETRACTED 2026-11-10: not reproduced, cause not found; see the ladder below)** (`perf -p`: 0.9 / 0.8 / 0.9 CPUs utilized) | — | Pin met (HTTP 500 @ 100%, gRPC ~200). Logs: `docs/handoff/artifacts/logs/overhead-run{1,2,3}.log`. **Not** &lt;2% — resume must say measured ~87% (or “near one core”). Footnote: eBPF probe time is on syscall CPUs, not this `perf -p`. |

**Target:** &lt;2% CPU under documented load. If exceeded, note cause and next optimization (sampling, smaller prefix, fewer attach points).

**Method (Phase 11):** `perf stat -p <agent_pid>` `task-clock`; % of one core = `CPUs utilized × 100`. Mean of 3 runs.

## Phase C re-measurement (2026-11-09, WSL2, loopback, no k3s)

Executed with `perf stat -p <agent>` (15 s window), `perf record -F 999`, and `kernel.bpf_stats_enabled=1` + `bpftool prog show`. Agent started with `OBSAGENT_K8S=0`, OTLP off. Load: vegeta 500 rps HTTP/1.1 `/fast` and/or ghz 200 rps gRPC (50 conns) against `testdata` servers. `perf stat` and `bpf_stats` are single windows; run-to-run spread was ~14–21% of one core for the combined load.

| Load | Agent userspace CPU (one core = 100%) | Per request |
|---|---|---|
| idle (agent only) | ~1.3% (kernel `/proc` reads: `proc_pid_status`, `kernfs_*`, from the 1 Hz sweep) | — |
| HTTP/1.1 500 rps | ~9.8% (1.46 s / 15 s), 916 M instructions | ~122 k instructions |
| gRPC 200 rps | ~7.8% (1.17 s / 15 s) | — |
| HTTP 500 + gRPC 200 | ~14–21% (2.06–3.09 s / 15 s) | ~150–230 k instructions |

- **Pure pipeline is cheap.** `obsagent replay` of 250 000 recorded records (30 000 exchanges) = ~650 M instructions, ~2 600 per record, ~21 600 per exchange (HTTP/1.1 only, decode + reassemble + correlate + parse; no aggregation, export, identity, or locks). That is ~15% of the ~150 k instructions per request measured live.
- **Live profile is flat.** No function above ~4%; top entries are `malloc`, tokio scheduling, and kernel `bpf()` map syscalls (`htab_map_delete_elem`, from `clear_inflight`).
- **In-kernel BPF time is as large as the agent.** Combined load, whole run (~28 s): `exit_write` 15.9 µs/call (58.9 k calls), `exit_writev` 33 µs/call, `exit_recvfrom` 8.3 µs/call, `exit_read` 2 µs/call; all probes together ≈ 2.8 s. This is charged to the application's syscall CPU, not to the agent's `perf -p`.
- **Rejected hypothesis.** Reading only `prefix_len` bytes and dropping the 256-iteration zero loop in `emit_io_kind` did not change per-call cost (`exit_write` 17.0 µs, `exit_writev` 34.7 µs vs 15.9 / 33.0); reverted. Untested: RingBuf wakeup per event (`submit(0)` wakes the consumer when it has caught up).
- **The August ~87% is not reproduced here** (≈14–21% for a similar load). Differences not isolated: k3s node (many other processes in the 1 Hz `/proc` sweep and more captured traffic), `OBSAGENT_K8S=1`, OTLP/trace export on, 3 `api` replicas via ClusterIP. Next experiment: rerun this script on the k3s node and compare `events` per second by `comm`.

**Footnote:** eBPF programs run on the **syscall’s CPU** (apps, kubelet). That cost is **not** in the agent `perf -p` number. Do not treat cgroup `cpu.stat` as a second official headline.

## k3s re-measurement and probe costs (2026-11-10)

**k3s node, HTTP only.** Agent on the k3s node (WSL2) with `OBSAGENT_K8S=1`, OTLP off, vegeta 500 rps / 30 s to the `api` ClusterIP, `perf stat -p`: idle ~0.7% of one core, under load ~10.6%. This matches the plain-WSL2 HTTP number (~9.8%), so the k3s node and the K8S default deny-list do not explain the August ~87%. What this run did not include, and August did: 200 rps gRPC (`frontend-grpc` is in CrashLoopBackOff on this cluster), OTLP export to the collector, `WORKERS=32`. The gap remains unexplained; the remaining suspects are gRPC/HTTP/2 decode cost and OTLP export under load.

**`OBSAGENT_FILEOPS` (openat/execve probes) is expensive on open-heavy workloads.** Worst-case load: 4 processes x 300 000 `open()`+`close()` each (about 1.2 M opens in ~1 s), plain WSL2, agent with `OBSAGENT_K8S=0`, three runs each:

| Configuration | Slowest process wall time (3 runs) | Agent CPU over a 20 s window (3 runs) |
|---|---|---|
| no agent | 0.94, 0.89, 0.89 s | n/a |
| agent, probes off (`OBSAGENT_FILEOPS` unset or `0`) | 1.15, 1.14, 1.11 s | 0.13-0.14 s (~0.7%) |
| agent, `OBSAGENT_FILEOPS=1` | 1.65, 1.64, 1.60 s | 1.02-1.10 s (~5.1-5.5%) |

The probes make this loop about 44% slower than the agent without them (1.63 s vs 1.13 s mean) and about 80% slower than no agent. A preflight check before every run looked for leftover `obsagent` and load-generator processes; it found a still-exiting `obsagent` from the previous run at the start of 4 of the 9 runs (the agent takes more than 1 s to exit after `SIGINT`) and killed it before measuring.

`bpf_stats` for `enter_openat`: 2.15 s over 1.22 M calls, about 1.76 us per `open`; `enter_execve` is negligible at these counts. Every open emits one RingBuf record, so the agent also pays to drain and policy-check each one. This is a deliberately open-heavy loop, not a typical service, but it shows the probes should be scoped (`OBSAGENT_COMM_ALLOW`) rather than run node-wide. `OBSAGENT_FILEOPS=0` now means off (before, any set value attached the probes).

## Retraction of the August ~87% (2026-11-10)

The August figure (~87% of one core) is **retracted as a headline**. It could not be reproduced, and no named cause was found. What was measured instead, on the same WSL2 k3s node, agent on the node with `OBSAGENT_K8S=1`, vegeta 500 rps HTTP/1.1 to the `api` ClusterIP, 30 s windows, one or two runs per config:

| Config (conditions added one at a time) | Agent CPU, % of one core | In-kernel BPF time, % of one core | Runs |
|---|---|---|---|
| A: HTTP 500 rps, `WORKERS=64` | 18.0, 11.0 | 12.4, 8.3 | 2 |
| B: A + gRPC 200 rps (`ghz`, 50 conns, direct to the grpcbin ClusterIP) | 22.6, 14.3 | 17.3, 12.0 | 2 |
| C: B + OTLP export to the in-cluster collector | 20.4, 14.9 | 13.6, 12.3 | 2 |
| D: C + `WORKERS=32` (the August setting) | 14.1 | 11.0 | 1 |

- **Measured range: agent 11-23% of one core; in-kernel BPF time a further 8-17%; together about 19-40%.** The BPF time is the `bpf_stats` run time summed over every BPF program on the node, so it can include programs other than the agent's. It is charged to the applications' syscall CPUs, not to the agent process.
- **No single added condition moves the number far**: gRPC adds about 3-4 points, OTLP and `WORKERS=32` add nothing measurable at this noise level. The highest single run (rep 1 of B, 22.6% agent + 17.3% kernel) is still under half of 87%.
- **The two reps differ by up to 7 points** (A: 18.0 vs 11.0). In the first rep, A (`drops=3266`) and C (`drops=241`, sticky sampling at `sample_n=8`) had ring-buffer drops; B did not; every second-rep run had `drops=0`. I did not isolate why the first rep was higher. Treat these as a range, not a mean. Config D has one run; a third rep of each config was killed by the host's low-memory guard and was not rerun.
- **Not tested**: 60 s runs, 3 replicas of `api` behind the ClusterIP with load spread across them as in August, and the August binary. If the 87% was real it came from something not in this ladder (an older build, a different node state, or the August measurement window including start-up); I cannot say which.
- **Run lengths and tools**: `perf stat -p <agent>` `task-clock` over the 30 s load window; achieved rates were HTTP 100% success and gRPC 199.9-200 rps in every run.

The README no longer states 87% as a result.
