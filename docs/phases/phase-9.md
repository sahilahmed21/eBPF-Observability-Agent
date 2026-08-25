# Phase 9 — OTLP traces + Grafana

Plan: [phase-9-implementation-plan.md](phase-9-implementation-plan.md).  
North star: [VISION-95.md](VISION-95.md) Q10, Q20.  
Evidence: [../testing/phase-9.tdd.md](../testing/phase-9.tdd.md).

**Buys:** reconstructed **traces** (sampled spans from exchanges) and a Grafana dashboard whose PromQL matches **exported** metric names.

**Blocked on:** Milestone 8 (spans carry protocol + optional wire/handshake).

## Checklist

- [x] One root span per HTTP/1.1 exchange or h2/gRPC stream (own trace; no W3C inject)
- [x] `traceId` / `spanId` = splitmix64 of `(tgid, fd, stream_id, t_start_ns)` — no hash crate
- [x] Handshake is a **one-shot attribute** `obsagent.tls.handshake_ns` on the first sampled span for that `(tgid, fd)` when handshake `ts_ns ≤ t_start_ns` (not a child span)
- [x] OTLP/HTTP JSON POST `/v1/traces`; drain never `.await`s collector
- [x] Head sample default 1/10; `OBSAGENT_TRACE_SAMPLE=1` always-on
- [x] Counters `obsagent.traces.sampled` / `not_sampled` / `dropped` (headless `trace_s` / `trace_ns` / `trace_drop`)
- [x] Collector traces pipeline (logging exporter); `smoke9` 2xx gate
- [x] Grafana: HTTP p50/p99, gRPC p50/p99, handshake, drops, traces, service-map table — queries use exported stems
- [x] Histogram label `http` / `h2` / `grpc` unchanged; span `obsagent.protocol` maps `http` → `http/1.1`
- [x] No prefixes/headers in span attributes (`redact_headers` / no `:authorization`)

## Milestone 9

Collector accepts traces (2xx). Grafana panels match **actual** metric names. Handoff screenshot of HTTP + gRPC panels is evidence, not CI.

**Status:** PASS 2026-08-19 (review fixes). `test-agent` 123; `smoke9` `/v1/traces` 2xx and no OTLP HTTP/TCP rows. Grafana live screenshot is handoff, not CI. Uncommitted.
