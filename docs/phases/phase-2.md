# Phase 2 — HTTP awareness

## Checklist

- [x] Capture bounded prefixes on socket `read`/`write` (+ `sendto`/`recvfrom` per Q2 revision) — `ebpf/src/main.rs`
- [x] Per-socket correlation state machine (`docs/architecture/correlation.md`) — `agent/src/correlate.rs`
- [x] Userspace `httparse`; graceful truncated headers — `agent/src/http.rs`
- [x] Path normalization (`/user/123` → `/user/:id`) — `agent/src/http.rs`
- [x] `hdrhistogram` per endpoint — `agent/src/http_agg.rs`
- [x] CLI: endpoint table (latency, rate, 4xx/5xx %) — dual/toggle with TCP (`t`)
- [x] Correctness test vs injectable-latency server — `scripts/correctness-phase2.sh` PASS
- [x] Overhead row for Phase 2 — `docs/overhead.md` (above &lt;2% on sampled load; see notes)

## Milestone 2

Point at local HTTP server (nginx / Axum); per-endpoint latency with zero changes to that server.

**Achieved:** `testdata/latency-server` (Axum) + `http-probe`; smoke2 + correctness evidence in `docs/testing/phase-2.tdd.md`.
