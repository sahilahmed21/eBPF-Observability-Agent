# Phase 7 — Implementation Plan (HTTP/2 + gRPC)

Companion to [phase-7.md](phase-7.md). Locked: [VISION-95.md](VISION-95.md) Q1–Q4.

**Phase 7 buys one thing:** unary HTTP/2 / gRPC latency without an SDK. BPF stays dumb (prefixes only). Parser and stream FSM are userspace.

**Gate from Phase 6:** Milestone 6 green (writev + reassembly). Fd-only HTTP/1.1 FSM **will mis-pair** multiplexed streams — that is why this phase exists.

---

## 0. Inventory

| Done | Missing |
|---|---|
| Reassembly 8 KiB / `(tgid,fd)` | Frame demux |
| httparse HTTP/1.1 | HPACK |
| TlsIo plaintext prefixes | h2-over-TLS path using same parser |
| MetricsRegistry HTTP histograms | `grpc` protocol label |

### Out of Phase 7

Dual-plane (Phase 8), traces (Phase 9), real-node map (Phase 10), streaming RPC timelines, protobuf, grpc-web, full HPACK dynamic table (residual), CONTINUATION (residual), Go TLS.

---

## 1. Locked decisions

| ID | Choice |
|---|---|
| **P7-Q1** | Key = `(tgid, fd, stream_id)`. After h2 detect, **forbid** HTTP/1.1 `observe` on that fd. |
| **P7-Q2** | Detect: connection preface `PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n` **or** first 9 bytes look like a frame with valid type 0–9 and length ≤ 16 KiB. Optional: ALPN `h2` if we later see it (not required for M7). |
| **P7-Q3** | Hand-roll frames. **No** `h2` crate as parser of record. Tests may use recorded hex from RFC 7540 / grpcurl. |
| **P7-Q4** | Frame types: HEADERS, DATA, RST_STREAM (fail stream), GOAWAY (evict conn). SETTINGS / WINDOW_UPDATE / PING / PRIORITY: parse header, ignore payload. |
| **P7-Q5** | Latency = resp HEADERS `ts_ns` (END_HEADERS) − req HEADERS `ts_ns` (END_HEADERS). DATA does not define latency. |
| **P7-Q6** | HPACK: static table + Huffman + **dynamic cap 32** (VISION Q3 amended for M7 — tonic's default table). If `:path` missing → drop series. Not a spec-complete compressor. |
| **P7-Q7** | gRPC method = `:path`. Optional: count DATA frames with 5-byte gRPC prefix for `rpc.messages` (nice-to-have, not a gate). No protobuf. |
| **P7-Q8** | Testdata: **tonic** (or grpc-go) server, **h2c** (cleartext), `/package.Slow/Sleep` or equivalent, 50 ms sleep in handler. Client in testdata. |
| **P7-Q9** | Same p50 band as Phase 2/3/6. |
| **P7-Q10** | h2 over OpenSSL: same parser on `TlsIo` + reassembly. Testdata must `ldd` libssl (not rustls). |
| **P7-Q11** | Metrics: `http.protocol=h2` or `rpc.system=grpc` + route = `:path`. Reuse MetricsRegistry; add protocol label. Cardinality cap still 2048. |

### Open during execution

| # | Question | Blocks | How |
|---|---|---|---|
| **P7-Q12** | tonic vs grpc-go for testdata | 7.5 | **tonic h2c** (`testdata/grpc-slow`). h2-TLS testdata is Python `ssl` (libssl), not tonic+rustls. |
| **P7-Q13** | Do writev/sendmsg carry full frames on this stack? | 7.2 | h2 leftover (8 KiB/dir) joins split prefixes. HTTP/1.1 reassembler is not used on h2 fds. First-iovec 256 B is still Q8. |

---

## 2. Architecture

```text
SockIo / TlsIo prefix
        │
        ├─ already h2-marked OR preface/frame? ─► h2::feed  (leftover per dir)
        │                                              │
        │                                              ├─ stream_id N HEADERS req
        │                                              └─ stream_id N HEADERS resp → H2Exchange
        │
        └─ else HTTP/1.1 Reassembler → Correlator (unchanged)
```

**Do not parse HTTP/2 in BPF. Do not send h2 bytes through the HTTP/1.1 reassembler.**

### Modules

| Path | Role |
|---|---|
| `agent/src/h2/frame.rs` | 9-byte header + payload slice |
| `agent/src/h2/hpack.rs` | static + Huffman |
| `agent/src/h2/conn.rs` | `H2Conn`: map stream_id → pending |
| `agent/src/h2/mod.rs` | `feed(&[u8], ts_ns) -> Vec<H2Exchange>` |
| `agent/src/correlate.rs` | skip HTTP/1.1 if fd marked h2 |
| `common/` | optional `EventKind` unchanged; no new BPF event required |

Mark h2 fds in **userspace** `HashSet<(tgid,fd)>` first. Optional later: BPF map `H2_FDS` — not required for M7.

### H2Exchange (userspace)

```text
tgid, fd, stream_id, method, path, status, t_start_ns, t_end_ns, peer
protocol: Http2 | Grpc  // Grpc if path starts with '/' and contains '.' service pattern
                        // or content-type would be grpc — we only have :path; use path heuristic:
                        // path starts with '/' and has at least two segments package.Service/Method
```

---

## 3. Subphases

### 7.0 — Preconditions

**Work:** Milestone 6 ticked. `correctness6` PASS.

**Verify:** ROADMAP Phase 6 complete.

---

### 7.1 — Frame parser · unit

**Work:** `h2/frame.rs`. Parse length/type/flags/stream_id; reject truncated. Test vectors: empty SETTINGS, HEADERS with known payload (even if HPACK is stubbed).

**Verify:** `cargo test -p obsagent h2::frame` (or module tests) GREEN.

---

### 7.2 — Stream correlator · unit

**Work:** `H2Conn`. Two streams (id 1 and 3): req HEADERS then resp HEADERS each; assert two latencies, no cross-pair. RST_STREAM drops pending. GOAWAY clears conn.

**Verify:** unit test `two_streams_no_mispair`.

---

### 7.3 — HPACK subset · unit

**Work:** Static table indices for `:method GET/POST`, `:status 200`, Huffman decode for `:path`. Test: recorded bytes → `:path=/hello.Greeter/SayHello`.

**Verify:** that path string exact.

---

### 7.4 — Drain wire-up

**Work:** After reassemble, try h2 detect; if h2, `h2::feed` else HTTP/1.1. Mark fd. Feed `H2Exchange` into `HttpAggregator` + `MetricsRegistry` with protocol label. Redact: if `:authorization` appears in decoded headers, drop value (don’t log).

**Verify:** `test-agent` still 38+; new tests pass.

---

### 7.5 — h2c gRPC testdata + correctness

**Work:** `testdata/grpc-slow/` (tonic). `scripts/correctness-phase7-grpc.sh`. `wsl-run.sh correctness7-grpc`.

**Verify:** p50 in band; series path is the gRPC method; count = N probes.

---

### 7.6 — OpenSSL h2

**Work:** Same server with OpenSSL (tonic-openssl or nginx grpc + openssl, or Python http2+ssl if libssl). `scripts/correctness-phase7-h2-tls.sh`. `ldd` evidence.

**Verify:** `wsl-run.sh correctness7-h2-tls` PASS.

---

### 7.7 — Smoke + close

**Work:** `scripts/smoke-milestone7.sh` (h2c rows appear). Tick checklist. `docs/testing/phase-7.tdd.md`. Residuals in `docs/architecture/correlation.md` (update H2 section from “stretch” to “shipped with residuals”).

**Verify:** ROADMAP Phase 7 boxes.

---

## 4. Success criteria

1. Two-stream unit test: no mis-pair.
2. `correctness7-grpc` p50 in band.
3. `correctness7-h2-tls` p50 in band.
4. HTTP/1.1 correctness3/6 still PASS.
5. Never call this a “call graph.”

---

## 5. Appendix — §10

| ID | Answer | Date | Notes |
|---|---|---|---|
| P7-Q2 | Detect = preface **or** SETTINGS/PING/GOAWAY on stream 0 **or** WINDOW_UPDATE **or** HEADERS/RST/CONTINUATION on a nonzero stream. All-zero DATA is not a start. | 2026-08-19 | Q2 clarified; not a type-0–9 catch-all |
| P7-Q12 | tonic h2c (`testdata/grpc-slow`); OpenSSL h2 = Python `ssl` | 2026-08-18 | rustls unused |
| P7-Q13 | first-iovec 256 B residual; leftover 8 KiB **skip-cursor** (do not store DATA) | 2026-08-19 | Q8 unchanged; Q9 cap honored |
