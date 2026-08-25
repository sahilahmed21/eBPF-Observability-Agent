# Zero-Instrumentation eBPF Observability Agent

Rust + Aya (eBPF) agent that reconstructs **HTTP/1.1**, **HTTP/2/gRPC**, and OpenSSL HTTPS latency from syscalls/uprobes with **zero app SDK**, builds a **live named service map** on a real node, and exports OTLP metrics/traces. Optional CPU stack join on slow spans (`OBSAGENT_PROFILE=1`).

**Claim lock:** [docs/handoff/SESSION-VISION-95.md](docs/handoff/SESSION-VISION-95.md) is **10/10 PASS** (2026-08-25). Measured pin-load overhead is **~87% of one core** — do **not** claim `&lt;2%` CPU.

---

## The problem

Every APM vendor sells “add our SDK to every service.” eBPF flips that: attach to the kernel, watch syscalls and network I/O, reconstruct application-level traces with no per-language SDK.

Hard parts:

1. Programming inside the kernel under a strict **verifier** (no unbounded loops, 512-byte stack, no arbitrary memory access).
2. Reconstructing app semantics (HTTP req/res pairing, framing) from raw syscall bytes.
3. **TLS** — interesting bytes are encrypted before the wire; intercept via **uprobes** on the SSL library, not the network.

---

## Resume signal (claim-locked)

> Built a zero-instrumentation observability agent in Rust + Aya (eBPF) that reconstructs per-service HTTP/gRPC latency and a live named service map on a real k3s node, exports OTLP metrics/traces, joins CPU stacks to slow spans, and measures **~87% of one core** under the Vision-95 pin load (HTTP 500/s + gRPC ~200/s) — not under 2%.

Evidence table: [`docs/handoff/SESSION-VISION-95.md`](docs/handoff/SESSION-VISION-95.md). Overhead detail: [`docs/overhead.md`](docs/overhead.md).

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

**Start here for a full tour (flows, files, mermaid):** [`docs/architecture/PROJECT-DEEP-DIVE.md`](docs/architecture/PROJECT-DEEP-DIVE.md).

**Correlation key (no request ID):** `(pid, fd, 4-tuple)` + per-socket state machine. Phase 2 uses sock I/O prefixes; Phase 3 feeds OpenSSL plaintext (`TlsIo`) into the **same** SM after `SSL_set_fd`→fd mapping. Phase 8 adds wire timing (`SockIoTimes`) joined to TLS content; handshake via `SSL_do_handshake`. See [correlation.md](docs/architecture/correlation.md).

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
| **0** Setup | Toolchain + BTF + hello kprobe | Load/unload Aya kprobe |
| **1** MVP | `connect`/`accept4` latency + CLI | Live table, overhead baseline |
| **2** HTTP | HTTP/1.1 from sock I/O | Per-endpoint p50 on local server |
| **3** TLS | OpenSSL uprobes | HTTPS content latency (M3) |
| **4–5** Prod demo | Service map, OTLP metrics, kind DS | kind scrape; identity **not** proven |
| **6** Capture | `writev`/`sendmsg`, reassembly, deny-list, IPv6 | `correctness6` |
| **7** gRPC | HTTP/2 frames + `:path` | `correctness7-grpc` + h2-tls |
| **8** Dual-plane | SockIoTimes + handshake | `correctness8-dual` |
| **9** Traces | OTLP spans + Grafana | collector `/v1/traces` 2xx |
| **10** Map | Real-node named src/dst | k3s e2e |
| **11** Overhead | `perf stat` + sampling | &lt;2% or honest number |
| **12** Profiles | Stacks on slow spans | **Claim lock** |

Phases 0–5 are the demo. **95% of the original brief:** [`docs/phases/VISION-95.md`](docs/phases/VISION-95.md). **Implement from:** [`docs/phases/README.md`](docs/phases/README.md) · [`docs/ROADMAP.md`](docs/ROADMAP.md).

Suggested pace: Phases 6–12 ~8–12 focused weeks (see VISION-95 calendar). Do not claim gRPC or &lt;2% until Milestone 12.

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
