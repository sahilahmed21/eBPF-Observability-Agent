# Overhead log

Record the **same** load script every milestone (`benches/` once it exists).

| Phase | Date | Load | Agent CPU % | Agent RSS | Notes |
|---|---|---|---|---|---|
| 0 | | | | | hello kprobe only |
| 1 | | | | | connect/accept MVP |
| 2 | | | | | + HTTP |
| 3 | | | | | + TLS uprobes |
| 4 | | | | | + map + OTLP |

**Target:** &lt;2% CPU under documented load. If exceeded, note cause and next optimization (sampling, smaller prefix, fewer attach points).

Method: `perf stat` and/or `/proc/<agent-pid>/stat` sampling vs baseline (load without agent).
