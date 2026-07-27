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

## TLS plane + syscall plane

`SSL_write` internally leads to `write`/`sendto` on the underlying fd. Two probes for one logical op:

| Signal | Authoritative for |
|---|---|
| TLS uprobe (`SSL_*`) | **Content** (plaintext) |
| Syscall / sock probe | **Wire timing** |

Dedup: same `(pid, tid)` + sub-microsecond / same-thread timestamp window. Prefer TLS content; keep syscall timestamps for latency-to-wire.

## Known failure modes (document, don’t pretend solved)

| Case | Effect |
|---|---|
| HTTP/1.1 pipelining / out-of-order | Occasional mis-pair |
| HTTP/2 multiplexing | fd-only pairing breaks → need stream IDs (stretch) |
| Connection reuse across logical services | Need container/cgroup identity (Phase 4) |

Whiteboard diagram for interviews: this file + the SM above.
