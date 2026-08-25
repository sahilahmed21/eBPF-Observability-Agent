# Phase 9 TDD evidence

**Source plan:** [phase-9-implementation-plan.md](../phases/phase-9-implementation-plan.md)  
**Architecture:** one root span per exchange/stream; handshake **attribute** (not child);
splitmix ids; head sample 1/N; span queue cap 1024 drop-incoming; no POST retry;
Grafana PromQL stems must match scrape names.  
**Status:** review fixes in tree (self-ingest exclusion, handshake consume-after-queue, span kind from role, gRPC status, resource `src`). `smoke9` is the collector 2xx + no-self-series gate.

## Task → test mapping

| Plan | Test target | GREEN |
|---|---|---|
| 9.1 JSON | `trace_export::traces_json_parses_and_duration_matches` | valid JSON; duration; `http/1.1`; no Authorization |
| 9.1 status | `five_xx_is_error_status` | OTEL status ERROR iff ≥500 |
| 9.1 grpc | `grpc_uses_rpc_status_and_strips_query` | `rpc.grpc.status_code`; query stripped; handshake attr |
| 9.1 ids | `ids_are_deterministic` | same meta → same ids |
| 9.2 sample | `sample_n1_always_n10_not_all` | 1 = always; 10 ≈ 10% |
| 9.2 queue | `queue_drops_incoming_when_full` | cap drop incoming |
| 9.2 handshake | `handshake_consumed_once_if_preceding` | consume-once; future ts ignored |
| 9.2 TTL | `handshake_ttl_evicts` | 60 s |
| 9.2 hub | `export::sampled_exchange_enqueues_span` | strip query; handshake once; wire attr |
| 9.2 hub | `not_sampled_skips_queue` | `trace_ns` |
| 9.2 hub | `full_queue_counts_dropped` | `traces_dropped` |
| 9.1 grpc-err | `grpc_nonzero_status_is_error` | gRPC status ≠ 0 → OTEL ERROR |
| 9.1 kind | `server_role_is_server_kind` | SERVER when request half was Read |
| 9.1 resource | `traces_json_groups_resource_by_src` | `service.name` = reconstructed src |
| 9.2 handshake-q | `export::queue_full_does_not_consume_handshake` | peek then consume only on push ok |
| 9.2 B1 | `filter::obsagent_comm_always_denied` / `classify_ingest_*` | self tgid/comm; k8s `otelcol` |
| 9.3 collector | `scripts/smoke-milestone9.sh` | `/v1/traces` 2xx; `trace_s>0`; no OTLP HTTP/TCP rows |
| 9.4 Grafana | `grafana_queries_pinned_scrape_names` | pinned `_milliseconds_bucket` / `_total` |

## Commands

```bash
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh test-agent
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh build
wsl -d Ubuntu -u root -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh smoke9
```

## Results

| Command | Result | Date |
|---|---|---|
| `test-agent` | **123 passed** | 2026-08-19 |
| `smoke9` | **PASS** Grafana names; `trace_s=10`; sink `/v1/traces` 5967 B 2xx; `/v1/metrics` 4069 B; `otlp_ok=2`; no `POST /v1/*` HTTP rows; no `:14318` TCP | 2026-08-19 |
| `smoke9` (claim re-run, HEAD `da6a862`) | **PASS** `trace_s=234`; `/v1/traces` 2xx; self-tgid excluded | 2026-08-24 — [../handoff/artifacts/logs/smoke9.log](../handoff/artifacts/logs/smoke9.log) |
| Grafana live (claim row 4) | **PASS** — Prometheus scrapes collector `:8889` → Grafana on `:9090`; live panels | 2026-08-25 — [../handoff/artifacts/grafana-vision95.png](../handoff/artifacts/grafana-vision95.png) · [../handoff/artifacts/dashboard2.png](../handoff/artifacts/dashboard2.png) (post-DS-resume, ~01:05 activity) |

**Screenshot note:** panels show HTTP + gRPC + edges + trace sampling. Series include kube/Docker noise (`proc:unknown`, `/containers/…`); claim row 4 needs live panels. Optional polish: allow/deny before a resume-hero shot.
