# eBPF Observability Agent

**Zero-instrumentation** HTTP/gRPC observability for Linux nodes — reconstruct latency and a live service map from kernel syscalls and OpenSSL uprobes, with **no application SDK**.

Built in **Rust + [Aya](https://aya-rs.dev/)** (eBPF). Deploys as a Kubernetes **DaemonSet**. Exports **OpenTelemetry** (OTLP) metrics and sampled traces.

---

## Results at a glance

Validated on **WSL2 + k3s** (claim lock **10/10**, 2026-08-25):

| Capability | Result |
|---|---|
| HTTP / HTTPS latency | Injectable `/slow` **50 ms** → observed p50 ≈ **50.7 ms** |
| gRPC / HTTP/2 | Cleartext gRPC + OpenSSL HTTP/2 gates **PASS** |
| Dual-plane TLS | Content latency + wire timing join; handshake via `SSL_do_handshake` |
| Service map | Named edge **`demo/frontend` → `demo/api`** on a real node |
| OTLP + Grafana | Metrics + sampled traces scraped end-to-end |
| Overhead (pin load) | HTTP **500/s** + gRPC **~200/s**, 60s × 3 → **~87% of one core** (`perf stat`) |
| Overload | RingBuf drops visible; sticky sampling engages (`sample_n=2`); RSS stable |
| CPU profiles | Optional join of stacks onto slow spans (`OBSAGENT_PROFILE=1`) |

> Overhead is **measured**, not aspirational. This project does **not** claim “under 2% CPU.”

**Project status:** [Goals achieved — why this is complete](docs/GOALS-AND-SUCCESS.md) (claim lock **10/10**).

![Grafana dashboard](docs/handoff/artifacts/grafana-vision95.png)

---

## What it does

APM SDKs require changing every service. This agent attaches to the **kernel** and **libssl** instead:

1. **Capture** — syscall tracepoints (`connect`/`accept`, `read`/`write`/`writev`/…) and OpenSSL uprobes (`SSL_read`/`SSL_write`, handshake).
2. **Transport** — compact events on a **RingBuf**; under pressure, **drop + counter** (never block the kernel).
3. **Reconstruct** — pair requests/responses with a `(process, fd)` state machine (HTTP/2 adds `stream_id`); parse in userspace.
4. **Export** — per-route latency histograms, service-map edges, OTLP metrics/traces; optional CPU stacks on slow requests.

```
  Apps (no SDK)                    Linux kernel (eBPF)                 Agent (userspace)
 ┌──────────────┐                 ┌──────────────────┐               ┌────────────────────┐
 │ HTTP / gRPC  │──syscalls──────►│ Tracepoints      │──RingBuf─────►│ Correlate + parse  │
 │ OpenSSL TLS  │──uprobes───────►│ + OpenSSL hooks  │               │ Service map        │
 └──────────────┘                 │ DROPS / SAMPLE_N │               │ OTLP → collector   │
                                  └──────────────────┘               └────────────────────┘
```

**Scope (honest):** OpenSSL 1.1/3.x plaintext path; not rustls/Go `crypto/tls`. Single-node DaemonSet model; not multi-cluster APM. Privileged host agent — treat as a trust boundary.

## Agent observability additions (2026-11)

Measured on WSL2 (kernel 6.6), loopback, no k3s. Reproduce with the scripts named below.

| Capability | How it is verified | Result |
|---|---|---|
| Replayable pipeline | `obsagent replay <file>` over a real capture, `tests/fixtures/slow-50ms.evlog` (no root) | 15 exchanges, p50 50.5 ms; live agent on the same run: 15, 50.5 ms; 20 replays identical |
| Record with redaction | `OBSAGENT_RECORD=<path>` (mode 0600) | `authorization`, `cookie`, `x-api-key` header values overwritten before write; HTTP/2 header blocks are **not** redacted |
| Loss behaviour | unit test over replay: drop 0/10/50% of records | exchanges recovered = exchanges whose request and response records both survive |
| Streamed LLM calls (SSE, HTTP/1.1) | `scripts/correctness-llm.sh`, 30 calls vs client-side timing | worst error: TTFB 0.14%, total 0.05%; token counts exact; exported as OTLP `chat` client spans with `gen_ai.usage.*_tokens` (unit-tested, not yet checked against a live collector) |
| Policy audit (detection only) | `scripts/correctness-policy.sh`, `OBSAGENT_FILEOPS=1 OBSAGENT_POLICY=<rules>` | 5/5 canary violations detected, 0 unexpected, 5 benign scenarios |
| Unit tests | `./scripts/wsl-run.sh test-common` and `test-agent` | 22 + 188 pass |

Scope notes: LLM capture needs the TLS plaintext path (OpenSSL), HTTP/1.1 chunked SSE, and sees only the first 256 B of each read, so a cut-off usage object gives `None`, never `0`. The policy audit has 5 labeled scenarios, deny-prefix/deny-basename rules only, and no process-tree scoping; the `openat`/`execve` probes fire for every captured process, so their overhead was not measured.

---

## Limitations

- **Overhead is high and environment-dependent.** ~87% of one core (agent process only, `perf stat -p`) at 500 HTTP + ~200 gRPC rps on WSL2 + k3s; a 2026-11 re-measurement on plain WSL2 loopback gave ~14–21% for the same load, and in-kernel BPF time (~16 µs per `write`) is additional and not in either number. The k3s gap is not yet explained. Details: [`docs/overhead.md`](docs/overhead.md).
- **Latency ends at response headers**, so a streamed response is measured to its headers, not its last byte.
- **Capture is a 256 B prefix** of the first iovec per syscall; no `io_uring` or `sendfile` probes.
- **OpenSSL only.** No Go `crypto/tls` or rustls. OpenTelemetry's eBPF Instrumentation (OBI) covers more TLS stacks and protocols; this project is a learning-grade agent, not a replacement for it.
- **No trace propagation.** Each exchange is a root span; the service map shows edges but spans are not parent/child linked.
- **Tested on one kernel** (WSL2 6.6). Unit tests run in CI; the correctness gates need root and a BPF-capable host.

---

## Quick demo (Kubernetes)

**Needs:** Linux node with BTF (`ls /sys/kernel/btf/vmlinux`), Docker, `kubectl`, a cluster (kind / k3s / similar), and privileges for BPF.

```bash
# 1. Build the agent image
docker build -f deploy/Dockerfile -t ebpf-obs-agent:latest .

# 2. Load into the cluster (example: kind)
kind load docker-image ebpf-obs-agent:latest
# k3s: import with your usual ctr/nerdctl path

# 3. Deploy RBAC, collector, agent, demo apps
kubectl apply -f deploy/k8s/rbac.yaml
kubectl apply -f deploy/k8s/otel-collector.yaml
kubectl apply -f deploy/k8s/configmap.yaml
kubectl apply -f deploy/k8s/daemonset.yaml
kubectl apply -f demos/microservices/k8s.yaml

# 4. Watch the agent (headless metrics)
kubectl -n observability logs -f ds/ebpf-obs-agent

# 5. Scrape OTLP metrics via the collector Prometheus exporter
kubectl -n observability port-forward svc/otel-collector 8889:8889
# open http://127.0.0.1:8889/metrics  — look for http_* / obsagent_* series
```

You should see HTTP routes, latency percentiles, and edges such as **frontend → api** once traffic flows in the demo namespace.

**Capabilities (DaemonSet):** `CAP_BPF`, `CAP_PERFMON`, `CAP_SYS_PTRACE`, `CAP_SYS_RESOURCE`, `hostPID: true`, plus BTF / `/host/proc` / cgroup mounts. See `deploy/k8s/daemonset.yaml`.

---

## Local run (Linux / WSL2)

Build and run on a Linux host with BTF (WSL2 Ubuntu is the supported Windows path):

```bash
# Toolchain (once)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup install stable && rustup install nightly --component rust-src
cargo install bpf-linker
sudo apt install -y build-essential clang llvm pkg-config

# From the repo (WSL helper strips CRLF and sets CARGO_TARGET_DIR)
./scripts/wsl-run.sh build
sudo ./scripts/wsl-run.sh smoke9          # OTLP path smoke (needs collector or skip)
# Correctness examples (root + BPF):
sudo ./scripts/wsl-run.sh correctness6    # writev + split-header latency
sudo ./scripts/wsl-run.sh correctness7-grpc
```

Headless agent against local traffic (after `./scripts/wsl-run.sh build`):

```bash
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/obsagent-target}"
sudo OBSAGENT_HEADLESS=1 RUST_LOG=info \
  "$CARGO_TARGET_DIR/release/obsagent"
# Optional: OBSAGENT_OTLP=1 OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318
# Optional: OBSAGENT_PROFILE=1   # CPU stacks on slow spans
```

---

## Stack

| Layer | Choice |
|---|---|
| eBPF | Aya (Rust, CO-RE / BTF) |
| Kernel | Linux **5.15+** with `/sys/kernel/btf/vmlinux` |
| Userspace | Rust, Tokio |
| Export | OTLP/HTTP → OpenTelemetry Collector → Prometheus / Grafana |
| Deploy | Kubernetes DaemonSet |

---

## Repository layout

```
agent/          # Userspace binary (drain, correlate, OTLP)
ebpf/           # eBPF programs + maps
common/         # Shared #[repr(C)] event ABI
testdata/       # Deterministic HTTP / gRPC / writev probes
deploy/         # Dockerfile + DaemonSet / RBAC / collector
demos/          # Microservices manifest for the named-edge demo
scripts/        # Smoke, correctness, overhead, e2e helpers
docs/           # Architecture notes, overhead table, security
```

---

## Security

The agent can observe **TLS plaintext** at the OpenSSL boundary and requires elevated capabilities. Deploy node-scoped with least privilege, redact sensitive headers before export, and treat the process as a high-value trust boundary. See [`docs/security.md`](docs/security.md).

---

## License

MIT OR Apache-2.0 (see workspace `Cargo.toml`).
