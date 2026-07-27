# Correctness tests (Phase 1+)

Plan:

1. Start `demos/http-server` with known `?ms=` latency.
2. Attach agent; generate N requests.
3. Assert reported percentile within tolerance (clock + probe jitter).
