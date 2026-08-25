# Phase 7 — HTTP/2 + gRPC

Plan: [phase-7-implementation-plan.md](phase-7-implementation-plan.md).  
North star: [VISION-95.md](VISION-95.md) Q1–Q4.  
Evidence: [../testing/phase-7.tdd.md](../testing/phase-7.tdd.md).

**Buys:** the word **gRPC** — unary RPC latency from HTTP/2 frames + `:path`, cleartext h2c and OpenSSL h2. Per-method latency, not a call graph (that is Phase 10).

**Blocked on:** Milestone 6.

## Checklist

- [x] Hand-rolled HTTP/2 frame parser (9-byte header; HEADERS/DATA/RST/GOAWAY)
- [x] Detect h2 (preface / frames); mark fd; **HTTP/1.1 FSM must not run**
- [x] Correlator key `(tgid, fd, stream_id)`
- [x] HPACK static table + Huffman + 32-entry dynamic table for `:method`, `:path`, `:status`, `:authority`
- [x] gRPC method = `:path` (`/package.Service/Method`); no protobuf decode
- [x] Testdata: h2c gRPC `/slow` 50 ms (tonic)
- [x] `wsl-run.sh correctness7-grpc` p50 in band
- [x] HTTP/2 over OpenSSL: `wsl-run.sh correctness7-h2-tls`
- [x] OTLP/TUI series labeled `grpc` / `h2` (metrics path from Phase 5)
- [x] Document residuals: CONTINUATION (bounded leftover), trailers-only, unbounded HPACK dynamic table, skip-cursor leftover (8 KiB store, skip DATA)

## Milestone 7

Two concurrent streams do not mis-pair. Unary gRPC `/slow` p50 in `max(±10 ms, ±10%)`. One unary RPC over OpenSSL h2 in band.

**Status:** unit tests green; e2e gates PASS — Milestone 7 ticked.
