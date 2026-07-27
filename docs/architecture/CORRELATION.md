# Correlation without request IDs

This is the core hard problem: kernel events have **no application request ID**. We reconstruct pairing from structural signals.

## Primary key

```
correlation_key = (tgid / pid, fd, 4-tuple)
```

Optionally refine with `tid` for TLS↔syscall merge.

## HTTP/1.1 state machine (per socket)

```
                 timeout / error
            ┌────────────────────────┐
            ▼                        │
     ┌──────────────┐         ┌──────┴───────┐
     │ AwaitingReq  │────────►│ RequestSeen  │  (server: read headers)
     └──────────────┘  read   └──────┬───────┘
                                     │ write response start
                                     ▼
                              ┌──────────────┐
                              │ ResponseSeen │──► emit span / histogram sample
                              └──────────────┘
                                     │
                                     └──► AwaitingReq (keep-alive)
```

**Client side** is the mirror: write (request) then read (response).

### Rules of thumb

1. On a **non-pipelined** keep-alive connection, alternating write↔read on the same fd within a bounded window ≈ one request/response pair.
2. Store pending request timestamp + method/path when the request prefix is parsed.
3. On matching response prefix (status line), compute latency = `t_response - t_request`.
4. **Timeout eviction:** if no response within T (e.g. 30s), drop pending state and count as incomplete.

## Known failure mode (document, don’t hide)

HTTP/1.1 **pipelining** and out-of-order responses can mis-pair. We accept occasional mis-pairs in Phase 2 in exchange for simplicity and low overhead. HTTP/2 (stretch) fixes multiplexing properly via **stream IDs**.

Interview honesty: explain the tradeoff; show the FSM; show how you’d evolve to stream-aware correlation.

## TLS plane ↔ syscall plane

When both Phase 2 syscall capture and Phase 3 TLS uprobes are active, the same logical write may fire twice:

```
SSL_write(plaintext)  ──sub-μs, same tid──►  write(ciphertext on fd)
```

**Merge policy**

- Content: prefer **TLS uprobe** bytes (plaintext).
- Timing for “on the wire”: prefer **syscall / sock** timestamps if available.
- Dedup key: `(pid, tid)` + timestamp delta under a tight threshold (e.g. tens of μs) + matching length heuristics.

## Service map edges (Phase 4)

```
node = resolved identity (pod/service/process)
edge = aggregated (caller → callee) from completed pairs
       metrics: rate, p99 latency, error rate
```

Caller/callee inference uses process identity on each side of the connection (client connect vs server accept) plus HTTP host/authority when present.

## Diagram for whiteboard

Memorize this one-liner:

> “We key on pid+fd+tuple, drive a tiny per-socket FSM, and treat TLS uprobes as authoritative plaintext with syscall timestamps for wire time — no distributed request ID required.”
