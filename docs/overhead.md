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

**Footnote:** eBPF programs run on the **syscall’s CPU** (apps, kubelet). That cost is **not** in the agent `perf -p` number. Do not treat cgroup `cpu.stat` as a second official headline.
