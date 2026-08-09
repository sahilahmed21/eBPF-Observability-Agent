# `testdata/`

Deterministic HTTP server with injectable latency for Phase 2 correctness checks.

## latency-server

Axum binary (`cargo build --release -p latency-server`):

| Path | Behavior |
|---|---|
| `GET /fast` | 200 immediately |
| `GET /slow?delay_ms=N` | sleep N ms then 200 (default 50) |
| `GET /users/:id` | 200 + body |
| `GET /err` | 500 |

`PORT` env (default `18080`).

Probe with `testdata/http-probe` (`cargo build --release -p http-probe`) — uses
`std::io::{Read,Write}` which is `sendto`/`recvfrom` on glibc (see Phase 2 Q2 revision).

Self-signed cert fixtures: Phase 3.
