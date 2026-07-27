# Architecture overview

## Goal

Observe application-level HTTP/gRPC behavior **without instrumenting applications**, by attaching eBPF programs to kernel syscalls / socket paths and (for TLS) to userspace SSL library entry points.

## Components

| Component | Runtime | Responsibility |
|-----------|---------|----------------|
| `ebpf/` | Kernel (BPF VM) | Probes, bounded capture, maps, RingBuf emit |
| `common/` | Shared | `#[repr(C)]` event/key layouts both sides agree on |
| `agent/` | Userspace | Load/attach, drain events, correlate, aggregate, export/UI |
| `xtask/` | Build host | Compile eBPF + userspace in the right order |
| `deploy/` | Cluster | DaemonSet, RBAC, Grafana assets |

## Probe strategy (evolution)

```
Phase 1:  tracepoints (connect/accept) + optional sock-layer kprobes
Phase 2:  + read/write (or recv/send) byte-prefix capture on TCP fds
Phase 3:  + uprobes on SSL_read/SSL_write (plaintext before encrypt / after decrypt)
Phase 4:  same probes, richer identity (cgroup → pod) + OTLP export
Stretch:  HTTP/2 frames, perf_event stack sampling
```

## Why not XDP for HTTP?

XDP sees packets before (or instead of) the full TCP stack path you want for correlation with process identity. Full TCP reassembly in BPF is painful and expensive. Industry practice for MVP HTTP visibility (Beyla/Pixie-style) is: capture at **syscall or TLS library** boundaries where the buffer is already a contiguous userspace byte range owned by a known `pid`/`fd`.

XDP remains useful later for pure L3/L4 metrics or early drop policies — not for Phase 1–3 request reconstruction.

## Privilege model

| Capability | Why |
|------------|-----|
| `CAP_BPF` / `CAP_SYS_ADMIN` | Load programs, create maps |
| `CAP_PERFMON` | Perf/trace attachments on modern kernels |
| `CAP_SYS_PTRACE` | Uprobe attach / process memory introspection patterns |
| `hostPID` (K8s) | See host PIDs for cross-container correlation |
| Mount `/sys/kernel/btf` | CO-RE / BTF |

Prefer explicit capabilities over `privileged: true` when the host kernel supports it; document fallbacks.

## Failure domains

1. **Verifier reject** → program never loads; fix structure (unroll, helper reads, bounded loops).
2. **RingBuf overflow** → drop events; increment drop counter; never silently grow kernel memory without bound.
3. **Mis-correlation** → pipelined HTTP/1.1 or multiplexed HTTP/2; document and tighten FSM / add stream IDs in stretch.
4. **TLS library skew** → wrong `libssl` soname / static Go crypto; attach discovery must be version-aware; document unsupported runtimes.

## Related docs

- [DATA_FLOW.md](./DATA_FLOW.md) — event path kernel → UI/OTLP
- [PROBE_MAP.md](./PROBE_MAP.md) — which probes fire when
- [CORRELATION.md](./CORRELATION.md) — request pairing without request IDs
- [FOLDER_STRUCTURE.md](./FOLDER_STRUCTURE.md) — repo layout detail
