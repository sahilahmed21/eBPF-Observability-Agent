# Phase 7 TDD evidence

**Source plan:** [phase-7-implementation-plan.md](../phases/phase-7-implementation-plan.md)  
**Status:** unit GREEN. e2e gates PASS (Milestone 7).

## User journeys

1. Unary gRPC latency without an SDK, from HTTP/2 `:path`.
2. Two concurrent streams on one fd do not swap latencies.

## Task → test mapping

| Plan | Test target | RED | GREEN |
|---|---|---|---|
| 7.1 frames | `agent/src/h2/frame.rs` | compile before modules existed | RFC-shaped header parse + preface vs GET; zeros are not h2 |
| 7.2 HPACK | `agent/src/h2/hpack.rs` | | `:path=/hello.Greeter/SayHello`; dynamic index 62; RFC 7541 C.4 Huffman; table-size 0; Huffman EOS/padding fail |
| 7.3 streams | `agent/src/h2/conn.rs` | | `two_streams_no_mispair`; RST; GOAWAY; timeout; TLS client_only; probe-shaped HPACK; 256 B slices; 16 KiB DATA skip; preface reset; `evict_closed`; unpaired status |
| 7.4 drain | `ingest_http_io` demux | | HTTP/1.1 unit tests still pass; h2 path is unit-tested via `H2Registry::feed` |
| 7.5 grpc | `wsl-run.sh correctness7-grpc` | live: `h2_xchg=0` until probe waited for HEADERS | PASS p50=52.43ms |
| 7.6 h2 tls | `wsl-run.sh correctness7-h2-tls` | live: `tlsio=0` when probe ran before uprobes attached | PASS p50=51.09ms `tlsio=50` |

## Commands

```bash
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh test-common
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh test-agent
wsl -d Ubuntu -u root -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh correctness7-grpc
wsl -d Ubuntu -u root -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh correctness7-h2-tls
```

## Results

| Command | Result | Date |
|---|---|---|
| `wsl-run.sh test-common` | 16 passed | 2026-08-18 |
| `wsl-run.sh test-agent` | 81 passed | 2026-08-19 |
| `wsl-run.sh correctness7-grpc` | PASS: `grpc POST /slow.Slow/Sleep` p50=52.43ms (band 40–60) | 2026-08-19 |
| `wsl-run.sh correctness7-h2-tls` | PASS: `h2 GET /slow` p50=51.09ms, `tlsio=50` | 2026-08-19 |
| `correctness7-grpc` (claim HEAD `da6a862`) | **PASS** p50 in band (`/slow.Slow/Sleep`) | 2026-08-24 — [../handoff/artifacts/logs/correctness7-grpc.log](../handoff/artifacts/logs/correctness7-grpc.log) |
| `correctness7-h2-tls` (claim HEAD) | **PASS** p50 in band (`h2 GET /slow`) | 2026-08-24 — [../handoff/artifacts/logs/correctness7-h2-tls.log](../handoff/artifacts/logs/correctness7-h2-tls.log) |

## e2e notes

Parser unit tests now include capture-shaped 256 B slices and skip-cursor over 16 KiB DATA. Live gRPC required the probe to **read until response HEADERS** (a single `read()` returned SETTINGS only). Live OpenSSL h2 required gates to wait for `obsagent ready` so uprobes are attached before the probe. Q8 (first iovec, 256 B) is unchanged.
