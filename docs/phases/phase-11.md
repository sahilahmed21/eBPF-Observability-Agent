# Phase 11 — Overhead + sampling

Plan: [phase-11-implementation-plan.md](phase-11-implementation-plan.md).  
North star: [VISION-95.md](VISION-95.md) Q14, Q15.  
Evidence: [../testing/phase-11.tdd.md](../testing/phase-11.tdd.md).

**Buys:** the **&lt;2% CPU** wording — or an honest measured replacement. Do this **after** Phases 7–10 probes exist.

**Blocked on:** Milestone 10 for the **headline** (real node). If the VM is the measurement host, **do not** use WSL `ps` bursts as the claim. Code + unit tests do not wait on M10.

## Checklist

- [x] `benches/vision95-load.md` pinned: 500 RPS HTTP/1.1 + 200 RPS h2/gRPC, 60 s, 3 runs (vegeta v12.13.0 + ghz v0.121.0)
- [ ] Baseline (no agent) then agent; **`perf stat -p <pid>`** (not `ps`) — needs M10 node
- [ ] Record RSS, `events_dropped`, sample rate, kernel, date
- [x] In-kernel sticky 1/N of `SockIo`/`TlsIo`/`SockIoTimes` when drops &gt; 0 **or** `OBSAGENT_SAMPLE_N`
- [x] `connect`/`accept`/`handshake` never sampled away
- [x] Overload script (5k RPS): drops rise **or** N increases; histograms still move (script; run on node)
- [x] `DENIED_TGID` / `ALLOWED_TGID` filled from userspace (leaders; deny-list or allow-only)
- [ ] Optional trial: 128 B prefix (document; default stays 256 unless required)
- [ ] Vision-95 row in `docs/overhead.md` (numbers)
- [ ] Either mean **&lt; 2% of one host core** **or** fail the 2% wording (do not lower load)

## Milestone 11

Three-run `perf stat` row published. Sampling demonstrated under overload. 2% claim legal **only** if the number hit.

**Status:** code + unit tests landed; **headline not measured** (no M10 node run).
