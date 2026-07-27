# Phase 2 — HTTP awareness

## Checklist

- [ ] Capture bounded prefixes on socket `read`/`write` (cross-ref Phase 1 fds)
- [ ] Per-socket correlation state machine (`docs/architecture/correlation.md`)
- [ ] Userspace `httparse`; graceful truncated headers
- [ ] Path normalization (`/user/123` → `/user/:id`)
- [ ] `hdrhistogram` per endpoint
- [ ] CLI: endpoint table (latency, rate, 4xx/5xx %)
- [ ] Correctness test vs injectable-latency server
- [ ] Overhead row for Phase 2

## Milestone 2

Point at local HTTP server (nginx / Axum); per-endpoint latency with zero changes to that server.
