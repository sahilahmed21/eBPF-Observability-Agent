# Data flow

## Kernel → userspace

```
probe (kprobe / tracepoint / uprobe)
        │
        ├─ lookup/update BPF HashMap (entry timestamps, per-socket state)
        │
        └─ bpf_ringbuf_reserve / submit
                │
                ▼
         RingBuf map
                │
                ▼ (overflow)
         drop + atomic drop_count
                │
                ▼
         userspace poll (Tokio AsyncFd)
                │
                ▼
         decode common::Event
                │
                ├─ correlation
                ├─ HTTP/TLS parse
                ├─ histograms / service map
                └─ Ratatui | OTLP
```

## Event kinds (evolving)

Keep events **small and fixed-layout** (verifier + RingBuf friendly). Grow fields only when a phase needs them.

| Kind | Payload (concept) | Phase |
|---|---|---|
| `ConnectEnter` / `ConnectExit` | pid, tgid, ts, fd, ret, latency_ns, addrs | 1 |
| `AcceptExit` | same family | 1 |
| `SockIO` | pid, tgid, fd, dir (r/w), len, prefix\[256\] | 2 |
| `TlsIO` | twin of SockIO (`EventKind::TlsIo`); fd from `SSL_set_fd` map | 3 |
| `DropStats` (userspace poll of map) | dropped event count | 1+ |

Exact Rust/`aya` structs live in `common/`. Phase 3 M3: TLS-only latency into the same correlator —
no dual-plane wire-timing merge.

## Aggregation windows

- CLI: rolling **60s** window per remote / endpoint.
- Metrics export: histogram buckets continuous; scrape/reset policy decided in Phase 4 (prefer cumulative OTel histograms).

## Overhead control

- Bound prefix size (256–512 B).
- Filter non-TCP fds early in kernel when possible.
- Optional sampling rate map (tunable) before RingBuf submit under extreme load.
- Measure every milestone → `docs/overhead.md`.
