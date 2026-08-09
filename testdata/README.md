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

Self-signed HTTPS fixtures (Phase 3 Q9):

| File | Role |
|---|---|
| `https-latency-server.py` | stdlib `ssl` HTTPS server (system OpenSSL / libssl) |
| `https-probe.py` | stdlib `ssl` client — same CLI shape as `http-probe` |

Must **not** use rustls. CPython on this host imports `SSL_write_ex` / `SSL_read_ex` + `SSL_set_fd`
(see Phase 3 TDD evidence). Generate certs in smoke/correctness scripts via `openssl req -x509`.

See [phase-3-implementation-plan.md](../phases/phase-3-implementation-plan.md) Q9.
