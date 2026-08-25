# Phase 8 TDD evidence

**Source plan:** [phase-8-implementation-plan.md](../phases/phase-8-implementation-plan.md)  
**Architecture:** content-primary `DualPlane`; time-pruned per-fd Write/Read timestamps (not a count-capped ring); HTTP/1.1 wire join only; labeled `http.client.wire.duration`; `SSL_do_handshake` only.  
**Status:** unit GREEN + `correctness8-dual` PASS 2026-08-19 post-review. Uncommitted.

## Task → test mapping

| Plan | Test target | GREEN |
|---|---|---|
| 8.1 ABI | `common` kinds 5–6 | unchanged |
| 8.2 join | `later_writes_do_not_evict_request_half` | 40 extra Writes after request half still Hit |
| 8.2 join | `join_hit_rate_excludes_fallback` | fallback is not a hit |
| 8.2 join | `join_counts_follow_timeout_window` | 60s prune |
| 8.2 join | `sequential_exchanges_keep_distinct_halves` | two HTTP/1.1 exchanges on one fd |
| 8.2 join | `ticket_read_inside_end_window_is_selected` | residual: latest Read in window wins |
| 8.2 wire | `http_agg::wire_p50_is_per_endpoint` | `/slow` vs `/fast` |
| 8.2 wire | `metrics_registry::wire_histogram_carries_route_labels` | OTLP attributes on wire hist |
| 8.2 gate | `wsl-run.sh correctness8-dual` | handshake required; wire_p50 on `GET /slow` row; hit rate not fallback |
| 8.3 handshake | same gate | FAIL if `hs_p50` missing |
| 8.4 unmapped | BPF `UNMAPPED_SEEN` | once per `SSL*` (no userspace unit) |

## Commands

```bash
wsl-run.sh test-agent
wsl-run.sh build
wsl-run.sh correctness8-dual
```

## Results

| Command | Result | Date |
|---|---|---|
| `test-agent` | 99 passed (incl. flood-join RED→GREEN) | 2026-08-19 |
| `correctness8-dual` | PASS content 51.31ms join hit 100% (5/5) wire_p50=51.35ms on GET /slow hs_p50=3.19ms unmapped=0 | 2026-08-19 |
| `correctness8-dual` (claim HEAD `da6a862`) | **PASS** content p50 in band, join 100%, wire on `/slow`, handshake present | 2026-08-24 — [../handoff/artifacts/logs/correctness8-dual.log](../handoff/artifacts/logs/correctness8-dual.log) |
