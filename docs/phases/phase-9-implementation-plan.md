# Phase 9 — Implementation Plan (Traces + Grafana)

Companion to [phase-9.md](phase-9.md). Locked: [VISION-95.md](VISION-95.md) Q10, Q13, Q20.

**Phase 9 buys:** original “reconstruct application-level traces” + a showable Grafana. Metrics from Phase 5 stay.

**Gate from Phase 8:** dual-plane + handshake fields available as span attributes.

---

## 0. Out of Phase 9

Tempo HA, tail-based sampling, injecting `traceparent` into target apps, always-on 100% in production default, real-node identity (Phase 10).

---

## 1. Locked decisions

| ID | Choice |
|---|---|
| **P9-Q1** | Span per completed exchange/stream. Name = `{METHOD} {route}` (gRPC: `:path`). |
| **P9-Q2** | `traceId` / `spanId` = 16/8 bytes from **splitmix64** of `(tgid, fd, stream_id, t_start_ns)`. Deterministic for that exchange only. **No blake3/sha256 crate.** |
| **P9-Q3** | Attributes: `src`, `dst`, `http.method`, `http.route`, `http.status_code` or `rpc.grpc.status_code`, `obsagent.protocol` = `http/1.1`\|`h2`\|`grpc`, optional `obsagent.wire_latency_ns`, optional `obsagent.tls.handshake_ns`. **No raw prefixes.** Histogram label `http` is unchanged. |
| **P9-Q4** | Handshake: **attribute** on the first sampled span for `(tgid, fd)` if handshake `ts_ns ≤ t_start_ns`, then consume. **Not** a child span (handshake ends before HTTP starts; parent/child times would be invalid). |
| **P9-Q5** | Export: OTLP/HTTP JSON `/v1/traces` to `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` or same host as metrics with `/v1/traces`. Same 10 s tick **or** flush spans in the existing export task. |
| **P9-Q6** | Head sample 1/10 (`OBSAGENT_TRACE_SAMPLE` = N means 1/N; `1` = always). Decision at exchange complete, **sync** on drain thread (no await). |
| **P9-Q7** | Bounded span queue (e.g. 1024). Drop + `otlp_dropped` / `traces_dropped`. Never block RingBuf drain. |
| **P9-Q8** | Collector: add `otlp` traces receiver if missing; debug/logging exporter **or** Jaeger all-in-one in `deploy/`. |
| **P9-Q9** | Grafana dashboard JSON updated to real names; provision notes in `deploy/grafana/dashboards/README.md`. |
| **P9-Q10** | Unit test: `to_otlp_traces_json()` parses; duration_ns = t_end − t_start; sampled=false omitted from POST body. |

---

## 2. Architecture

```text
handle_exchange / H2Exchange
    → MetricsRegistry.record (existing, always)
    → if sample: SpanQueue.push (sync)
export task every 10s:
    snapshot metrics → POST /v1/metrics
    drain span queue → POST /v1/traces
```

New: `agent/src/trace_export.rs` (or extend `export.rs`). Keep `ExportHub` as the only I/O.

---

## 3. Subphases

### 9.0 — Preconditions

M8 green. Collector metrics path still 2xx on kind/local.

---

### 9.1 — Span JSON + unit test

**Work:** Build OTLP traces JSON (resource + scopeSpans). Test duration, status, protocol.

**Verify:** unit test; invalid JSON impossible (closed braces — remember M5 400).

---

### 9.2 — Sample + queue + export task

**Work:** Wire from `handle_exchange` and h2 path. Env sample rate. Counters.

**Verify:** `test-agent`; with `OBSAGENT_TRACE_SAMPLE=1` a local collector returns 2xx (or recorded 400 body if fail — fix like M5).

---

### 9.3 — Collector + visibility

**Work:** Update `deploy/k8s/otel-collector.yaml` (traces pipeline). Local docker-compose optional. Logging exporter sufficient for the gate.

**Verify:** one span in collector log or Jaeger UI. Handoff paste.

---

### 9.4 — Grafana

**Work:** Panels for HTTP, gRPC, handshake, `events_dropped`, `otlp_dropped`, traces sampled, service-map table. Deny-list assumed on in the scrape used for screenshots.

**Verify:** dashboard JSON queries **match** exported names (grep metric names). Screenshot in handoff (not committed if huge; path noted).

---

### 9.5 — Close

Tick phase-9.md; `phase-9.tdd.md`; `wsl-run.sh smoke9` if useful (collector 2xx).

---

## 4. Success criteria

1. POST `/v1/traces` 2xx.
2. One human-visible span for `/slow` or gRPC Sleep.
3. Grafana HTTP + gRPC panels on allow-listed scrape.
4. Drain does not await collector.

---

## 5. Appendix — §10

| ID | Answer | Date | Notes |
|---|---|---|---|
| P9-Q2 | splitmix64 pack+mix; no hash crate | 2026-08-19 | 16/8 bytes; zero-id avoided |
| P9-Q4 | handshake **attribute**, not child | 2026-08-19 | `obsagent.tls.handshake_ns`; consume-once; 60 s TTL |
| P9-Q3 | span protocol `http/1.1`; hist label stays `http` | 2026-08-19 | strip `?query` on span route |
| P9-Q7 | drop **incoming** at 1024; no POST retry | 2026-08-19 | `traces_dropped` + `otlp_dropped` |
