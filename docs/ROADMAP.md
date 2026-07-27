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

## Phase 3 — TLS interception

- [ ] Resolve `libssl` from `/proc/<pid>/maps`
- [ ] Uprobes: `SSL_write` / `SSL_read` (1.1 and 3)
- [ ] TLS↔syscall correlation + dedup
- [ ] Redaction pass (Authorization, Cookie, secret patterns)
- [ ] Design note: trust boundary / privileges
- [ ] **Milestone 3** complete

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
