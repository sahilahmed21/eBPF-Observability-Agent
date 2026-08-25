# Roadmap & phase checklist

Use this as the execution checklist. Update status as milestones land.

## Phase 0 — Foundations (next)

- [x] Confirm BTF: `/sys/kernel/btf/vmlinux` exists on the primary dev host
- [x] Install stable + nightly (`rust-src`), `bpf-linker`
- [x] Read [Aya book](https://aya-rs.dev/book/) (concepts before custom code)
- [x] Scaffold Aya workspace (`common`, `ebpf`, `agent`, `xtask`)
- [x] Load trivial kprobe; `aya-log` output; clean unload
- [x] **Milestone 0** complete

## Phase 1 — Syscall latency MVP

- [x] Tracepoints: connect enter/exit
- [x] Tracepoints: accept4 enter/exit
- [x] Pending HashMap + RingBuf events
- [x] Socket metadata (CO-RE sock fields preferred)
- [x] Tokio RingBuf consumer
- [x] Ratatui dashboard (p50/p95/p99, top talkers)
- [x] Overhead baseline recorded in `overhead-measurements.md`
- [x] **Milestone 1** complete

## Phase 2 — HTTP awareness

- [x] Bounded prefix capture on read/write for tracked TCP fds
- [x] Per-socket correlation FSM
- [x] `httparse` userspace parsing + graceful truncation
- [x] Path normalization heuristics
- [x] `hdrhistogram` per-endpoint table in CLI
- [x] Document pipelining mis-pair failure mode
- [x] **Milestone 2** complete

## Phase 3 — TLS interception (OpenSSL HTTPS)

Plan: [phases/phase-3-implementation-plan.md](phases/phase-3-implementation-plan.md) (Q1–Q12 locked; Q13–Q15 filled).

- [x] Try-attach `libssl.so.3` / `libssl.so.1.1` (soft-fail; cleartext must keep working)
- [x] Uprobes: `SSL_set_fd` (+ rfd/wfd), `SSL_write`/`SSL_read` + `_ex` enter-stash / exit-emit
- [x] `TlsIo` → existing `(tgid,fd)` correlator (TLS-only latency — **no** dual-plane wire merge)
- [x] Skip sock I/O on TLS-marked fds; metrics-only UI; never log raw prefixes
- [x] OpenSSL-backed HTTPS testdata + smoke/correctness/overhead
- [x] Security note reviewed (`security.md` / design-notes)
- [x] **Milestone 3** complete

## Phase 4 — Production grade

Plan: [phases/phase-4-implementation-plan.md](phases/phase-4-implementation-plan.md).

- [x] Service map graph in memory
- [x] PID → cgroup → container → pod/namespace identity
- [x] OTLP metrics (traces optional / deferred)
- [x] Collector + Prometheus scrape path (Grafana dashboard JSON optional)
- [x] Dockerfile + DaemonSet (capabilities, BTF mounts)
- [x] kind/minikube demo with multi-service app
- [x] **Milestone 4** complete (kind `obsagent` 2026-08-15: DS + demo + OTLP scrape)

## Phase 5 — Production hardening

Plan: [phases/phase-5-implementation-plan.md](phases/phase-5-implementation-plan.md).

- [x] Cumulative OTLP histograms (replace gauge-per-sample)
- [x] Agent self-metrics + label/peer hygiene
- [x] Grafana dashboard JSON
- [x] smoke4 / optional kind e2e gate
- [x] **Milestone 5** complete (local gates + `KIND_E2E=1` smoke4 + Prometheus scrape 2026-08-15)

## Original vision → 95% (Phases 6–12)

Phases 0–5 are the **demo**. Phases 6–12 claim lock is **10/10** (2026-08-25). Speak the resume sentence with **~87% of one core** — never &lt;2%.

North star + Qs: [phases/VISION-95.md](phases/VISION-95.md). Stretch S1/S2 are absorbed into Phases 7 and 12.

**Implement in order. Each phase has checklist + implementation plan + TDD file.**

### Phase 6 — Capture completeness

Checklist: [phases/phase-6.md](phases/phase-6.md) · Plan: [phases/phase-6-implementation-plan.md](phases/phase-6-implementation-plan.md)

- [x] `readv` / `writev` / `sendmsg` / `recvmsg` (first iovec, 256 B)
- [x] Userspace reassembly (8 KiB / 60 s) + split-header gate
- [x] Comm/cgroup allow-deny list (k8s defaults)
- [x] IPv6 sockaddr → `SockMeta`
- [x] **Milestone 6** (`correctness6-writev`)

### Phase 7 — HTTP/2 + gRPC

Checklist: [phases/phase-7.md](phases/phase-7.md) · Plan: [phases/phase-7-implementation-plan.md](phases/phase-7-implementation-plan.md)

- [x] Hand-rolled frame parser + `(tgid, fd, stream_id)` correlator
- [x] HPACK static+Huffman for `:method` / `:path` / `:status`
- [x] Cleartext h2c gRPC `/slow` correctness (p50 band)
- [x] HTTP/2 over OpenSSL (`correctness7-h2-tls`)
- [x] **Milestone 7**

### Phase 8 — Dual-plane TLS + handshake (original v3)

Checklist: [phases/phase-8.md](phases/phase-8.md) · Plan: [phases/phase-8-implementation-plan.md](phases/phase-8-implementation-plan.md)

- [x] Timing-only sock events on TLS fds; content stays `TlsIo`
- [x] Join window; content p50 gate + join rate
- [x] `SSL_do_handshake` histogram
- [x] `tls_unmapped` counter; optional server-side TLS
- [x] **Milestone 8**

### Phase 9 — OTLP traces + Grafana

Checklist: [phases/phase-9.md](phases/phase-9.md) · Plan: [phases/phase-9-implementation-plan.md](phases/phase-9-implementation-plan.md)

- [x] Sampled OTLP spans from exchanges
- [x] Collector `/v1/traces` 2xx (`smoke9`)
- [x] Grafana queries match exported stems (live screenshot is handoff, not CI)
- [x] **Milestone 9** (smoke9 + live PNG `docs/handoff/artifacts/grafana-vision95.png` 2026-08-25)

### Phase 10 — Real-node service map

Checklist: [phases/phase-10.md](phases/phase-10.md) · Plan: [phases/phase-10-implementation-plan.md](phases/phase-10-implementation-plan.md)

- [x] ClusterIP → Service `ns/name` (unit; not EndpointSlice)
- [x] VM/k3s (or equivalent): BPF tgid ∈ mounted `/proc` **or** cgroup-id identity (WSL)
- [x] Named src **and** dst for demo (`frontend` → `api`) on a real node
- [x] **Milestone 10** (`K3S_E2E` / evidence `docs/handoff/artifacts/e2e-k3s.pass` 2026-08-24)

### Phase 11 — Overhead + sampling

Checklist: [phases/phase-11.md](phases/phase-11.md) · Plan: [phases/phase-11-implementation-plan.md](phases/phase-11-implementation-plan.md)

- [x] Pinned load: 500 RPS HTTP/1.1 + 200 RPS h2/gRPC, `perf stat`, 3 runs (WSL2 k3s 2026-08-25; ClusterIP + `api`×3)
- [x] In-kernel 1/N sampling when drops &gt; 0 — overload gate: `sample_n=2`, `drops=68068`
- [x] **&lt;2% of one core** on that load **or** fail the wording → **failed wording**: mean **~87%** of one core (`docs/overhead.md`)
- [x] **Milestone 11** (measured; claim must not say under 2%)

### Phase 12 — CPU profiles + claim lock

Checklist: [phases/phase-12.md](phases/phase-12.md) · Plan: [phases/phase-12-implementation-plan.md](phases/phase-12-implementation-plan.md)

- [x] 99 Hz `perf_event` + blazesym; join stacks to slow spans
- [x] `/slow` named-function hit rate documented (`prof_hit=11/20`)
- [x] Verifier log ≥ 2 real pastes
- [x] README / resume sentence rewritten to proven claims (~87% of one core)
- [x] **Milestone 12 — 95% claim lock** (`docs/handoff/SESSION-VISION-95.md`) **10/10**

## Interview prep checklist

Interview pack lives **local-only** under `docs/interview/` (gitignored; not on GitHub).

- [ ] Whiteboard FSM for fd-keyed correlation
- [x] 2–3 real verifier rejection stories in the log (`docs/verifier-rejection-log.md`)
- [ ] TLS security tradeoff spoken without prompting
- [ ] RingBuf drop-counter policy explained with rationale
- [ ] Kind-in-Docker PID/`unknown` labeled; OTLP 400 scar (JSON + `bucketCounts`); k3s named edge
- [x] Never claim &lt;2% CPU — use **~87% of one core**; HTTP/2/gRPC **are** claim-locked
- [ ] Rust: `no_std` ABI, `Arc<Mutex<Correlator>>`, drain vs export `.await`, `decode.rs` unsafe ritual (not “I don’t know Rust”)
