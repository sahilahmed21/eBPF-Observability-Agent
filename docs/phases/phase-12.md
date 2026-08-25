# Phase 12 — CPU profiles + claim lock

Plan: [phase-12-implementation-plan.md](phase-12-implementation-plan.md).  
North star: [VISION-95.md](VISION-95.md) Q16, Q18.  
Evidence: [../testing/phase-12.tdd.md](../testing/phase-12.tdd.md).  
Claim lock: [../handoff/SESSION-VISION-95.md](../handoff/SESSION-VISION-95.md).

**Buys:** original stretch — slow endpoint → hot function, without redeploy — **and** the 95% resume sentence (with **measured** overhead).

**Blocked on:** none (Milestone 11 + 12 gates closed 2026-08-25).

## Checklist

- [x] `perf_event` stack sampling ~99 Hz; off by default; `OBSAGENT_PROFILE=1`
- [x] Userspace symbolize (`blazesym`); no BPF symbolizer
- [x] Join samples to completed spans: same tgid, `ts ∈ [t_start, t_end]`
- [x] Only spans ≥ 20 ms; top 5 frames as span event/attribute
- [x] `/slow` named-function path: busy-wait `slow_handler_sleep`; `prof_hit=11/20` (≥50%)
- [x] `docs/verifier-rejection-log.md` ≥ **2 real** pastes (Q18 force stack-limit ×2)
- [x] Claim-lock table (10 rows) filled in `SESSION-VISION-95.md` — **10/10**
- [x] README + resume bullets rewritten to **proven** sentence (~87% of one core)
- [x] Interview pack updated after the gate

## Milestone 12

Phase 12 join gate PASS **and** claim lock complete. You may claim 95% of the original brief **with the measured overhead wording**. You may **not** claim Pixie, all TLS libraries, zero drops, multi-cluster, or **&lt;2% CPU**.

**Status:** complete (2026-08-25).
