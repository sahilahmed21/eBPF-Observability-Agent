# Phase 1 — Implementation Plan

Companion to [phase-1.md](phase-1.md) (the checklist). This is the *how*: inventory of what
exists, architecture for the MVP only, ordered subphases with verification, and every decision
that is still **open** (not guessed).

**Phase 1 is the first product.** It buys one thing: live connect/accept latency per remote
endpoint from kernel probes, drained over RingBuf, shown in a CLI, with drops and overhead
visible. No HTTP parse, no TLS, no OTLP, no service map.

Gate from Phase 0: Milestone 0 is achieved ([phase-0.md](phase-0.md),
[phase-0.tdd.md](../testing/phase-0.tdd.md)). Toolchain path is proven. Event ABI, maps, and
attach points for connect/accept are **not**.

---

## 0. Inventory — done vs missing

### Done (Phase 0 + docs)

| Item | Evidence |
|---|---|
| Linux 5.15+ target with BTF | WSL2 `6.6.114.1`, `/sys/kernel/btf/vmlinux` present |
| Toolchain: stable + nightly(`rust-src`) + `bpf-linker` + `bpftool` | [phase-0.tdd.md](../testing/phase-0.tdd.md) baseline table |
| Workspace `agent/` + `ebpf/` + `common/` | builds via `cargo build --release` |
| One BPF ELF (`probes`), embedded at compile time | L2/L3 in Phase 0 plan; `agent/build.rs` |
| Smoke kprobe load → `aya-log` → clean unload | `scripts/smoke-milestone0.sh` PASS |
| Environment gate | `scripts/preflight.sh` |
| Locked transport/backpressure/timestamp design (docs) | RingBuf + drop counter; `CLOCK_MONOTONIC` ns (L4) |
| Phase 1 checklist + milestone statement | [phase-1.md](phase-1.md) |
| Architecture intent for Phase 1 attach surface | [OVERVIEW.md](../architecture/OVERVIEW.md) |

### Missing (nothing below is implemented in code)

| Item | Current state |
|---|---|
| Event ABI in `common/` | `common/src/lib.rs` is empty placeholder |
| Tracepoints `sys_enter/exit_connect`, `accept4` | Still `smoke_probe` on `try_to_wake_up` |
| Enter/exit HashMap for latency | No maps in `ebpf/` |
| RingBuf + `drop_count` map | Not present |
| Socket / sockaddr metadata on events | Not present |
| `aya-tool` / CO-RE bindings | Deferred in Phase 0; not installed (per evidence report) |
| Phase 0 Step 8 attach de-risk (`tcp_v4_connect` + `sys_enter_connect`) | **Explicitly deferred** — not run |
| Tokio RingBuf consumer | Agent only flushes `aya-log` |
| Rolling 60s aggregates (p50/p95/p99, count, errors) | No aggregator; no `hdrhistogram` / Ratatui deps in `agent/Cargo.toml` |
| Ratatui live table + sparkline | Not present |
| Overhead measurement under load | `docs/overhead.md` empty; `benches/` has README only |
| Drop counter wired to userspace / CLI | Not present |
| Phase 1 smoke / correctness script | Only `smoke-milestone0.sh` exists |
| CI | Not configured (Phase 0 Step 9.3 deferred) |
| Least-privilege `setcap` proof | Phase 0 Step 9.2 deferred |

### Explicitly out of Phase 1 (owned later)

| Item | Owner |
|---|---|
| Socket `read`/`write` byte capture, HTTP parse, path normalize | Phase 2 |
| Correlation state machine for req/res | Phase 2 ([correlation.md](../architecture/correlation.md)) |
| TLS uprobes / redaction of payloads | Phase 3 |
| Service map, OTLP, DaemonSet, Grafana | Phase 4 |
| HTTP/2, gRPC, CPU profile merge | Stretch |
| Sampling-rate map under overload | Optional later; backpressure doc allows it after drop counter exists |

---

## 1. Scope

### In scope

| # | Deliverable | Why it exists |
|---|---|---|
| D1 | `#[repr(C)]` event ABI in `common/` shared by kernel + userspace | Without a locked layout, RingBuf bytes are untyped |
| D2 | Tracepoint (preferred) programs for connect enter/exit and accept path | Milestone: latency from kernel probes only |
| D3 | BPF `HashMap` keyed for enter→exit pairing; latency computed on exit | Enter alone cannot produce duration |
| D4 | One `RingBuf` + `drop_count` map; submit on exit (or drop + count) | Locked transport + visible loss ([ring-buffer-backpressure.md](../architecture/ring-buffer-backpressure.md)) |
| D5 | Address / endpoint fields on the exit event | CLI groups by remote endpoint |
| D6 | Tokio consumer draining RingBuf into typed events | Bridge kernel → userspace |
| D7 | Rolling 60s aggregates: count, errors, p50/p95/p99 per remote endpoint | Checklist + [data-flow.md](../architecture/data-flow.md) |
| D8 | Ratatui table + sparkline showing those aggregates + drop count | Milestone: live CLI |
| D9 | Overhead row in `docs/overhead.md` under a **named** load | Checklist; target &lt;2% is project-wide, not yet measured for Phase 1 |
| D10 | Automated gate script for Phase 1 (load → see events → unload / no leak) | Same reason as `smoke-milestone0.sh` |

### Not in Phase 1

Anything in the “out of Phase 1” table above. Also: process/cgroup filtering, pinning maps,
multi-binary packaging, musl static binary, Dockerfile.

---

## 2. Open questions — do not invent answers

These are **not** assumed. Each must be answered (experiment or explicit choice) before or during
the subphase that depends on it. Until answered, that subphase is blocked or must take the
documented fallback path only after the fallback is chosen in writing here.

| # | Question | Why it matters | Blocks | How to resolve (method, not answer) |
|---|---|---|---|---|
| Q1 | Do `syscalls:sys_enter_connect` / `sys_exit_connect` (and `accept`/`accept4`) attach and fire on **this** kernel (WSL2 6.6)? | Phase 0 Step 8 never ran; checklist prefers these TPs | Subphase 1.2 | Attach + `curl` / `nc` trigger; log once; record pass/fail in verifier log or a short note in this doc |
| Q2 | If Q1 fails: fall back to which attach? `tcp_v4_connect` kprobe only? enter+exit kprobes? | Changes program set and symbol stability story | Subphase 1.2 | Decide only after Q1 evidence; document choice + reversal cost |
| Q3 | Socket metadata source for MVP: CO-RE sock fields vs `bpf_probe_read_user` on `sockaddr`? | Checklist prefers sock-layer CO-RE; `aya-tool` not installed yet | Subphase 1.3 | Try preferred path first; if verifier/timebox fails, take sockaddr path and record why |
| Q4 | IPv4 only for Milestone 1, or IPv4+IPv6? | Event size, display, test matrix | Subphase 1.1 (ABI) | Explicit choice before freezing `common/` layout |
| Q5 | Aggregation key: remote `ip:port` only, or include local port / pid / direction (connect vs accept)? | Defines Ratatui rows and “per remote endpoint” meaning | Subphase 1.5 | Explicit choice; default candidate in docs is “remote endpoint” — still need exact fields |
| Q6 | Percentile store: `hdrhistogram` crate now, or simple sorted window / reservoir for Phase 1? | OVERVIEW locks `hdrhistogram` for the project; Phase 1 checklist only requires p50/p95/p99 | Subphase 1.5 | Explicit choice; if not `hdrhistogram`, note Phase 2 must align with OVERVIEW |
| Q7 | Non-blocking `connect` (`EINPROGRESS`): is “latency” enter→exit of the syscall, or until connection established? | Syscall exit is not “connected” for non-blocking sockets | Subphase 1.2 / 1.3 | Explicit semantics + document limitation; TCP established tracking may be Phase 2+ |
| Q8 | RingBuf size (bytes) and HashMap max entries? | Memory vs drop rate under load | Subphase 1.3 | Pick numbers, measure drops under D9 load, tune once |
| Q9 | Keep `smoke_probe` in the ELF for regression, or replace entirely? | Loader + smoke script currently expect `smoke_probe` | Subphase 1.2 | Explicit choice; if removed, rewrite Milestone 0 script or add Phase 1 script as the gate |
| Q10 | Overhead load definition: what binary, concurrency, duration, host? | `benches/` empty; overhead table has no method yet beyond “same script every milestone” | Subphase 1.7 | Write the script first, then measure; do not claim &lt;2% without that script |
| Q11 | Dev privilege path for Phase 1: continue `sudo -E` / `wsl -u root`, or finish Phase 0 `setcap` proof first? | Not required for Milestone 1 functionally; affects how scripts are written | Subphase 1.0 / scripts | Operator choice; record which path smoke uses |

**Rule:** if a subphase needs a Q# answer and none is recorded in §3, stop and answer it. Do not
silently pick.

---

## 3. Decisions already locked (from prior docs) — not re-opened

| # | Decision | Source |
|---|---|---|
| L2 | One BPF ELF object; programs share maps | Phase 0 plan |
| L3 | Embed object at compile time via `OUT_DIR/probes` | Phase 0 plan |
| L4 | Event timestamps = `bpf_ktime_get_ns` (monotonic); wall clock only at export if ever | Phase 0 plan |
| L5 | `aya-log` OK for Phase 1 probes; not a hot-path design for later phases | Phase 0 plan |
| L9 | Install/use `aya-tool` when CO-RE structs are needed (this phase if Q3 = CO-RE) | Phase 0 plan |
| T1 | Kernel→user transport = RingBuf | OVERVIEW |
| T2 | Backpressure = drop + `drop_count` map; userspace surfaces it | ring-buffer-backpressure.md |
| T3 | Prefer tracepoints over raw kprobes on syscall internals | OVERVIEW |
| T4 | Prefer socket-layer + CO-RE for addresses when available | OVERVIEW / phase-1 checklist |
| T5 | CLI aggregation window = rolling 60s | data-flow.md / phase-1 checklist |
| T6 | Crate layout `obsagent` / `obsagent-ebpf` / `obsagent-common` | Phase 0 |

Any new Phase 1 lock (answers to Q1–Q11) gets appended to a table at the bottom of this file when
chosen — do not leave them only in chat.

---

## 4. Architecture (Phase 1 only)

### 4.1 Components

```
┌──────────────────────────────────────────────────────────────┐
│ Kernel (ebpf/probes)                                         │
│                                                              │
│  sys_enter_connect ──► HashMap insert {key → enter_ts, ...}  │
│  sys_exit_connect  ──► lookup → latency_ns → RingBuf event   │
│                        (on reserve fail → drop_count++)      │
│  accept / accept4  ──► (same pattern once Q1/Q2 answered)     │
│                                                              │
│  Maps: pending HashMap | EVENTS RingBuf | drop_count         │
└───────────────────────────────┬──────────────────────────────┘
                                │ typed bytes (common::Event)
                                ▼
┌──────────────────────────────────────────────────────────────┐
│ Userspace (agent/)                                           │
│                                                              │
│  AsyncFd RingBuf drain → decode Event                        │
│       │                                                      │
│       ├─► rolling 60s aggregator (per endpoint key = Q5)     │
│       ├─► scrape drop_count map periodically                 │
│       └─► Ratatui: table + sparkline + drop line             │
└──────────────────────────────────────────────────────────────┘
```

### 4.2 Data path (how it works)

1. **Enter probe** fires: read pid/tgid (and whatever args Q3/Q4 require), store `enter_ts =
   bpf_ktime_get_ns()` in HashMap under a key that **exit can find** (key shape is part of
   implementation after Q1 — typically tid or pid+tid; exact key is not locked in docs today →
   treat as design detail to record when coding).
2. **Exit probe** fires: lookup enter record; if missing, count as unpaired (userspace metric or
   in-kernel counter — **not specified in existing docs** → open detail, keep minimal).
3. Compute `latency_ns = now - enter_ts`. Read return code. Attach address fields (Q3/Q4).
4. `bpf_ringbuf_reserve` → fill `common` event → `submit`. On failure: increment `drop_count`.
5. Userspace polls RingBuf; decodes with the same `repr(C)` layout; updates rolling window;
   redraws TUI; periodically reads `drop_count`.

### 4.3 ABI ownership

| Layer | Owns |
|---|---|
| `common/` | Fixed-layout event struct(s), constants, maybe event kind enum — compiles for host **and** BPF |
| `ebpf/` | Programs + map definitions; writes events only via `common` types |
| `agent/` | Load/attach, RingBuf + map handles, aggregate, TUI, overhead harness hooks |

Grow fields only when a Phase 1 subphase needs them ([data-flow.md](../architecture/data-flow.md)).
Do not add Phase 2 `SockIO` fields “for later”.

### 4.4 Event kinds (from data-flow — conceptual until `common/` lands)

| Kind | Phase 1 role |
|---|---|
| Connect enter/exit (or single exit event carrying latency) | Core MVP |
| Accept exit (same family) | Checklist includes accept4 |
| Drop stats | Userspace poll of map, not necessarily a RingBuf event |

Exact field list is **not** frozen in code yet. Freeze in Subphase 1.1 after Q4/Q5 (and Q7
semantics note).

### 4.5 Trust / privilege (Phase 1)

Phase 1 still needs BPF attach privileges. Payload redaction is mostly Phase 3; Phase 1 events are
addrs + latency + pid — still host-sensitive. Do not log full sockaddrs at `info` in a way that
becomes a habit for later payload phases ([security.md](../security.md)).

---

## 5. Subphases

Every subphase has: intent → work → verify. A subphase is done when its verify passes, not when
code was typed.

### Subphase 1.0 — Preconditions & open-question prep · gate

**Intent:** Confirm Phase 0 still green; install only tools required by chosen Q3 path; record Q11.

**Work:**

1. Re-run `scripts/preflight.sh` and `scripts/smoke-milestone0.sh` on the Linux target.
2. Answer **Q11** (how privileged runs happen) in §3 appendix.
3. If pursuing CO-RE (Q3): install `aya-tool` per README
   (`cargo install --git https://github.com/aya-rs/aya -- aya-tool`). If not, skip install.

**Verify:**

| Check | Pass |
|---|---|
| `preflight.sh` | exit 0 |
| `smoke-milestone0.sh` | exit 0 |
| Q11 written into this doc | present |

**Stop:** If Milestone 0 regresses, fix Phase 0 — do not start Phase 1 maps/ABI.

---

### Subphase 1.1 — Event ABI in `common/` · foundation

**Intent:** Freeze the bytes on the wire between kernel and userspace for Milestone 1.

**Depends on:** Q4 (address family), Q5 (enough fields for the aggregation key), Q7 note
(document what `latency_ns` means).

**Work:**

1. Answer Q4, Q5, Q7 in the decisions appendix.
2. Add `#[repr(C)]` event struct(s) + kind tag in `common/`.
3. Userspace: `Pod` impl behind `user` feature (as Phase 0 comment already describes).
4. No programs yet — ABI-only change must still `cargo build --release`.

**Verify:**

| Check | Pass |
|---|---|
| `cargo build --release` | exit 0 |
| Struct size/alignment documented (comment or `const` assert in `common`) | present |
| Q4/Q5/Q7 recorded | present |

**ponytail:** one event kind that carries latency on exit is enough if enter is HashMap-only;
separate enter RingBuf events are YAGNI unless a consumer needs them.

---

### Subphase 1.2 — Attach surface: connect enter/exit · critical path

**Intent:** Prove the preferred tracepoints (or chosen fallback) load, fire, and unload cleanly.

**Depends on:** Q1, then Q2 if needed; Q9 (smoke_probe fate).

**Work:**

1. Run Q1 experiment (minimal log-only probes). Record result.
2. If fail → resolve Q2, implement fallback, record.
3. Resolve Q9: keep or replace smoke probe; update scripts accordingly.
4. Replace or supplement `smoke_probe` with connect enter/exit programs sharing one ELF (L2).
5. Loader in `agent/`: attach the new programs (tracepoint API in Aya — use whatever the 0.14
   API exposes; confirm against Aya book at implementation time, do not assume API names here).

**Verify:**

| Check | Pass |
|---|---|
| Programs visible in `bpftool prog list` while running | expected names present |
| Trigger (`curl`/`nc`/local connect) produces probe evidence (aya-log or later events) | ≥1 per connect |
| Clean unload / no pins | same bar as Milestone 0 |
| Q1/Q2/Q9 recorded | present |
| Verifier rejection (if any) | one entry in [verifier-rejection-log.md](../verifier-rejection-log.md) |

**Stop:** If neither TP nor fallback attaches within a half-day timebox, environment/API issue —
document and stop; do not invent a third speculative attach stack.

---

### Subphase 1.3 — Maps: HashMap + RingBuf + drop_count + addresses · core kernel

**Intent:** Emit typed latency events with endpoint metadata; drops countable.

**Depends on:** 1.1, 1.2; Q3, Q8.

**Work:**

1. Answer Q3 and Q8; record.
2. Add maps to `ebpf/`; enter inserts; exit looks up, builds `common` event, RingBuf submit.
3. On reserve failure: increment `drop_count`.
4. Implement address capture per Q3 choice (CO-RE and/or sockaddr read).
5. Keep hot path free of unnecessary `aya-log` once events flow (L5: info OK while debugging).

**Verify:**

| Check | Pass |
|---|---|
| `bpftool map list` shows pending + RingBuf + drop counter while running | yes |
| Userspace can open maps by name (even if consumer is still a debug print) | yes |
| Artificial overload or tiny RingBuf (if used to test) moves `drop_count` | optional but preferred |
| Verifier clean or logged | yes |

---

### Subphase 1.4 — Tokio RingBuf consumer · bridge

**Intent:** Drain events into typed Rust values continuously until Ctrl-C.

**Depends on:** 1.3.

**Work:**

1. RingBuf async drain pattern (same `AsyncFd` style as existing aya-log flush).
2. Decode with `common` layout; log or counters first — TUI can wait.
3. Handle partial/shutdown cleanly (drop `Ebpf` → unload).

**Verify:**

| Check | Pass |
|---|---|
| Generating connects prints/counts matching events in userspace | yes |
| Ctrl-C unloads; no leaked progs/pins | yes |

---

### Subphase 1.5 — Rolling 60s aggregates · metrics

**Intent:** p50/p95/p99, count, errors per aggregation key (Q5); expose drop_count.

**Depends on:** 1.4; Q5, Q6.

**Work:**

1. Answer Q6; record.
2. Implement rolling window (60s) keyed by Q5.
3. Error definition: based on syscall return / errno on the event — exact mapping recorded when
   implemented (docs do not spell errno table today).
4. Scrape `drop_count` on an interval.

**Verify:**

| Check | Pass |
|---|---|
| Known fast vs slow connects (manual delay if needed) move percentiles in the expected direction | qualitative OK for 1.5; tighten in 1.6/1.7 |
| Drop counter visible when forced | yes |

---

### Subphase 1.6 — Ratatui live table + sparkline · milestone UI

**Intent:** Operator sees live endpoints without reading logs.

**Depends on:** 1.5.

**Work:**

1. Add Ratatui (and whatever backend the chosen Ratatui version needs) to `agent` deps when
   implementing — versions not pinned in docs today.
2. Table: endpoint key, count, errors, p50/p95/p99.
3. Sparkline: simple rate or latency series from the rolling window.
4. Show drop count on screen.

**Verify:**

| Check | Pass |
|---|---|
| Live update while `curl` loop runs | visible |
| Readable on a normal terminal width | yes |

**Milestone 1 (product):** this subphase + working 1.2–1.5.

---

### Subphase 1.7 — Overhead baseline + Phase 1 gate script · hardening

**Intent:** Checklist rows for overhead + drop counter “wired”; automate regression.

**Depends on:** 1.6; Q10.

**Work:**

1. Answer Q10: write the load script under `benches/` (even if minimal).
2. Measure agent CPU/RSS vs baseline; fill Phase 1 row in `docs/overhead.md`.
3. Add `scripts/smoke-milestone1.sh` (or equivalent): build artifact assumed present; load agent;
   generate connects; assert events or TUI-adjacent signal; unload; no leak. Exact assertions
   depend on what 1.4–1.6 expose (log line vs counter file — choose the smallest observable).
4. Confirm drop counter appears in CLI (already in 1.6) under the load or a forced small buffer.

**Verify:**

| Check | Pass |
|---|---|
| `docs/overhead.md` Phase 1 row filled | non-empty |
| Gate script exit 0 on clean target | yes |
| phase-1.md checklist can be checked off with evidence pointers | yes |

---

### Subphase 1.8 — Accept path · checklist completion

**Intent:** Checklist includes `accept4`; Milestone text says connect/accept.

**Depends on:** 1.2–1.6 patterns stable; Q1 evidence for accept TPs.

**Work:**

1. Confirm accept/accept4 tracepoint availability (same method as Q1).
2. Reuse HashMap/RingBuf/event shape; distinguish direction/kind so Q5/TUI can show it if needed.
3. Extend gate script with a listener+connect pair.

**Verify:**

| Check | Pass |
|---|---|
| Accept-side latency rows appear under a local listen/connect test | yes |
| Unload clean | yes |

**Note:** If accept attach fails on this kernel after a documented attempt, record the failure and
narrow Milestone 1 to connect-only **only** with an explicit checklist amendment — do not silently
ship connect-only while the checklist still claims accept4.

---

## 6. Definition of Done

Phase 1 is complete when every [phase-1.md](phase-1.md) checklist row is checked **and** the
verify column below has evidence (script output, overhead row, or short note in
`docs/testing/` — create a `phase-1.tdd.md` when executing, same style as Phase 0).

| Checklist item | Verification |
|---|---|
| Tracepoints connect (+ accept4) | Attach evidence + Q1/Q2 notes; programs in `bpftool` |
| HashMap enter / latency + RingBuf on exit | Events in userspace with nonzero `latency_ns` for paired calls |
| Socket metadata | Endpoint fields populated per Q3/Q4 |
| Tokio RingBuf consumer | Continuous drain until Ctrl-C |
| Rolling 60s aggregates | p50/p95/p99, count, errors for Q5 key |
| Ratatui table + sparkline | Live UI |
| Overhead → `docs/overhead.md` | Phase 1 row + named `benches/` script (Q10) |
| Drop counter wired | Visible in UI (and/or metrics print) when drops occur |

**Milestone 1 statement (from phase-1.md):** Live CLI of connect/accept latency per remote
endpoint from kernel probes only; measured overhead documented.

---

## 7. Risks

| Risk | Why real here | Mitigation |
|---|---|---|
| Attach TPs missing / different on WSL2 | Step 8 never run | Subphase 1.2 + Q1/Q2 before ABI consumers harden |
| Non-blocking connect latency misleading | Q7 unanswered | Document semantics; do not claim “TCP RTT” |
| CO-RE / `aya-tool` rabbit hole | Preferred path, not proven | Timebox; fall back to sockaddr (Q3) |
| Phase 1 grows into Phase 2 | HTTP/correlation tempting once bytes are near | Out-of-scope list; refuse SockIO fields |
| Milestone verified by eyeball only | Same failure mode as pre-D5 Phase 0 | Subphase 1.7 gate script |
| Tiny RingBuf or huge HashMap wrong-sized | Q8 open | Measure drops under Q10 load once |

---

## 8. Stop rules / timeboxes (guidance)

| Subphase | If stuck |
|---|---|
| 1.0 | Phase 0 regression → fix Phase 0 |
| 1.2 | No attach after timebox → stop and write evidence; change environment or Q2 |
| 1.3 | Verifier fights on CO-RE → switch to sockaddr path (Q3), log rejection |
| 1.6 | TUI polish beyond table+sparkline → stop; polish is not the milestone |
| 1.7 | Overhead &gt;2% → record cause; sampling is Phase 2+ optional, not a silent Phase 1 feature creep |

Exact hour budgets are not locked in existing docs for Phase 1 — do not invent them. Use the
stop rules above; add budgets here only if you want them before starting.

---

## 9. Handoff

### From Phase 0 (consumed)

- Working build/load/unload path; `preflight.sh`; `smoke-milestone0.sh`
- L2 (shared maps in one ELF), L4 (monotonic ns)
- Deferred: Step 8 attach de-risk, `aya-tool`, setcap, CI

### To Phase 2 (produced)

Phase 2 needs: stable RingBuf + drop counter + userspace drain; pid/fd (or enough identity) on
events to cross-ref socket I/O; overhead method already in use. Phase 2 must **not** redesign
transport. Correlation SM and `httparse` start only after Milestone 1.

---

## 10. Decisions appendix (fill while executing)

| ID | Choice | Date | Evidence / reason |
|---|---|---|---|
| Q1 | **PASS – sys_enter/exit_connect + accept4 attach and fire** | 2026-08-06 | `scripts/smoke-milestone1.sh` PASS on WSL2 6.6; format files present |
| Q2 | _n/a_ | 2026-08-06 | Q1 passed |
| Q3 | **sockaddr via `bpf_probe_read_user`** | 2026-08-06 | MVP path; CO-RE/`aya-tool` deferred (ponytail: add when sock-layer fields needed) |
| Q4 | **A – IPv4 only** | 2026-08-06 | Small ABI + simple tests; IPv6 in Phase 2+ |
| Q5 | **B – remote endpoint + direction (connect/accept)** | 2026-08-06 | Milestone needs connect vs accept separated; PID skipped (cardinality) |
| Q6 | **A – hdrhistogram** | 2026-08-06 | Matches OVERVIEW; avoid rewrite later |
| Q7 | **A – syscall enter→exit only** | 2026-08-06 | What probes observe; EINPROGRESS ≠ established; TCP state = Phase 2+ |
| Q8 | **A – RingBuf 256 KiB, HashMap 8192** | 2026-08-06 | Conservative MVP; tune in overhead subphase |
| Q9 | **B – keep smoke_probe alongside** | 2026-08-06 | Preserve Phase 0 gate until Milestone 1 script replaces it |
| Q10 | **`benches/connect-load.sh` (N curls to 1.1.1.1)** | 2026-08-06 | Script added; CPU%/RSS numbers still pending timed run |
| Q11 | **A – `wsl -d Ubuntu -u root`** | 2026-08-06 | Phase 0 path; setcap after milestone |

When a row is filled, Subphase work that depended on it may proceed.
