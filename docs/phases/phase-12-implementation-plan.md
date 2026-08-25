# Phase 12 — Implementation Plan (CPU profiles + 95% claim lock)

Companion to [phase-12.md](phase-12.md). Locked: [VISION-95.md](VISION-95.md) Q16, Q18.

**Phase 12 buys two things:** (1) stack join onto slow spans; (2) the only moment the original resume sentence becomes legal.

**Gate from Phase 11:** overhead number recorded with **profiles off**. This phase does not re-open the 2% run unless you explicitly re-measure with profiles on (optional extra row, not the headline).

---

## 0. Out of Phase 12

Parca/Pyroscope product compatibility, Java JIT, always-on fleet profiling, rewriting the 2% load with profiles enabled as the headline.

---

## 1. Locked decisions

| ID | Choice |
|---|---|
| **P12-Q1** | ~99 Hz `perf_event` (user stacks minimum; kernel stacks if cheap). Aya `PerfEvent` attach or userspace `perf_event_open` + ring — **lock:** prefer Aya perf_event programs that emit `{tgid, ts_ns, ip_stack[N]}` on a **separate** small RingBuf `STACKS` so HTTP `EVENTS` is not flooded. |
| **P12-Q2** | Max 16 frames per sample (or 32 if verifier allows). PerCpuArray scratch if stack &gt; 512 B. |
| **P12-Q3** | Symbolize in userspace with **blazesym**. Cache per `/proc/<tgid>/maps` fingerprint. |
| **P12-Q4** | Join: keep a short ring of samples (e.g. last 2 s, cap 8192). On span complete with `latency_ns ≥ 20 ms`, attach top-5 **symbolized** frames whose tgid matches and ts in window. |
| **P12-Q5** | Export: OTLP span **event** `profile` with `frame.0`…`frame.4` strings. Truncate paths. |
| **P12-Q6** | Default off. `OBSAGENT_PROFILE=1` for M12 gate and demos. |
| **P12-Q7** | Testdata: `/slow` sleeps in a **named** Rust function `slow_handler_sleep` (no inline). Hit rate = fraction of slow spans that include that symbol. Gate: **≥ 50%** over N=20. |
| **P12-Q8** | Verifier log: ≥ 2 real entries from the 6–12 work. Prefer natural. VISION Q18 force-stack **only** if still empty at 12.5. |
| **P12-Q9** | Claim lock file: `docs/handoff/SESSION-VISION-95.md` copying the 10-row table from VISION-95 §11 with evidence links. |
| **P12-Q10** | README resume signal **replaced** with the proven sentence (VISION-95 §0). Remove gRPC/`&lt;2%` if those gates failed. |

### Open

| # | Question | Blocks | How |
|---|---|---|---|
| **P12-Q11** | Aya PerfEvent vs userspace perf_event_open | 12.1 | Spike 0.5 day; pick one in §10 |
| **P12-Q12** | Frame count vs verifier | 12.1 | Start N=8; raise if load ok |

---

## 2. Architecture

```text
perf_event @ 99Hz → STACKS RingBuf {tgid, ts_ns, ips[N]}
userspace: blazesym → Sample{tgid, ts, frames[]}
           ring buffer 2s

span complete (≥20ms) → join samples → span event profile
```

Do not symbolize on the HTTP drain hot path if it stalls RingBuf: **lock:** dedicated tokio task for stacks; join uses a `Mutex<VecDeque<Sample>>` with cap.

---

## 3. Subphases

### 12.0 — Preconditions

M11 row exists. Claim lock template created empty.

---

### 12.1 — Perf stacks BPF + userspace read

**Work:** `STACKS` map; program; drain task. Count samples/s in headless.

**Verify:** with `OBSAGENT_PROFILE=1`, samples/s ≈ 99 × runnable threads (order of magnitude, not exact). Paste verifier issues.

---

### 12.2 — blazesym

**Work:** Resolve IPs for testdata binary. Unit test with a known function address if feasible; else smoke: `slow_handler_sleep` appears at least once.

**Verify:** one symbolized frame contains `slow_handler_sleep`.

---

### 12.3 — Join + OTLP event

**Work:** Window join; top 5; trace export.

**Verify:** `correctness12-profile`: N=20 `/slow`; hit rate ≥ 50%. Script `scripts/correctness-phase12.sh`. `wsl-run.sh correctness12` **on the VM** (symbols must match the demo binary).

---

### 12.4 — Verifier log

**Work:** Ensure ≥ 2 real pastes. If 0–1, either harvest from git history of 6–11 or VISION Q18 force.

**Verify:** `docs/verifier-rejection-log.md` has dates, program names, fix sentences.

---

### 12.5 — Claim lock

**Work:** Fill `SESSION-VISION-95.md` 10 rows. Rewrite README + `RESUME_AND_STORY.md` to proven claims. Tick ROADMAP Milestone 12.

**Verify:** every row has a path to evidence. Any red row → **do not** use the full original sentence.

---

## 4. Claim-lock table (copy into the handoff)

| # | Evidence | Unlocks | PASS? |
|---|---|---|---|
| 1 | `correctness6` + split-header | real syscall I/O | |
| 2 | `correctness7-grpc` + `correctness7-h2-tls` | gRPC / HTTP/2 | |
| 3 | `correctness8-dual` + handshake | dual-plane v3 | |
| 4 | traces 2xx + Grafana screenshot | traces | |
| 5 | k3s named `frontend → api` | service map | |
| 6 | Vision-95 `perf stat` row | 2% or honest % | |
| 7 | overload sampling | backpressure | |
| 8 | `correctness12` hit rate | profiles | |
| 9 | verifier log ≥ 2 | verifier hardness | |
| 10 | README/resume rewritten | stop lying | |

---

## 5. Success criteria (Milestone 12)

1. Profile join gate ≥ 50% hit rate.
2. Claim-lock file complete.
3. Public README matches evidence.
4. 5% residuals still listed (Go TLS, XDP, …).

---

## 6. Appendix — §10

| ID | Answer | Date | Notes |
|---|---|---|---|
| P12-Q11 | *(fill)* | | attach mechanism |
| P12-Q12 | *(fill)* | | N frames |
| Hit rate | *(fill)* | | N=20 |
