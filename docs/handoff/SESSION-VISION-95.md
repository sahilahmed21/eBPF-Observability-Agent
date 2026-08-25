# VISION-95 claim lock

**Fill at Milestone 12.** Do not tick PASS without an evidence path **on the current HEAD**.  
**Updated:** 2026-08-25 — **10/10 PASS**. Overhead claim uses measured **~87% of one core** (not &lt;2%).

Source: [../phases/VISION-95.md](../phases/VISION-95.md) §11 · [phase-12-implementation-plan.md](../phases/phase-12-implementation-plan.md)  
Closure runbook: [SESSION-CLAIM-LOCK-CLOSURE.md](SESSION-CLAIM-LOCK-CLOSURE.md)  
Findings log: [SESSION-CLAIM-FINDINGS-2026-08-24.md](SESSION-CLAIM-FINDINGS-2026-08-24.md)

| # | Evidence | Unlocks | PASS | Link |
|---|---|---|---|---|
| 1 | `correctness6` + split-header | real syscall I/O | **PASS** | [artifacts/logs/correctness6.log](artifacts/logs/correctness6.log) — p50=50.69ms, `count=7`, `reasm=254`; ([phase-6.tdd.md](../testing/phase-6.tdd.md)) |
| 2 | `correctness7-grpc` + `correctness7-h2-tls` | gRPC / HTTP/2 | **PASS** | [artifacts/logs/correctness7-grpc.log](artifacts/logs/correctness7-grpc.log) · [correctness7-h2-tls.log](artifacts/logs/correctness7-h2-tls.log) · [phase-7.tdd.md](../testing/phase-7.tdd.md) |
| 3 | `correctness8-dual` + handshake | dual-plane v3 | **PASS** | [artifacts/logs/correctness8-dual.log](artifacts/logs/correctness8-dual.log) · [phase-8.tdd.md](../testing/phase-8.tdd.md) |
| 4 | OTLP traces 2xx + Grafana screenshot | traces | **PASS** | [artifacts/logs/smoke9.log](artifacts/logs/smoke9.log) · [artifacts/grafana-vision95.png](artifacts/grafana-vision95.png) · [artifacts/dashboard2.png](artifacts/dashboard2.png) · [phase-9.tdd.md](../testing/phase-9.tdd.md) |
| 5 | k3s named `frontend → api` | service map | **PASS** | [artifacts/e2e-k3s.pass](artifacts/e2e-k3s.pass) · [phase-10.tdd.md](../testing/phase-10.tdd.md) · [artifacts/logs/e2e-k3s.log](artifacts/logs/e2e-k3s.log) |
| 6 | Vision-95 `perf stat` row | 2% or honest % | **PASS** (honest %) | [docs/overhead.md](../overhead.md) Phase 11 — mean **~87%** of one core (0.9 / 0.8 / 0.9); pin 500+200 met; [artifacts/logs/overhead-run1.log](artifacts/logs/overhead-run1.log) … `run3.log` |
| 7 | overload sampling | backpressure | **PASS** | [artifacts/logs/overload-vision95.log](artifacts/logs/overload-vision95.log) — vegeta toward 5k (achieved ~1k/s); agent `drops=68068` **`sample_n=2`**; RSS flat (~31→29 MiB); HTTP still 200s |
| 8 | `correctness12` hit rate | profiles | **PASS** | [artifacts/logs/correctness12.log](artifacts/logs/correctness12.log) — `prof_hit=11` / N=20; busy-wait `slow_handler_sleep` (CPU-clock samples need on-CPU work) |
| 9 | verifier log ≥ 2 | verifier hardness | **PASS** | [docs/verifier-rejection-log.md](../verifier-rejection-log.md) — 2× stack-limit pastes (`emit_io_kind`, `profile_sample`); Q18 force; artifacts `vrej-build-stack600.log` / `vrej-build-getstack600.log` |
| 10 | README/resume rewritten | stop lying | **PASS** | Public README uses **~87% of one core**, not &lt;2%; resume bullets kept local under `docs/interview/` (gitignored) |

**Claim lock: 10/10 PASS.** Legal resume sentence uses the **measured** overhead wording below. Residuals (Go TLS, rustls, XDP, multi-cluster, Pixie parity) stay out.

**Legal VISION-95 sentence (post-lock):**  
> Built a zero-instrumentation observability agent in Rust + Aya (eBPF) that reconstructs per-service HTTP/gRPC latency and a live named service map on a real k3s node, exports OTLP metrics/traces, joins CPU stacks to slow spans, and measures **~87% of one core** under the Vision-95 pin load (HTTP 500/s + gRPC ~200/s) — not under 2%.

**Row 6 measured X% ≥ 2:** sentence must use X% (**~87% of one core**), not 2%.
