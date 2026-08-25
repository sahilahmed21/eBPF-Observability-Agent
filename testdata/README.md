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

## grpc-slow (Phase 7)

Tonic **h2c** (cleartext) **server**. The probe is a single `write_all` of preface + HEADERS + DATA so the 256 B first-iovec cap (Q8) contains `:path`. tonic's own client uses `writev` and hides HEADERS in iov[1+].

```
cargo build --release -p grpc-slow
```

Needs vendored `protoc` via `protoc-bin-vendored` (no host `protoc` required).

## h2-tls (Phase 7)

`h2-tls-server.py` / `h2-tls-probe.py` — stdlib `ssl` ALPN `h2`, hand-rolled frames, **libssl not rustls**.

See [phase-3-implementation-plan.md](../phases/phase-3-implementation-plan.md) Q9.
