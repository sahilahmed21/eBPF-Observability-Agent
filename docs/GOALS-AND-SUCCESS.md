# Goals achieved — why this project is complete

**Status:** **Complete** against the VISION-95 / Milestone 12 definition of done.  
**Date locked:** 2026-08-25 · **Claim lock:** [10/10 PASS](handoff/SESSION-VISION-95.md)  
**Environment:** WSL2 Ubuntu + k3s (node `collosal`)

This document states what we set out to build, what we proved with gates, and how “success” is defined — including the one original target we **did not** hit and how we handled it honestly.

---

## 1. Verdict

| Question | Answer |
|---|---|
| Is the project a **success**? | **Yes** — zero-instrumentation HTTP/gRPC observability, real-node service map, OTLP export, overload sampling, optional CPU profiles, all claim-locked with evidence. |
| Is it **complete**? | **Yes** for the 95% brief (Phases 0–12 + claim lock). The remaining ~5% is explicit non-scope (below), not unfinished core. |
| Can we claim “under 2% CPU”? | **No.** Measured pin-load cost is **~87% of one core**. Success includes **telling the truth** about that number. |

**One-line outcome:**

> Built a Rust + Aya eBPF agent that reconstructs HTTP/gRPC latency and a live named service map on a real k3s node, exports OTLP metrics/traces, joins CPU stacks to slow spans, and measures **~87% of one core** under the Vision-95 pin load — not under 2%.

---

## 2. Original goals → achieved?

These map to the north-star brief in [phases/VISION-95.md](phases/VISION-95.md) §0.

| # | Goal | Status | How we know |
|---|---|---|---|
| 1 | Zero instrumentation (no app SDK) | **Done** | Syscall tracepoints + OpenSSL uprobes only |
| 2 | Reconstruct **HTTP** latency | **Done** | Correctness gates; `/slow` 50 ms → p50 ≈ **50.7 ms** |
| 3 | Reconstruct **gRPC / HTTP/2** latency | **Done** | `correctness7-grpc` + `correctness7-h2-tls` PASS |
| 4 | Capture modern I/O (`writev` / split headers) | **Done** | Phase 6 gate; reassembly counters move |
| 5 | TLS plaintext (OpenSSL) without app changes | **Done** | Uprobes; dual-plane content × wire (Phase 8) |
| 6 | TLS **handshake** timing | **Done** | `SSL_do_handshake` events; dual gate |
| 7 | Live **named** service map on a **real** node | **Done** | k3s e2e: `demo/frontend` → `demo/api` |
| 8 | OTLP **metrics + traces** + readable Grafana | **Done** | smoke9 + dashboard screenshots |
| 9 | Survive overload (visible drops + sampling) | **Done** | `drops=68068`, `sample_n=2`, RSS flat |
| 10 | CPU stacks on slow request timeline | **Done** | `prof_hit=11/20` (≥50% gate) |
| 11 | Document verifier hardness | **Done** | ≥2 real stack-limit pastes |
| 12 | Public claims match evidence | **Done** | README rewritten; no fake &lt;2% |
| — | Stay **under 2% CPU** on pin load | **Missed (honest)** | `perf` ×3 → mean **~87% of one core**; claim that instead |

---

## 3. Milestone ladder (what “done” meant at each stage)

| Milestone | Intent | Outcome |
|---|---|---|
| M0–M1 | Toolchain, connect/accept latency CLI | Shipped |
| M2–M3 | Cleartext HTTP + OpenSSL HTTPS | Shipped |
| M4–M5 | DaemonSet path, OTLP metrics foundation | Shipped |
| M6 | `writev` / reassembly completeness | Gate PASS |
| M7 | HTTP/2 + gRPC | Gate PASS |
| M8 | Dual-plane TLS + handshake | Gate PASS |
| M9 | Sampled traces + Grafana | Gate PASS |
| M10 | Named edges on real node | e2e PASS (cgroup-id identity on WSL) |
| M11 | Pin-load overhead + overload sampling | Measured ~87%; overload PASS |
| M12 | Profiles + claim lock 10/10 | **Closed** |

Evidence index: [handoff/SESSION-VISION-95.md](handoff/SESSION-VISION-95.md).

---

## 4. Why we call it a success (criteria)

Success was **not** “infinite product.” It was a locked definition:

1. **Capability** — Reconstruct request latency and topology without SDKs.  
2. **Proof** — Every claim row has a gate, log, or artifact on HEAD.  
3. **Integrity** — Failed &lt;2% overhead → publish the measured number.  
4. **Deployability** — DaemonSet + collector path that an outsider can follow from the README.  
5. **Engineering depth** — Backpressure, verifier constraints, TLS library boundary, k8s identity — demonstrated, not slideware.

If any of (2) or (3) were missing, we would **not** call the project complete.

---

## 5. Hard problems that count as wins

| Challenge | Outcome |
|---|---|
| No request IDs in the kernel | `(tgid, fd)` / `(tgid, fd, stream_id)` correlation |
| TLS ciphertext on the wire | OpenSSL uprobes + dual-plane timing |
| WSL BPF tgid ≠ host `/proc` | `cgroup_id` on events + cgroup index → named pods |
| RingBuf overload | Drop counters + sticky connection sampling |
| CPU profiles with `sleep` | On-CPU burn in `/slow` so samples hit the span |
| Vanity &lt;2% CPU | Refused; documented ~87% of one core |

---

## 6. Explicitly out of scope (not “incomplete”)

These were never part of the 95% lock. Leaving them undone does **not** reopen the project:

- Go `crypto/tls`, rustls, GnuTLS as primary TLS paths  
- XDP / Cilium Hubble / multi-cluster APM  
- Perfect HTTP/1.1 pipelining / full HPACK / CONTINUATION  
- Zero-drop always-complete traces  
- Pixie- or Datadog-parity product surface  
- Claiming **under 2% CPU**

---

## 7. Where to look for proof

| Artifact | What it shows |
|---|---|
| [README.md](../README.md) | Public demo + results table |
| [handoff/SESSION-VISION-95.md](handoff/SESSION-VISION-95.md) | Claim lock 10/10 checklist |
| [overhead.md](overhead.md) | Phase 11 `perf` numbers |
| [handoff/artifacts/grafana-vision95.png](handoff/artifacts/grafana-vision95.png) | Grafana |
| [handoff/artifacts/e2e-k3s.pass](handoff/artifacts/e2e-k3s.pass) | Named service-map e2e |
| [verifier-rejection-log.md](verifier-rejection-log.md) | Real BPF stack-limit rejects |
| [security.md](security.md) | Trust boundary / redaction |

---

## 8. Closing statement

This repository is a **finished systems spike with production-shaped gates**: eBPF capture, userspace reconstruction, Kubernetes identity, OTLP export, overload behavior, and optional profiling — all evidenced on a real node.

It is a **success** because the original product picture (minus the failed &lt;2% vanity target) is **implemented, measured, and claimed accurately**.

It is **complete** because Milestone 12 claim lock is **10/10**; further work is residual R&amp;D, not unfinished definition-of-done.
