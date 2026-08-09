# Roadmap & phase checklist

Use this as the execution checklist. Update status as milestones land.

## Phase 0 — Foundations (next)

- [ ] Confirm BTF: `/sys/kernel/btf/vmlinux` exists on the primary dev host
- [ ] Install stable + nightly (`rust-src`), `bpf-linker`
- [ ] Read [Aya book](https://aya-rs.dev/book/) (concepts before custom code)
- [ ] Scaffold Aya workspace (`common`, `ebpf`, `agent`, `xtask`)
- [ ] Load trivial kprobe; `aya-log` output; clean unload
- [ ] **Milestone 0** complete

## Phase 1 — Syscall latency MVP

- [ ] Tracepoints: connect enter/exit
- [ ] Tracepoints: accept4 enter/exit
- [ ] Pending HashMap + RingBuf events
- [ ] Socket metadata (CO-RE sock fields preferred)
- [ ] Tokio RingBuf consumer
- [ ] Ratatui dashboard (p50/p95/p99, top talkers)
- [ ] Overhead baseline recorded in `overhead-measurements.md`
- [ ] **Milestone 1** complete

## Phase 2 — HTTP awareness

- [ ] Bounded prefix capture on read/write for tracked TCP fds
- [ ] Per-socket correlation FSM
- [ ] `httparse` userspace parsing + graceful truncation
- [ ] Path normalization heuristics
- [ ] `hdrhistogram` per-endpoint table in CLI
- [ ] Document pipelining mis-pair failure mode
- [ ] **Milestone 2** complete

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

- [ ] Service map graph in memory
- [ ] PID → cgroup → container → pod/namespace identity
- [ ] OTLP traces + metrics
- [ ] Grafana dashboard
- [ ] Dockerfile + DaemonSet (capabilities, BTF mounts)
- [ ] kind/minikube demo with multi-service app
- [ ] **Milestone 4** complete

## Stretch

- [ ] S1: HTTP/2 frame demux + gRPC `:path`
- [ ] S2: perf_event stacks + blazesym + merge onto slow requests

## Interview prep checklist

- [ ] Whiteboard FSM for fd-keyed correlation
- [ ] 2–3 real verifier rejection stories in the log
- [ ] TLS security tradeoff spoken without prompting
- [ ] RingBuf drop-counter policy explained with rationale
