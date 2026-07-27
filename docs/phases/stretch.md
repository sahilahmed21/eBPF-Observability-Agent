# Stretch goals

Only after Phase 4 is solid.

## S1 — gRPC / HTTP/2

- Parse 9-byte HTTP/2 frame header; demux by stream ID.
- gRPC: 5-byte length prefix inside DATA; method from `:path` (`/package.Service/Method`).
- Hand-roll frame parse preferred over pulling full `h2` (interview clarity).

## S2 — Continuous CPU profiling

- `perf_event` sampling (~99 Hz); symbolize (`blazesym`).
- Merge stack samples with slow-request timestamps on same PID.
