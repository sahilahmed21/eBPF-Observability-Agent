# Data flow

## Kernel → userspace event path

```
┌─────────────┐     HashMap (pending)      ┌─────────────┐
│ Probe entry │ ─────────────────────────► │ Probe exit  │
│ (timestamp) │                            │ (delta, fd) │
└─────────────┘                            └──────┬──────┘
                                                  │
                                                  │ RingBuf reserve / submit
                                                  ▼
                                           ┌─────────────┐
                                           │  RingBuf    │◄── drop_counter map
                                           └──────┬──────┘
                                                  │
                     AsyncFd / Tokio poll         │
                                                  ▼
                                           ┌─────────────┐
                                           │  consumer   │
                                           └──────┬──────┘
                                                  │
                    ┌─────────────────────────────┼─────────────────────────────┐
                    ▼                             ▼                             ▼
             correlation FSM              HTTP / TLS parse               identity resolve
             (per pid+fd)                 (httparse / SSL)               (cgroup → pod)
                    │                             │                             │
                    └─────────────────────────────┼─────────────────────────────┘
                                                  ▼
                                           aggregators
                                      (hdrhistogram, rates)
                                                  │
                              ┌───────────────────┴───────────────────┐
                              ▼                                       ▼
                         dashboard                               otel exporter
                         (Ratatui)                               (OTLP gRPC)
```

## Event families (planned)

All events live in `common::events` as `#[repr(C)]` structs with a versioned `event_type` discriminant so userspace can evolve parsers safely.

| `event_type` | Phase | Payload (conceptual) |
|--------------|-------|----------------------|
| `ConnectLatency` | 1 | pid, comm, fd, saddr/daddr, sport/dport, latency_ns, ret |
| `AcceptLatency` | 1 | same family for accept4 |
| `SockIoPrefix` | 2 | pid, fd, direction (read/write), len, bytes[N] |
| `TlsIoPrefix` | 3 | pid, tid, ssl_ptr, direction, len, bytes[N] |
| `DropStats` | 1+ | periodic or on-read map snapshot of lost events |

Exact field layouts land with Phase 1 code; docs stay conceptual until then.

## Aggregation windows

- **CLI default:** rolling 60s window per remote endpoint / HTTP route.
- **OTel metrics:** cumulative or delta histograms exported on a fixed interval (e.g. 15s).
- **Traces:** synthetic spans emitted when a request/response pair completes (or times out).

## Backpressure

If userspace is slow:

1. Kernel `bpf_ringbuf_reserve` fails → increment `drop_counter` → continue.
2. Userspace may also apply secondary sampling on high-cardinality paths.
3. Buffer size increases are a **bounded delay** of the problem, not the primary fix.

See [../design-notes/ring-buffer-backpressure.md](../design-notes/ring-buffer-backpressure.md).
