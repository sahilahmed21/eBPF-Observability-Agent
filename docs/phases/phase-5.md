# Phase 5 — Production Hardening

Plan: [phase-5-implementation-plan.md](phase-5-implementation-plan.md).

## Checklist

- [x] MetricsRegistry: cumulative explicit-bucket histograms
- [x] Export rewritten (no gauge-per-sample)
- [x] Agent self-metrics (`events_dropped`, `otlp_dropped`, `edges`)
- [x] Peer cache + cmdline identity fallback
- [x] Grafana dashboard JSON
- [x] `smoke4` gate (+ optional `KIND_E2E=1`)
- [x] Traces deferred (Q5)

## Milestone 5

Cumulative OTLP histograms + smoke4 PASS; kind e2e when `KIND_E2E=1`.
