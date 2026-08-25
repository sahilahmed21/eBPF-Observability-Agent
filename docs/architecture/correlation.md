# Request correlation (no request ID)

Interview question: *How do you correlate a kernel-level TCP event with an application-level HTTP request when you have no request ID?*

## Answer we implement

Kernel has no HTTP request ID. Correlation is **reconstructed** from structure:

**Primary key:** `(pid, fd)` plus optional **4-tuple** `(saddr, sport, daddr, dport)` once known.

On one non-pipelined HTTP/1.1 connection:

- **Client:** `write` (request) then `read` (response) on same fd.
- **Server:** `read` (request) then `write` (response) on same fd.

Within a bounded time window, that pair is almost always one exchange.

## Per-socket state machine

HTTP/1.1 (unchanged):

```
                 timeout / error
                        │
                        ▼
              ┌──────────────────┐
              │  Idle / Evicted  │
              └────────┬─────────┘
                       │ first half of exchange
                       ▼
              ┌──────────────────┐
   client:    │  RequestSent     │──write──┐
   server:    │  AwaitingRequest │◄─read───┤
              └────────┬─────────┘         │
                       │ matching other dir│
                       ▼                   │
              ┌──────────────────┐         │
              │ AwaitingResponse │◄────────┘
              └────────┬─────────┘
                       │ response bytes / status
                       ▼
              ┌──────────────────┐
              │ ResponseReceived │ → emit latency span → Idle
              └──────────────────┘
```

Stale states evicted by timeout (client disconnect, hung request).

HTTP/2 (Phase 7): demux **before** the HTTP/1.1 reassembler. Leftover bytes stay `(tgid, fd, dir)`. Pairing key is `(tgid, fd, stream_id)` from HEADERS (`:method` then `:status`). DATA is ignored for latency. Sticky INFLIGHT until GOAWAY / 60 s idle / deny / close.

```
preface or valid frame
        │
        ▼
   H2Registry leftover ──► HEADERS END_HEADERS
                                │
                    :method → pending[stream]
                    :status → H2Exchange (resp ts − req ts)
```

## TLS plane (Phase 3 / Milestone 3)

HTTPS encrypts in userspace, so Phase 2 sock I/O prefixes are ciphertext and useless.
Phase 3 hooks OpenSSL and **reuses this same `(tgid, fd)` state machine**:

1. `SSL_set_fd` (and rfd/wfd) → map `SSL*` → `fd`
2. `SSL_read` / `SSL_write` uprobes → bounded plaintext prefix (`TlsIo`)
3. Feed `TlsIo` into the existing correlator / httparse / aggregator

**Latency (M3):** TLS half exit→exit (same SM semantics as Phase 2 SockIo). **Not** wire RTT.

### Dual-plane TLS + syscall timing (Phase 8)

Content stays `TlsIo` (plaintext parse). TLS fds emit `SockIoTimes` (no prefix).
Userspace `DualPlane` keeps **time-pruned** Write/Read timestamps per fd (correlator
60 s timeout — not a count-capped ring). After a completed **HTTP/1.1** TLS exchange
it takes the latest preceding sock Write/Read in `[t−W, t]` (default 5 ms).
Miss → M3 TLS-only latency. HTTP/2 is not dual-plane joined: sock times are per-fd
and would be pasted onto the wrong stream.
Handshake: `SSL_do_handshake` (kind 6), start ts keyed by `SSL*` (cleared on `close`
and `SSL_free`). `tls_unmapped` increments **once per unmapped `SSL*`**, not per
`SSL_read`.

Wire latency is recorded on the same series key as content (`src,dst,method,route,…`).
Join **hit rate** is hits / (hits+fallback+miss) over the rolling 60 s window;
fallback is not a hit.

### Sampled traces (Phase 9)

Completed exchanges enqueue a **root** span (sampled 1/N, default 10) on the drain
thread. Handshake is **not** a child: `HandshakeIndex` stores the last success per
`(tgid, fd)` and copies `obsagent.tls.handshake_ns` onto the first sampled span
whose `t_start_ns` is after the handshake timestamp, then forgets it. Wall-clock
span times are `now − duration` / `now` at completion — never `bpf_ktime`.
HTTP/2 spans have no wire attribute (same reason as dual-plane).
The agent's own tgid/comm, and k8s-default `otelcol`, are dropped at ingest so
OTLP export is not correlated as an application exchange.

## Known failure modes (document, don’t pretend solved)

| Case | Effect |
|---|---|
| HTTP/1.1 pipelining / out-of-order | Occasional mis-pair |
| HTTP/2 multiplexing | fd-only pairing is forbidden once the fd is marked h2. Userspace leftover is `(tgid, fd, dir)` with a **skip-cursor** for frames larger than 8 KiB (DATA is not stored). Correlator key is `(tgid, fd, stream_id)` (Phase 7). Dual-plane **wire** is HTTP/1.1 only — per-fd sock times are not a per-stream clock. |
| TLS 1.3 NewSessionTicket inside `[t_end−W, t_end]` | Latest Read in the window wins; a ticket Read can steal the HTTP response stamp. Testdata uses `SSL_OP_NO_TICKET` so the demo does not hit this. |
| HTTP/2 fd reuse | Userspace drops `H2Conn` when BPF `SOCK_META`/`INFLIGHT` is gone, or on a new client preface. |
| First-frame detect | Preface, SETTINGS/PING/GOAWAY on stream 0, WINDOW_UPDATE, or HEADERS/RST/CONTINUATION on a nonzero stream. All-zero 9 bytes are not HTTP/2. |
| `SSL_set_fd` never called (custom BIO) | No fd → drop TLS event (M3) |
| rustls / Go crypto/tls / BoringSSL-only | No OpenSSL symbols → no TLS events (unsupported in M3) |
| Connection reuse across logical services | Need container/cgroup identity (Phase 4) |

Whiteboard diagram for interviews: this file + the SM above.
