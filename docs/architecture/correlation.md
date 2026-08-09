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

## TLS plane (Phase 3 / Milestone 3)

HTTPS encrypts in userspace, so Phase 2 sock I/O prefixes are ciphertext and useless.
Phase 3 hooks OpenSSL and **reuses this same `(tgid, fd)` state machine**:

1. `SSL_set_fd` (and rfd/wfd) → map `SSL*` → `fd`
2. `SSL_read` / `SSL_write` uprobes → bounded plaintext prefix (`TlsIo`)
3. Feed `TlsIo` into the existing correlator / httparse / aggregator

**Latency (M3):** TLS half exit→exit (same SM semantics as Phase 2 SockIo). **Not** wire RTT.

### Dual-plane TLS + syscall timing — not in Milestone 3

A future enhancement could treat TLS as content and syscall sock probes as wire timing, with
tid/ts dedup. That requires nested-syscall attribution, custom BIO / async edge cases, and is a
**different product**. Do not sneak it into Phase 3. See
[phase-3-implementation-plan.md](../phases/phase-3-implementation-plan.md) Q2.

## Known failure modes (document, don’t pretend solved)

| Case | Effect |
|---|---|
| HTTP/1.1 pipelining / out-of-order | Occasional mis-pair |
| HTTP/2 multiplexing | fd-only pairing breaks → need stream IDs (stretch) |
| `SSL_set_fd` never called (custom BIO) | No fd → drop TLS event (M3) |
| rustls / Go crypto/tls / BoringSSL-only | No OpenSSL symbols → no TLS events (unsupported in M3) |
| Connection reuse across logical services | Need container/cgroup identity (Phase 4) |

Whiteboard diagram for interviews: this file + the SM above.
