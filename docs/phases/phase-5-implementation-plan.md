# Phase 5 — Implementation Plan (Production Hardening)

Companion to Phase 5 architecture review (2026-08-12/13). Defaults accepted.

**Phase 5 buys:** honest cumulative OTLP histograms, agent self-metrics, label hygiene,
Grafana dashboard, kind/`smoke4` gate. Does **not** start Stretch (HTTP/2 / CPU).

---

## Locked decisions

| ID | Choice |
|---|---|
| **Q1 Histogram** | Explicit buckets (ms): `1,2,5,10,25,50,100,250,500,1000,2500,5000,10000` (+Inf) |
| **Q2 Export** | In-process `MetricsRegistry` aggregates; flush cumulative OTLP/HTTP JSON histograms+sums. SDK optional; payload must be cumulative hist (not gauge-per-sample) |
| **Q3 Series key** | `(src, dst, http.method, http.route, status_class)` with cap → `_other` |
| **Q4 kind/smoke4** | Required Milestone 5 gate |
| **Q5 Traces** | Out of M5 |
| **Q6 Cardinality** | Max 2048 HTTP series; overflow bucket |
| **Q7 Flush** | Periodic snapshot flush; never block RingBuf drain; soft-fail collector |
| **Q8 Peer cache** | Userspace TTL cache of `(tgid,fd)→peer` to reduce unknown dst |
| **Q9 Identity** | `comm` empty → cmdline basename fallback before `unknown` |

---

## Explicit non-goals

HTTP/2, CPU profiles, dual-plane TLS, always-on traces, ClusterIP Endpoints (unless kind demo needs it).
