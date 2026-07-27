# Architecture overview

## Goal

Zero-instrumentation observability: reconstruct HTTP/gRPC latency and a service map from kernel + TLS-library probes only. No SDKs in target apps.

## Components

| Component | Role | Lives in |
|---|---|---|
| eBPF programs | kprobes/tracepoints/uprobes; emit typed events | `ebpf/` |
| Shared types | Event layout ABI (pod/kernel agree) | `common/` |
| Agent | RingBuf drain, correlate, parse, aggregate, export | `agent/` |
| Testdata | Known-latency HTTP(S) servers | `testdata/` |
| Deploy | Container + DaemonSet | `deploy/` |

## Probe surface (by phase)

| Phase | Attach points | Events |
|---|---|---|
| 1 | `syscalls:sys_enter/exit_connect`, `accept4`; later `tcp_v4_connect` / sock fields | Connect/accept latency, 4-tuple |
| 2 | `read`/`write`/`recvfrom`/`sendto` (socket fds only) | Bounded buffer prefix (256–512 B) |
| 3 | `SSL_write` / `SSL_read` in `libssl` | Plaintext prefix + SSL* context |
| 4 | Same + cgroup/CRI identity | Service-map nodes/edges |
| S | HTTP/2 frame parse; `perf_event` stacks | Stream demux; CPU samples |

Prefer **tracepoints** over raw kprobes on syscall internals (ABI stability). Prefer **socket-layer** + CO-RE for addresses over only userspace `sockaddr` when available.

## Data path

1. Probe fires → write compact event to **RingBuf** (or drop + `drop_count` map).
2. Userspace `AsyncFd` drains RingBuf.
3. Correlation state machine keys on `(pid, fd, tuple)`.
4. HTTP parse (`httparse`) → path normalize → `hdrhistogram`.
5. Sink: Ratatui (dev) and/or OTLP (prod).

See [data-flow.md](data-flow.md).

## Design decisions (locked)

| Decision | Choice | Why |
|---|---|---|
| Kernel→user transport | RingBuf | Better mem + ordering than PerfEventArray |
| Backpressure | Drop + counter | Bound kernel memory; visible loss |
| HTTP correlation | fd state machine | No request ID in kernel |
| TLS | OpenSSL uprobes first | Plaintext before encrypt; Go crypto/tls later/limitation |
| Histograms | `hdrhistogram` | Industry-standard percentile math |
| Export | OTLP | Interops with Collector → Tempo/Prometheus/Grafana |

## Non-goals (YAGNI until asked)

- Full TCP reassembly at XDP
- Graph DB for service map (in-memory adjacency is enough)
- Perfect HTTP/1.1 pipelining pairing (document mis-pair rate)
- Supporting every TLS stack on day one
