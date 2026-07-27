# Zero-Instrumentation eBPF Observability Agent

A DaemonSet-deployable agent that reconstructs **HTTP/gRPC traces** and a **live service map** from kernel syscalls and TLS uprobes — with **zero application code changes**, near-zero overhead (target **&lt;2% CPU**), and no per-language SDK.

> **Resume signal:** Built a zero-instrumentation observability agent in Rust using eBPF (Aya) that reconstructs per-service HTTP/gRPC latency and a live service map purely from kernel-level syscall and TLS uprobe data, with under 2% CPU overhead.

---

## Why this exists

Every APM vendor sells “just add our SDK to every service.” That is the adoption barrier.

eBPF-based tools (Cloudflare’s internal tooling, Grafana Beyla, Pixie / Tetragon at CNCF) flip the model:

1. Attach to the kernel.
2. Watch syscalls and network I/O (and TLS library boundaries via uprobes).
3. Reconstruct application-level traces — HTTP latency, gRPC call graphs, TLS handshake timing — without touching app code.

This project implements that model end-to-end in **Rust + Aya**, from a syscall latency MVP through production Kubernetes packaging.

---

## What you get (by phase)

| Phase | Capability | Status |
|-------|------------|--------|
| **0** | Toolchain + BTF + hello-world kprobe | Not started |
| **1** | `connect` / `accept` latency tracker + Ratatui CLI | Planned |
| **2** | HTTP/1.1 request/response pairing + per-endpoint histograms | Planned |
| **3** | OpenSSL TLS uprobe interception + redaction | Planned |
| **4** | Service map, OTLP export, Grafana, K8s DaemonSet | Planned |
| **S1** | gRPC / HTTP/2 frame demux | Stretch |
| **S2** | Continuous CPU profiling merged into the trace timeline | Stretch |

---

## Architecture at a glance

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         Target workloads (no SDKs)                        │
│   process A ──SSL_write/read──► libssl.so     process B ──write/read──►  │
└───────────────┬───────────────────────┬─────────────────┬────────────────┘
                │ uprobes               │ kprobes /       │
                │ (TLS plaintext)       │ tracepoints     │
                ▼                       ▼                 ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                     eBPF programs (Aya, CO-RE, BTF)                       │
│  connect/accept timing │ socket metadata │ HTTP byte prefixes │ TLS hooks │
│                         RingBuf / HashMaps / drop counters                │
└───────────────────────────────────┬──────────────────────────────────────┘
                                    │ events
                                    ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                     Userspace agent (Tokio + Rust)                        │
│  RingBuf consumer → correlation FSM → HTTP parse → histograms            │
│  TLS↔syscall merge → service map → redaction → OTLP / CLI dashboard      │
└───────────────┬───────────────────────────────┬──────────────────────────┘
                │                               │
                ▼                               ▼
         Ratatui CLI                    OTel Collector → Grafana
         (live tables)                  (metrics + traces)
```

**Design principles**

- Prefer **tracepoints** over fragile kprobe symbol names where stable ABI exists.
- Prefer **socket-layer / CO-RE** field access for addressing over raw `fd`-only views.
- Prefer **RingBuf** over PerfEventArray for ordering and memory efficiency.
- Bound all kernel-side captures (prefix sizes, map cardinality, loop bounds) for the verifier and for overhead.
- Default load shedding: **sample / drop with explicit drop counters** when the ring buffer cannot keep up.

See [`docs/architecture/`](docs/architecture/) for deep dives.

---

## Repository layout

```
.
├── README.md                          # This file
├── Cargo.toml                         # Workspace root (Phase 0+)
├── .gitignore
├── rust-toolchain.toml                # Stable + nightly (eBPF) pins
│
├── docs/
│   ├── architecture/                  # System design, data flows, probe map
│   │   ├── OVERVIEW.md
│   │   ├── DATA_FLOW.md
│   │   ├── PROBE_MAP.md
│   │   ├── CORRELATION.md
│   │   └── FOLDER_STRUCTURE.md
│   ├── design-notes/                  # Interview-ready design writeups
│   │   ├── tls-security.md
│   │   ├── ring-buffer-backpressure.md
│   │   └── path-normalization.md
│   ├── verifier-rejection-log.md      # Real verifier scars (fill as you hit them)
│   ├── overhead-measurements.md       # CPU%/RSS per phase under fixed load
│   └── ROADMAP.md                     # Phase checklist
│
├── common/                            # Shared types: eBPF ↔ userspace
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── events.rs                  # RingBuf event layouts (#[repr(C)])
│       └── keys.rs                    # Map keys (pid/fd/tuple helpers)
│
├── ebpf/                              # eBPF crate (bpfel-unknown-none)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                    # Program entry / license
│       ├── connect.rs                 # Phase 1: connect/accept probes
│       ├── socket.rs                  # Phase 1: sock metadata (CO-RE)
│       ├── http_capture.rs            # Phase 2: read/write prefixes
│       └── tls.rs                     # Phase 3: SSL_read/SSL_write uprobes
│
├── agent/                             # Userspace binary
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── loader.rs                  # Load/attach BPF, maps, uprobes
│       ├── consumer/                  # Async RingBuf drain (Tokio AsyncFd)
│       ├── correlation/               # Per-socket HTTP FSM + TLS merge
│       ├── http/                      # httparse + path normalize
│       ├── hist/                      # hdrhistogram aggregations
│       ├── identity/                  # PID → cgroup → pod/service
│       ├── service_map/               # In-memory call graph
│       ├── redaction/                 # Header/secret scrubbing
│       ├── otel/                      # OTLP traces + metrics
│       └── dashboard/                 # Ratatui live UI
│
├── xtask/                             # Build helpers (aya-style)
│   ├── Cargo.toml
│   └── src/main.rs
│
├── demos/                             # Local / kind demo workloads
│   ├── http-server/                   # Axum test service (injectable latency)
│   ├── https-server/                  # TLS-terminated companion
│   └── microservices/                 # Multi-service demo for service map
│
├── deploy/                            # Production packaging (Phase 4)
│   ├── Dockerfile
│   ├── k8s/
│   │   ├── daemonset.yaml
│   │   ├── rbac.yaml
│   │   └── configmap.yaml
│   └── grafana/
│       └── dashboards/
│
├── scripts/                           # Dev / measurement helpers
│   ├── check-btf.sh
│   ├── measure-overhead.sh
│   └── loadgen.sh
│
└── tests/                             # Integration / correctness harnesses
    ├── correctness/
    └── fixtures/
```

Full rationale for each crate and module: [`docs/architecture/FOLDER_STRUCTURE.md`](docs/architecture/FOLDER_STRUCTURE.md).

---

## Stack

| Layer | Choice | Why |
|-------|--------|-----|
| Language | Rust | Memory safety in userspace; Aya ecosystem for eBPF |
| eBPF framework | [Aya](https://aya-rs.dev/) | Idiomatic Rust eBPF, CO-RE, no BCC runtime dependency |
| Kernel | Linux **5.15+** with **BTF** | RingBuf, CO-RE, modern map types |
| Async runtime | Tokio | Non-blocking RingBuf consumer |
| CLI UI | Ratatui | Live latency tables / sparklines |
| HTTP parse | `httparse` | Zero-copy header parsing of byte prefixes |
| Histograms | `hdrhistogram` | High-dynamic-range latency percentiles |
| Export | OpenTelemetry OTLP | Industry-standard traces + metrics |
| Deploy | K8s DaemonSet | Node-local privileged (or CAP_BPF) agent |

---

## Prerequisites

### Hard requirements

1. **Linux kernel 5.15+** with BTF:

   ```bash
   ls /sys/kernel/btf/vmlinux   # must exist
   uname -r
   ```

2. **Privileges** for loading BPF / attaching probes (dev: root or `CAP_BPF` + `CAP_PERFMON` + `CAP_SYS_PTRACE` on modern kernels).

3. **Rust toolchain** — stable for userspace, nightly + `rust-src` for the eBPF target; `bpf-linker` installed.

### Recommended environment

| Environment | Verdict |
|-------------|---------|
| Cloud / native Linux VM (Ubuntu 22.04/24.04) | **Preferred** — fewest BTF surprises |
| Native Linux laptop | Excellent |
| WSL2 | Often missing BTF; custom kernel required — avoid for Phase 0 |

This project is CPU/kernel-bound, not GPU-bound. A small cloud VM is ideal.

### Quick env check

```bash
./scripts/check-btf.sh
```

---

## Getting started (Phase 0 gate)

Phase 0 is the hard gate: if a trivial Aya kprobe will not compile, load, and log, nothing downstream will either.

```bash
# 1. Toolchain (outline — detailed in docs/ROADMAP.md Phase 0)
rustup install stable
rustup install nightly --component rust-src
cargo install bpf-linker

# 2. Verify BTF
./scripts/check-btf.sh

# 3. Scaffold / build / load hello-world kprobe (Phase 0 implementation)
#    cargo xtask build-ebpf
#    cargo xtask run   # or: sudo -E cargo run -p agent --release
```

**Milestone 0:** Load a trivial Aya kprobe, see output via `aya-log`, unload cleanly.

Do **not** skip reading the [Aya book](https://aya-rs.dev/book/) before writing custom programs. The verifier’s constraints (bounded loops, 512-byte stack, no arbitrary deref, path-sensitive analysis) feel arbitrary until you understand them.

---

## Phase roadmap (summary)

### Phase 1 — MVP: syscall latency tracker

- Tracepoints on `sys_enter/exit_connect` and `accept4`.
- Entry HashMap `(pid,tid) → timestamp`; exit computes delta → RingBuf event.
- Socket metadata via CO-RE on `struct sock` (preferred) or `sockaddr` probe-read.
- Tokio consumer + Ratatui: p50/p95/p99 connect latency per remote endpoint.
- **Record overhead baseline** under synthetic load.

### Phase 2 — HTTP awareness

- Capture bounded prefixes from `read`/`write` (or `recv`/`send`) on known TCP fds.
- Correlate with `(pid, fd, tuple)` + per-socket state machine (`AwaitingRequest → … → ResponseReceived`).
- Parse with `httparse`; normalize paths; `hdrhistogram` per endpoint.
- Document pipelining mis-pair failure mode honestly.

### Phase 3 — TLS interception

- Uprobes on `SSL_write` / `SSL_read` in `libssl.so.{1.1,3}` (version skew handled at attach time).
- Correlate TLS plane ↔ syscall plane via `(pid, tid)` + tight timestamp window.
- Redact `Authorization`, `Cookie`, and common secret patterns before export.
- Document privilege / trust-boundary implications ([`docs/design-notes/tls-security.md`](docs/design-notes/tls-security.md)).

### Phase 4 — Production grade

- Service map: process/cgroup → container → pod/namespace identity.
- OTLP traces + metrics; Grafana dashboard; DaemonSet with least-privilege caps where possible.
- Demo on kind/minikube with a multi-service app — **zero changes** to those services.

### Stretch

- **S1:** HTTP/2 frame demux + gRPC method from `:path`.
- **S2:** perf-event stack sampling + `blazesym` symbolization merged onto slow-request windows.

Detailed checklist: [`docs/ROADMAP.md`](docs/ROADMAP.md).

---

## Hard interview questions (and where we answer them)

| Question | Where we live it |
|----------|------------------|
| How do you correlate a kernel TCP event with an HTTP request with **no request ID**? | Phase 2 FSM — [`docs/architecture/CORRELATION.md`](docs/architecture/CORRELATION.md) |
| What does the **verifier** reject, and how did you restructure? | [`docs/verifier-rejection-log.md`](docs/verifier-rejection-log.md) (fill with real cases) |
| How do you intercept **TLS** without app changes, and what are the security implications? | Phase 3 + [`docs/design-notes/tls-security.md`](docs/design-notes/tls-security.md) |
| Ring buffer fills faster than userspace can drain — options and pick? | **Drop with drop-counter metric** by default — [`docs/design-notes/ring-buffer-backpressure.md`](docs/design-notes/ring-buffer-backpressure.md) |

---

## Testing & validation strategy

- **Correctness:** deterministic HTTP server with injectable sleep; assert reported p50/p99 within tolerance.
- **Overhead:** same load script every phase → fill [`docs/overhead-measurements.md`](docs/overhead-measurements.md).
- **Verifier scars:** every rejection → entry in the rejection log (interview ammunition).

```bash
./scripts/loadgen.sh          # fixed synthetic load
./scripts/measure-overhead.sh # agent CPU%/RSS vs baseline
```

---

## Security model (read before running in shared environments)

This agent is a **high-trust boundary**:

- Requires elevated privileges to attach eBPF / uprobes.
- TLS uprobes observe **plaintext** application data on the host.
- Must be scoped (node DaemonSet RBAC, audit, redaction) — never “run casually” on multi-tenant hosts.

See [`docs/design-notes/tls-security.md`](docs/design-notes/tls-security.md).

---

## Status

| Item | State |
|------|-------|
| Docs + folder scaffolding | **Done** (this commit) |
| Phase 0 — toolchain / hello kprobe | Next |
| Phases 1–4 | Not started |

---

## License

TBD (recommend Apache-2.0 or MIT once code lands).

---

## References

- [Aya book](https://aya-rs.dev/book/)
- [Aya template](https://github.com/aya-rs/aya-template)
- [Grafana Beyla](https://grafana.com/oss/beyla/)
- [Pixie](https://px.dev/) / [Tetragon](https://tetragon.io/)
- Linux kernel BPF docs (`Documentation/bpf/`)
