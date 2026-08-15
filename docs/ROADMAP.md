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

## Stretch

- [ ] S1: HTTP/2 frame demux + gRPC `:path`
- [ ] S2: perf_event stacks + blazesym + merge onto slow requests

## Interview prep checklist

- [ ] Whiteboard FSM for fd-keyed correlation
- [ ] 2–3 real verifier rejection stories in the log
- [ ] TLS security tradeoff spoken without prompting
- [ ] RingBuf drop-counter policy explained with rationale
