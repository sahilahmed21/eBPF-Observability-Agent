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
| 11 | 2026-08-25 | vegeta **500 RPS** HTTP/1.1 + ghz **200 RPS** gRPC, 60 s × 3; ClusterIP; `api`×3; WORKERS=32; `OBSAGENT_PROFILE=0`; WSL2 k3s | **mean ~87% of one core** (`perf -p`: 0.9 / 0.8 / 0.9 CPUs utilized) | — | Pin met (HTTP 500 @ 100%, gRPC ~200). Logs: `docs/handoff/artifacts/logs/overhead-run{1,2,3}.log`. **Not** &lt;2% — resume must say measured ~87% (or “near one core”). Footnote: eBPF probe time is on syscall CPUs, not this `perf -p`. |

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
