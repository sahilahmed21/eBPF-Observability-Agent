# HTTP demo server (Phase 2+)

Planned: small Axum (or similar) service with:

- `GET /health`
- `GET /slow?ms=N` — sleeps N ms for correctness tests
- `GET /user/:id` — for path-normalization checks

No APM SDK. The eBPF agent must see traffic without code changes here.
