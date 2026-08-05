# Overhead log

Record the **same** load script every milestone (`benches/` once it exists).

| Phase | Date | Load | Agent CPU % | Agent RSS | Notes |
|---|---|---|---|---|---|
| 0 | 2026-07-28 | n/a | — | — | hello kprobe only |
| 1 | 2026-08-06 | `benches/connect-load.sh 50` (~5.9s) | 1.6% (`ps`) | ~22.5 MiB | `scripts/overhead-phase1.sh` on WSL2 Ubuntu; headless; drops=0; ~60 events/60s window. `ps` %CPU is not host-normalized — re-check with `perf` before claiming &lt;2% under heavier load. |
| 2 | | | | | + HTTP |
| 3 | | | | | + TLS uprobes |
| 4 | | | | | + map + OTLP |

**Target:** &lt;2% CPU under documented load. If exceeded, note cause and next optimization (sampling, smaller prefix, fewer attach points).

Method: `perf stat` and/or `/proc/<agent-pid>/stat` sampling vs baseline (load without agent).
