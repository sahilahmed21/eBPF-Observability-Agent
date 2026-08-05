# Zero-Instrumentation eBPF Observability Agent

Reconstructs per-service HTTP/gRPC latency and a live service map from kernel syscalls and TLS uprobes — **zero app code changes**, target **&lt;2% CPU overhead**.

Built in **Rust + Aya (eBPF)**. Deployable as a Kubernetes DaemonSet. Exports OpenTelemetry-compatible traces/metrics.

---

## The problem

Every APM vendor sells “add our SDK to every service.” eBPF flips that: attach to the kernel, watch syscalls and network I/O, reconstruct application-level traces (HTTP latency, gRPC call graphs, TLS handshake timing) with no per-language SDK.

Hard parts:

1. Programming inside the kernel under a strict **verifier** (no unbounded loops, 512-byte stack, no arbitrary memory access).
2. Reconstructing app semantics (HTTP req/res pairing, gRPC framing) from raw syscall/packet bytes.
3. **TLS** — interesting bytes are encrypted before the wire; intercept via **uprobes** on the SSL library, not the network.

---

## Resume signal

> Built a zero-instrumentation observability agent in Rust using eBPF (Aya) that reconstructs per-service HTTP/gRPC latency and a live service map purely from kernel-level syscall and TLS uprobe data, with under 2% CPU overhead.

---

## Stack

| Layer | Choice |
|---|---|
| eBPF | [Aya](https://aya-rs.dev/) (Rust, CO-RE/BTF) |
| Kernel | Linux **5.15+** with BTF (`/sys/kernel/btf/vmlinux`) |
| Userspace | Rust, Tokio |
| CLI | Ratatui |
| Export | OpenTelemetry (OTLP) |
| Deploy | DaemonSet (`CAP_BPF` / `CAP_PERFMON` / `CAP_SYS_PTRACE`) |

---

## Architecture (short)

```
┌─────────────────────────────────────────────────────────────┐
│  Target processes (no SDK)                                  │
│    SSL_write/read ──uprobe──┐                               │
│    read/write/sendto ───────┼──► eBPF programs (kernel)     │
│    connect/accept4 ─────────┘         │                     │
│                                       ▼                     │
│                              RingBuf + drop counters        │
└───────────────────────────────────────┬─────────────────────┘
                                        │
                                        ▼
┌─────────────────────────────────────────────────────────────┐
│  Userspace agent                                            │
│    RingBuf consumer → correlation SM → HTTP parse           │
│    → histograms → service map → Ratatui / OTLP export       │
└─────────────────────────────────────────────────────────────┘
```

Full design: [`docs/architecture/`](docs/architecture/).

**Correlation key (no request ID):** `(pid, fd, 4-tuple)` + per-socket state machine + tight timestamp windows. TLS plaintext from uprobes; wire timing from syscalls. See [correlation.md](docs/architecture/correlation.md).

**Backpressure default:** sample/drop in-kernel with a **drop-counter metric** (option a). Bigger buffers only delay the problem. See [ring-buffer-backpressure.md](docs/architecture/ring-buffer-backpressure.md).

---

## Repository layout

```
eBPF-Observability-Agent/
├── README.md
├── docs/
│   ├── architecture/          # Design: data flow, correlation, TLS, backpressure
│   ├── phases/                # Phase checklists + milestones
│   ├── verifier-rejection-log.md
│   ├── overhead.md            # Measured CPU/mem per phase
│   └── security.md            # Trust boundary, redaction, capabilities
├── ebpf/                      # Aya eBPF crate (bytecode) — Phase 0+
├── agent/                     # Userspace binary (Tokio + Ratatui + OTLP)
├── common/                    # Shared event types (ebpf ↔ userspace)
├── testdata/                  # Deterministic HTTP/HTTPS test servers
├── benches/                   # Load scripts + overhead harness
├── deploy/
│   ├── Dockerfile
│   └── k8s/                   # DaemonSet, RBAC, ServiceAccount
└── notes/                     # Interview diagrams, scratch
```

Cargo workspace lands in **Phase 0** via `aya-template`. Folders above are the target shape; empty crates are intentional until Milestone 0.

---

## Phases

| Phase | Goal | Milestone |
|---|---|---|
| **0** Setup | Toolchain + BTF + hello kprobe | Load/unload Aya kprobe, `aya-log` works |
| **1** MVP | `connect`/`accept4` latency + CLI | Live table of endpoints, overhead baseline |
| **2** HTTP | Uprobe/kprobe byte capture + HTTP/1.1 | Per-endpoint p50/p95/p99 on local server |
| **3** TLS | OpenSSL `SSL_read`/`SSL_write` uprobes | Same metrics over HTTPS + redaction |
| **4** Prod | Service map, OTLP, Grafana, DaemonSet | `kubectl apply` → live map on kind |
| **S** Stretch | HTTP/2+gRPC frames; CPU profile merge | After Phase 4 only |

Detailed checklists: [`docs/phases/`](docs/phases/).

Suggested pace (solo, part-time): ~13 weeks to Phase 4; stretch +2–3 weeks. Realistic calendar: 4–5 months.

---

## Prerequisites (before Phase 0)

### Environment (pick one)

| Option | Notes |
|---|---|
| **Cloud/VM (recommended)** | Ubuntu 22.04/24.04, kernel 5.15+. Avoids WSL2 BTF fights. |
| Native Linux | Cleanest. |
| WSL2 | Often needs custom kernel with `CONFIG_DEBUG_INFO_BTF=y`. |

**Hard gate:** `ls /sys/kernel/btf/vmlinux` must exist.

### Toolchain (run on the Linux target)

```bash
sudo apt install -y linux-tools-common "linux-tools-$(uname -r)"   # bpftool: load/unload verification
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup install stable
rustup install nightly --component rust-src
cargo install bpf-linker
cargo install cargo-generate
# Phase 1+, only needed for CO-RE bindings from kernel BTF (not on crates.io):
# cargo install --git https://github.com/aya-rs/aya -- aya-tool
# Optional static userspace binary:
rustup target add x86_64-unknown-linux-musl
```

Read the [Aya book](https://aya-rs.dev/book/) before writing custom probes.

---

## Interview questions (built into the design)

1. **Correlate TCP ↔ HTTP with no request ID?** — fd-keyed state machine; see [correlation.md](docs/architecture/correlation.md).
2. **What does the verifier reject?** — live log in [verifier-rejection-log.md](docs/verifier-rejection-log.md).
3. **TLS without app changes + security?** — [tls-interception.md](docs/architecture/tls-interception.md) + [security.md](docs/security.md).
4. **Ring buffer flooding?** — drop + counter; [ring-buffer-backpressure.md](docs/architecture/ring-buffer-backpressure.md).

---

## Testing strategy

- **Correctness:** deterministic test server with injectable sleep; assert p50/p99 within tolerance (`testdata/`).
- **Overhead:** same load script every milestone; record in `docs/overhead.md`.
- **Verifier:** every rejection → one entry in `docs/verifier-rejection-log.md`.

---

## Security (non-negotiable)

Agent reads **plaintext TLS** on the host. Requires elevated caps. Treat as a high-value trust boundary: least-privilege caps, node-scoped deploy, header redaction (`Authorization`, `Cookie`, secret patterns) before export. Details: [`docs/security.md`](docs/security.md).

---

## Status

| Item | State |
|---|---|
| Docs + layout | ✅ |
| Phase 0 (BTF + hello kprobe) | ✅ Milestone 0 — loads, logs, unloads clean |
| Phase 1 (connect/accept latency CLI) | ✅ Milestone 1 — smoke1 + overhead row (`docs/overhead.md`) |
| Phase 2–4 | ⬜ |

**Next:** Optional accept4 smoke; then Phase 2 HTTP.

Phase 0 runbook: [`docs/phases/phase-0-implementation-plan.md`](docs/phases/phase-0-implementation-plan.md) ·
evidence: [`docs/testing/phase-0.tdd.md`](docs/testing/phase-0.tdd.md).

Phase 1 runbook: [`docs/phases/phase-1-implementation-plan.md`](docs/phases/phase-1-implementation-plan.md) ·
evidence: [`docs/testing/phase-1.tdd.md`](docs/testing/phase-1.tdd.md).

---

## License

TBD (add when you publish).
