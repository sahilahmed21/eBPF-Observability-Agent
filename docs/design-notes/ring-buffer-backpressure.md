# Ring buffer backpressure

## Problem

Under load, eBPF can produce events faster than userspace can drain the RingBuf.
If you “fix” by endlessly enlarging buffers, you trade CPU for **kernel memory pressure** and still eventually lose.

## Options

| Option | Pros | Cons |
|--------|------|------|
| **(a) Sample / drop + drop counter** | Predictable memory; visible loss; production-proven | Incomplete traces under overload |
| **(b) Increase RingBuf size** | Buys headroom | Delays the cliff; higher kernel memory |
| **(c) Userspace bounded channel + drop policy** | Controls userspace queueing | Does not help if reserve fails in kernel first |
| Block / backpressure into probes | — | **Not viable** — probes must be fast; blocking is unacceptable |

## Decision (default)

**Pick (a)** as the primary policy, with (b) as a tuned constant and (c) as a secondary userspace guard.

Concrete behavior:

1. On `bpf_ringbuf_reserve` failure → `atomic` increment `events_dropped`.
2. Expose `events_dropped` (and optionally `events_submitted`) via a BPF map read on a timer or CLI status line.
3. **Sticky 1/N of connections** (`KEEP_IO` on `SOCK_META`) when drops rise or `OBSAGENT_SAMPLE_N` is set. Not per-event `% N` (that breaks HTTP/2 HPACK).
4. Size the RingBuf generously for the demo load, but treat size as **not** the correctness strategy.

## Interview one-liner

> “We drop on reserve failure, export a drop counter so loss isn’t silent, and only then tune buffer size — we’d rather shed load than blow kernel memory.”
