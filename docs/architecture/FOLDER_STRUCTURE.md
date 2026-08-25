# Folder structure (detailed)

This document explains **why** each directory exists and what belongs in it. Placeholder `mod.rs` / `README.md` files mark modules before Phase 0 code lands.

## Workspace crates

```
Cargo.toml          # [workspace] members = agent, ebpf, common, xtask, demos/*
rust-toolchain.toml # channel pins + components
```

### `common/`

Shared between eBPF (no_std) and userspace (std).

| Path | Purpose |
|------|---------|
| `src/lib.rs` | Crate root; feature flags if needed (`user` vs `ebpf`) |
| `src/events.rs` | RingBuf event structs, `EventKind`, max prefix sizes |
| `src/keys.rs` | HashMap key types (`PidTgid`, `SockKey`) |

**Rules:** only `#[repr(C)]` plain data; no heap; sizes must match on both sides; bump an explicit schema version when layouts change.

### `ebpf/`

Compiled to `bpfel-unknown-none` (or `bpfeb-*` on big-endian — we target little-endian first).

| Path | Phase | Purpose |
|------|-------|---------|
| `src/main.rs` | 0 | Program license, module wiring |
| `src/connect.rs` | 1 | connect/accept enter/exit |
| `src/socket.rs` | 1 | CO-RE sock field helpers |
| `src/http_capture.rs` | 2 | bounded read/write capture |
| `src/tls.rs` | 3 | SSL_read/SSL_write uprobes |
| `src/maps.rs` | 1 | map declarations (HashMap, RingBuf, counters) |

**Rules:** no unbounded loops; use helpers for all pointer reads; keep stack tiny; put large buffers in per-CPU arrays / map values, not on the BPF stack.

### `agent/`

Userspace binary that loads BPF and serves CLI / export.

| Path | Purpose |
|------|---------|
| `src/main.rs` | CLI args, runtime bootstrap |
| `src/loader.rs` | `Ebpf::load`, attach probes, resolve libssl |
| `src/consumer/` | Tokio RingBuf reader, batching |
| `src/correlation/` | Per-socket FSM (Phase 2); TlsIo feeds same SM in Phase 3 |
| `src/http/` | `httparse`, path normalization |
| `src/hist/` | Rolling histograms |
| `src/identity/` | `/proc` cgroup → container/pod |
| `src/service_map/` | Adjacency graph |
| `src/redaction/` | Header/secret scrubbing |
| `src/otel/` | OTLP export |
| `src/dashboard/` | Ratatui UI |
| `src/reassemble.rs` | Phase 6 — 8 KiB per-fd buffer |
| `src/filter.rs` | Phase 6 — comm allow/deny; Phase 11 fills `DENIED_TGID` or `ALLOWED_TGID` |
| `src/sample.rs` | Phase 11 — BPF SAMPLE_N auto/pin policy |
| `src/h2/` | Phase 7 — frames, HPACK subset, stream FSM |
| `src/dual_plane.rs` | Phase 8 — TlsIo content × SockIoTimes |
| `src/trace_export.rs` | Phase 9 — OTLP `/v1/traces` JSON |
| `src/profile.rs` | Phase 12 — stack join (off by default) |

### `xtask/`

Build orchestration (compile eBPF with nightly, then userspace). Keeps `cargo build` ergonomics close to Aya’s recommended workflow.

### `demos/`

Local targets for Milestone 1–4 validation. Not production code.

| Demo | Use |
|------|-----|
| `http-server/` | Injectable latency endpoints for correctness tests |
| `https-server/` | OpenSSL-backed TLS for Phase 3 |
| `microservices/` | Multi-process call graph for Phase 4 service map |

### `deploy/`

| Path | Purpose |
|------|---------|
| `Dockerfile` | Multi-stage: build BPF + musl agent → distroless/scratch |
| `k8s/daemonset.yaml` | hostPID, caps, BTF mounts |
| `k8s/rbac.yaml` | Least-privilege K8s API access for pod naming |
| `grafana/dashboards/` | JSON dashboard for portfolio screenshots |

### `docs/`

| Path | Purpose |
|------|---------|
| `architecture/` | Stable design docs (this folder) |
| `design-notes/` | Interview-depth writeups |
| `verifier-rejection-log.md` | Append-only scar log |
| `overhead-measurements.md` | Numbers per phase |
| `phases/` | Phase checklists + implementation plans (0–12) + [VISION-95.md](../phases/VISION-95.md) |
| `testing/` | TDD evidence per phase |
| `handoff/` | Session notes; [SESSION-VISION-95.md](../handoff/SESSION-VISION-95.md) claim lock |

### `scripts/`

Developer UX: BTF check, loadgen, overhead sampling. Keep these runnable without the full agent early (Phase 0).

### `tests/`

| Path | Purpose |
|------|---------|
| `correctness/` | Assert latency within tolerance vs injectable server |
| `fixtures/` | Captured byte prefixes / pcap-like samples for parsers |

## What not to put where

- Do **not** put HTTP parsing in eBPF (keep BPF dumb and bounded; parse in userspace).
- Do **not** put Tokio/async in `common` or `ebpf`.
- Do **not** commit large Grafana screenshots into `ebpf/` — use `docs/` or portfolio assets outside the hot path.
- Do **not** vendor kernel headers by hand if CO-RE + BTF can express the fields you need.

## Growth path

```
Phase 0–5:  demo (HTTP/1.1, OpenSSL TLS-only, kind metrics)
Phase 6:    writev/sendmsg + reassemble + deny-list + SockMeta v6
Phase 7:    agent/h2 (frames, HPACK subset, gRPC :path)
Phase 8:    SockIoTimes + TlsHandshake; dual-plane join
Phase 9:    OTLP traces + Grafana
Phase 10:   k3s real-node identity + Service ClusterIP dest join
Phase 11:   SAMPLE_N + vision95-load perf stat
Phase 12:   STACKS RingBuf + blazesym join; claim lock
```
