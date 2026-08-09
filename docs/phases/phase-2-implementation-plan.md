# Phase 2 — Implementation Plan

Companion to [phase-2.md](phase-2.md) (the checklist). This is the *how*: inventory of what
exists, architecture for HTTP awareness only, ordered subphases with verification, and every
decision that is still **open** (not guessed).

**Phase 2 buys one thing:** per-endpoint HTTP latency (and rate / 4xx/5xx %) reconstructed from
bounded socket I/O prefixes + a userspace correlation state machine, with zero changes to the
target HTTP server. No TLS plaintext (Phase 3), no OTLP/DaemonSet (Phase 4), no HTTP/2 (stretch).

Gate from Phase 1: Milestone 1 achieved ([phase-1.md](phase-1.md),
[phase-1.tdd.md](../testing/phase-1.tdd.md)). RingBuf + DROPS + Tokio drain + hdrhistogram +
Ratatui/headless are proven. Socket `read`/`write` capture, correlation SM, and `httparse` are
**not**.

---

## 0. Inventory — done vs missing

### Done (Phase 0 + Phase 1 + docs)

| Item | Evidence |
|---|---|
| Workspace `agent/` + `ebpf/` + `common/` | builds via `wsl-run.sh build` |
| One BPF ELF (`probes`), embed via `OUT_DIR/probes` | L2/L3 |
| RingBuf `EVENTS` 256 KiB + `DROPS` Array + `PENDING` HashMap | `common/` + `ebpf/` |
| Connect/accept TPs + `SockLatencyEvent` (48B) | Phase 1 ABI |
| Tokio AsyncFd RingBuf drain | `agent/src/main.rs` |
| Rolling 60s agg (hdrhistogram) keyed by remote + direction | `agent/src/agg.rs` |
| Ratatui + `OBSAGENT_HEADLESS` | `agent/src/main.rs` |
| Smoke + overhead method | `scripts/smoke-milestone1.sh`, `docs/overhead.md` Phase 1 row |
| Correlation design (doc only) | [correlation.md](../architecture/correlation.md) |
| Data-flow conceptual `SockIO` kind | [data-flow.md](../architecture/data-flow.md) |
| Security policy (redact before off-node export) | [security.md](../security.md) |

### Missing (nothing below is implemented in code)

| Item | Current state |
|---|---|
| `SockIO` (or equivalent) event ABI with bounded prefix | Not in `common/` |
| Tracepoints for `read`/`write`/`recvfrom`/`sendto` (exact set = Q) | Not in `ebpf/` |
| Kernel filter for socket fds only | Not present |
| Per-socket correlation SM in userspace | Doc only; no `agent` module |
| `httparse` dependency + truncated-header handling | Not in `Cargo.toml` |
| Path normalization | Not present |
| Per-endpoint HTTP histograms (method+path, status classes) | Phase 1 agg is connect/accept only |
| CLI endpoint table (latency, rate, 4xx/5xx %) | Not present |
| Injectable-latency HTTP server in `testdata/` | README stub only |
| Correctness gate vs known injected latency | Not present |
| Phase 2 smoke / overhead scripts | Not present |
| `fd` on Phase 1 `SockLatencyEvent` | **Absent** — Phase 1 events have `pid`/`tgid` only |

### Explicitly out of Phase 2 (owned later)

| Item | Owner |
|---|---|
| TLS uprobes / plaintext from `SSL_*` | Phase 3 |
| Dual-plane TLS↔syscall wire-timing merge | **Out of Phase 3 M3** (locked Q2); post-Phase-3 if ever |
| Service map, OTLP, DaemonSet, cgroup identity | Phase 4 |
| HTTP/2 stream demux / gRPC | Stretch |
| IPv6 | Deferred from Phase 1 Q4; still not required for Milestone 2 unless locked here |
| Perfect HTTP/1.1 pipelining pairing | Document as known failure ([correlation.md](../architecture/correlation.md)) |
| Sampling-rate map under overload | Optional later; keep drop counter only |
| CI workflow / `setcap` least-privilege proof | Still deferred ops |

---

## 1. Scope

### In scope

| # | Deliverable | Why it exists |
|---|---|---|
| D1 | Fixed-layout `SockIO` (name TBD) ABI in `common/` + RingBuf demux | Kernel→user bytes for prefixes |
| D2 | Tracepoint programs for chosen sock I/O syscalls; emit on enter or exit per Q | Bounded prefix capture |
| D3 | Filter non-socket fds (kernel preferred) | Bound volume; checklist says “socket fds only” |
| D4 | Userspace correlation SM on `(pid, fd)` (+ optional 4-tuple) | No request ID in kernel |
| D5 | `httparse` request/response; graceful truncate | Endpoint + status extraction |
| D6 | Path normalize (`/user/123` → `/user/:id` style) | Cardinality control |
| D7 | Rolling 60s `hdrhistogram` per HTTP endpoint key | Checklist |
| D8 | CLI: endpoint table — latency, rate, 4xx/5xx % (+ drops) | Milestone UI |
| D9 | Injectable-latency server in `testdata/` + correctness check | Checklist |
| D10 | Overhead row for Phase 2 under a **named** load | Checklist; &lt;2% target |
| D11 | Smoke gate `scripts/smoke-milestone2.sh` | Same reason as M0/M1 |

### Not in Phase 2

Anything in the “out of Phase 2” table. Also: redesign of RingBuf/DROPS transport; removing
Phase 1 connect/accept programs (unless Q locks say replace UI only).

---

## 2. Open questions — do not invent answers

These are **not** assumed. Each must be answered (experiment or explicit choice) before or during
the subphase that depends on it. Until answered, that subphase is blocked.

| # | Question | Why it matters | Blocks | How to resolve |
|---|---|---|---|---|
| **Q1** | Prefix length `N`: **256** or **512** bytes? | Event size, RingBuf pressure, header completeness | Subphase 2.1 | Explicit choice; measure drops under Q12 load |
| **Q2** | Syscall set: `read`/`write` only, or +`recvfrom`/`sendto`, or also `readv`/`writev`/`sendmsg`/`recvmsg`? | Attach surface, verifier, coverage of real servers | Subphase 2.2 | Explicit choice; prefer smallest set that hits Axum/nginx smoke |
| **Q3** | Emit on **enter** (buf ptr known) vs **exit** (actual len known) vs enter+stash/exit+emit? | Correct `len` vs verifier complexity; short reads | Subphase 2.2 | Experiment + record; classic pattern is enter stash ptr, exit read `min(ret,N)` |
| **Q4** | Non-socket fd filter: skip in **kernel** (how?) vs emit all and filter in userspace? | Overhead vs implementation time; socket check needs sock/file helpers | Subphase 2.2 | Prefer kernel filter; if timebox fails, document userspace filter + cost |
| **Q5** | RingBuf multiplexing: tagged variable-size events on existing `EVENTS`, fixed envelope, or second map? | Userspace decode; keep Phase 1 events working | Subphase 2.1 | Explicit choice; must not break M1 smoke without checklist amendment |
| **Q6** | Add `fd` to `SockLatencyEvent` for cross-ref, or leave Phase 1 ABI frozen and correlate **only** via SockIO `(pid,fd)`? | ABI churn vs YAGNI; Phase 1 plan said Phase 2 needs fd “or enough identity” | Subphase 2.1 | Explicit choice |
| **Q7** | CLI: **replace** connect/accept table with HTTP endpoint table, **dual** views, or HTTP-only when SockIO present? | Ratatui scope | Subphase 2.6 | Explicit choice |
| **Q8** | HTTP latency definition: request-half ts → response-half ts (correlation SM), which timestamps (enter vs exit of syscall)? | What p50/p95 mean in the UI | Subphase 2.4 | Explicit semantics + document limitation |
| **Q9** | Aggregation key fields: `METHOD + normalized_path` only? + remote host? + client vs server role? | Row cardinality and Milestone wording (“per-endpoint”) | Subphase 2.5 | Explicit choice |
| **Q10** | Path normalize rules: digit segments only? UUIDs? hex? multi-segment heuristics? | False merges vs cardinality explosion | Subphase 2.5 | Explicit minimal rule set |
| **Q11** | Correlation stale timeout duration? | Hung requests / disconnects | Subphase 2.4 | Explicit duration (e.g. 30s / 60s) |
| **Q12** | Phase 2 overhead load: what server + client script + concurrency + duration? | Comparable `docs/overhead.md` row | Subphase 2.8 | Write script first, then measure |
| **Q13** | Correctness server: Axum in-repo, nginx container, or both? Injectable delay mechanism? | `testdata/` contents | Subphase 2.7 | Explicit choice |
| **Q14** | Local CLI redaction of `Authorization` / `Cookie` in captured prefixes before parse/log? | [security.md](../security.md) focuses on off-node export; Phase 2 still holds secrets in RAM | Subphase 2.5 | Explicit choice (recommended: strip before any log; metrics-only UI) |
| **Q15** | Keep `smoke_probe` (Phase 1 Q9)? | Loader + smoke0 | Subphase 2.0 | Default candidate: keep unless M2 gate replaces M0 — still confirm |
| **Q16** | Do `sys_enter/exit_read` (and chosen siblings) attach/fire on this WSL2 kernel? | Same class as Phase 1 Q1 | Subphase 2.2 | Attach experiment; record pass/fail |

**Rule:** if a subphase needs a Q# answer and none is recorded in §10, stop and answer it. Do not
silently pick.

---

## 3. Decisions already locked (from prior phases) — not re-opened

| # | Decision | Source |
|---|---|---|
| L2 | One BPF ELF; programs share maps | Phase 0 |
| L3 | Embed object via `OUT_DIR/probes` | Phase 0 |
| L4 | Timestamps = `bpf_ktime_get_ns` | Phase 0 |
| T1 | Transport = RingBuf | OVERVIEW |
| T2 | Backpressure = drop + `DROPS` | ring-buffer-backpressure.md |
| T3 | Prefer tracepoints over raw kprobes | OVERVIEW |
| T5 | CLI aggregation window = rolling 60s | data-flow.md |
| T6 | Percentiles via `hdrhistogram` | Phase 1 Q6 / OVERVIEW |
| P1-Q4 | IPv4 only (unless Phase 2 explicitly expands) | Phase 1 |
| P1-Q8 | RingBuf 256 KiB, PENDING 8192 (PENDING may gain a sibling map; sizes stay until measured) | Phase 1 |
| P1-Q11 | Privilege: `wsl -d Ubuntu -u root` | Phase 1 |
| C1 | Correlation primary key = `(pid, fd)` + optional 4-tuple | correlation.md |
| C2 | HTTP/1.1 non-pipelined pairing; pipelining/H2 = known failure / stretch | correlation.md |
| C3 | Do not pretend HTTP/2 works on fd-only pairing | correlation.md |

---

## 4. Architecture (Phase 2 only)

### 4.1 Components

```
┌──────────────────────────────────────────────────────────────┐
│ Kernel (ebpf/probes)                                         │
│                                                              │
│  Phase 1: connect/accept → SockLatencyEvent (unchanged path) │
│                                                              │
│  Phase 2: sys_enter/exit_{read,write,...}                    │
│       → filter socket fd (Q4)                                │
│       → bounded prefix[N] (Q1) via bpf_probe_read_user(_buf) │
│       → SockIO event → EVENTS RingBuf (or DROPS++)           │
│                                                              │
│  Maps: PENDING (P1) | EVENTS | DROPS | (+ IO pending if Q3)  │
└───────────────────────────────┬──────────────────────────────┘
                                │
                                ▼
┌──────────────────────────────────────────────────────────────┐
│ Userspace (agent/)                                           │
│                                                              │
│  AsyncFd drain → demux by kind (Q5)                          │
│       ├─ SockLatencyEvent → existing Aggregator (Q7)         │
│       └─ SockIO → Correlation SM (pid,fd)                    │
│                      → httparse → normalize (Q10)            │
│                      → HttpAggregator (Q8/Q9)                │
│       scrape DROPS → UI                                      │
│       Ratatui / headless endpoint table (Q7)                 │
└──────────────────────────────────────────────────────────────┘
```

### 4.2 Correlation (from correlation.md — implement, don’t reinvent)

On one non-pipelined HTTP/1.1 connection:

- **Client:** `write` (request) then `read` (response) on same fd.
- **Server:** `read` (request) then `write` (response) on same fd.

State machine: Idle → first half → AwaitingResponse → emit span → Idle; timeout → Idle (Q11).

### 4.3 ABI ownership

| Layer | Owns |
|---|---|
| `common/` | `SockIO` layout, kind tags, prefix `N`, size consts; **no** speculative fields |
| `ebpf/` | I/O TPs + any IO-pending map; writes only via `common` types |
| `agent/` | Demux, correlation, httparse, normalize, HTTP agg, CLI, testdata driver hooks |

Grow fields only when a Phase 2 subphase needs them. Do not add Phase 3 TLS fields “for later”.

### 4.4 Verified technical notes (not product decisions)

- Aya exposes `bpf_probe_read_user` / `bpf_probe_read_user_buf` for userspace buffer prefixes
  (docs.rs `aya_ebpf::helpers`). Same pattern as Phase 1 sockaddr reads.
- Tracepoint argument offsets remain kernel-format-file driven (as Phase 1 offsets 24/16).
- Exa MCP was **not** available in this session; Aya helper APIs checked via WebSearch/docs.rs.

### 4.5 Trust / privacy (Phase 2)

Phase 2 captures **plaintext HTTP prefixes** (headers may include secrets). Apply Q14 before any
logging. Do not print raw prefixes in headless info logs. Metrics UI shows method/path/status
aggregates only.

---

## 5. Subphases

Every subphase: intent → work → verify. Done = verify passes.

### Subphase 2.0 — Preconditions · gate

**Intent:** Confirm Phase 1 still green; record Q15.

**Work:**

1. `wsl-run.sh preflight`, `test-common`, `test-agent`, root `smoke1`.
2. Answer Q15 in §10.

**Verify:** preflight OK; tests pass; smoke1 4 PASS; Q15 written.

**Stop:** Phase 1 regression → fix Phase 1 first.

---

### Subphase 2.1 — SockIO ABI in `common/` · foundation

**Depends on:** Q1 (N), Q5 (mux), Q6 (fd on P1 event or not).

**Work:**

1. Lock Q1/Q5/Q6 in §10.
2. Add `#[repr(C)]` SockIO (+ kind enum extension). Size/align asserts + unit tests.
3. Document demux rule for userspace.
4. `cargo`/wsl test-common still green; Phase 1 layout tests still pass if ABI frozen.

**Verify:** size consts; build; tests; Q1/Q5/Q6 recorded.

**ponytail:** one SockIO shape for r/w directions; no separate request/response kernel types.

---

### Subphase 2.2 — Attach surface: sock I/O TPs · critical path

**Depends on:** Q2, Q3, Q4, Q16; ABI from 2.1.

**Work:**

1. Q16 attach experiment (log-only or minimal emit).
2. Implement chosen syscall set with enter/exit pattern from Q3.
3. Socket fd filter per Q4; timebox kernel path.
4. Agent attach new TPs; keep Phase 1 attaches.
5. Headless: generate `curl` against local listener; confirm SockIO events (nonzero count).

**Verify:** attach/unload clean; events observed; Q16/Q2–Q4 recorded; verifier issues →
`docs/verifier-rejection-log.md`.

---

### Subphase 2.3 — Userspace demux + wire decode · foundation

**Intent:** Typed decode of mixed RingBuf payloads without correlation yet.

**Work:** Demux helper + unit tests (fixture bytes); wire into drain loop (count SockIO).

**Verify:** unit tests; headless counter increments under traffic.

---

### Subphase 2.4 — Correlation state machine · core

**Depends on:** Q8, Q11.

**Work:**

1. Implement SM per [correlation.md](../architecture/correlation.md) in `agent/` (dedicated module).
2. Unit tests: client write→read; server read→write; timeout eviction; ignore unpaired.
3. Emit internal “HTTP exchange” record (method raw bytes / status / t_start / t_end) — parse may
   be stubbed until 2.5 if tests use synthetic frames.

**Verify:** unit tests GREEN; Q8/Q11 recorded.

---

### Subphase 2.5 — httparse + path normalize + HTTP agg · product metrics

**Depends on:** Q9, Q10, Q14.

**Work:**

1. Add `httparse` workspace dep.
2. Parse request/response from prefixes; truncated → skip or partial (document).
3. Normalize paths per Q10; unit tests.
4. Rolling 60s hdrhistogram per Q9 key; 4xx/5xx percentages.
5. Redaction per Q14.

**Verify:** unit tests for parse/normalize/agg; no Authorization in logs if Q14 requires strip.

---

### Subphase 2.6 — CLI endpoint table · milestone UI

**Depends on:** Q7.

**Work:** Ratatui (+ headless println) for endpoint rows: latency p50/p95/p99, rate, 4xx/5xx %,
drops. Preserve or dual Phase 1 view per Q7.

**Verify:** manual/TTY or headless sample output under load; checklist UI row evidence.

---

### Subphase 2.7 — Correctness vs injectable latency · gate

**Depends on:** Q13.

**Work:**

1. Add `testdata/` HTTP server with injectable delay.
2. Script: start server (delay D) → run agent headless → curl → assert observed p50 within
   tolerance band of D (tolerance **must be chosen explicitly** when locking Q13 — clock/SM
   skew).
3. Record in `docs/testing/phase-2.tdd.md`.

**Verify:** gate script exit 0; evidence report.

---

### Subphase 2.8 — Smoke + overhead · close Milestone 2

**Depends on:** Q12.

**Work:**

1. `scripts/smoke-milestone2.sh` (load → HTTP rows/events → unload).
2. Named bench + `scripts/overhead-phase2.sh`; fill `docs/overhead.md` Phase 2 row.
3. Check off [phase-2.md](phase-2.md) with evidence pointers.

**Verify:** smoke PASS; overhead row filled; checklist complete.

---

## 6. Definition of Done

Phase 2 complete when every [phase-2.md](phase-2.md) checklist row is checked **and** evidence
exists in `docs/testing/phase-2.tdd.md` + overhead row.

**Milestone 2:** Point at local HTTP server; per-endpoint latency with zero changes to that server.

---

## 7. Risks

| Risk | Why real | Mitigation |
|---|---|---|
| read/write volume overwhelms RingBuf | Higher rate than connect | Q1/Q4/Q12; drops visible; shrink N before sampling map |
| Non-socket fds flood if Q4 is userspace-only | Many processes | Prefer kernel filter; document if not |
| ABI mux breaks Phase 1 smoke | Shared EVENTS | Q5 + keep smoke1 green |
| Mis-paired pipelining | Documented failure mode | Do not claim perfect pairing |
| Secrets in prefixes | HTTP headers | Q14; no raw prefix logs |
| httparse needs more than N bytes | Large headers | Q1 512 vs 256; truncated = skip/partial |
| Missing `fd` on P1 events | Cross-ref connect↔HTTP | Q6; SockIO alone may suffice for Milestone 2 |

---

## 8. Stop rules

| Subphase | If stuck |
|---|---|
| 2.0 | Fix Phase 1 regression |
| 2.2 | No attach after timebox → record Q16 fail; do not invent kprobe fallback without new Q |
| 2.2 | Verifier on socket filter → fall back per Q4 written choice |
| 2.4–2.5 | Scope creep into TLS/H2 → refuse |
| 2.6 | UI polish beyond table metrics → stop |
| 2.8 | Overhead &gt;2% → record cause; do not silently add sampling map |

---

## 9. Handoff

### From Phase 1 (consumed)

- RingBuf + DROPS + drain; connect/accept latency CLI; overhead method; WSL root path
- **Gap:** `SockLatencyEvent` has no `fd` — resolve via Q6

### To Phase 3 (produced)

- SockIO prefixes + correlation SM + httparse path; Phase 3 adds OpenSSL `TlsIo` into the same SM (TLS-only latency; no dual-plane merge in M3)

### TDD mapping (plan task → test target)

| Plan | Test target |
|---|---|
| 2.1 ABI | `common` layout/size tests |
| 2.3 demux | `agent` decode unit tests |
| 2.4 SM | `agent` correlation unit tests |
| 2.5 parse/norm/agg | `agent` unit tests |
| 2.7 correctness | `scripts` + injectable server |
| 2.8 smoke/overhead | `smoke-milestone2.sh`, overhead script |

RED before production code per `/tdd-workflow`. Evidence → `docs/testing/phase-2.tdd.md`.

---

## 10. Decisions appendix (fill while executing)

| ID | Choice | Date | Evidence / reason |
|---|---|---|---|
| Q1 | **A – prefix 256 B** | 2026-08-07 | Smallest useful; bump only if smoke shows truncation |
| Q2 | **A revised → A+sendto/recvfrom** | 2026-08-07 | Locked A (read+write); strace of `http-probe` shows `sendto`/`recvfrom` only (glibc TcpStream). Extended attach surface with evidence; read/write kept. |
| Q3 | **A – enter stash / exit emit `min(ret,N)`** | 2026-08-07 | Exit knows transferred byte count |
| Q4 | **A – SOCK_FDS on enter + HTTP content on exit** | 2026-08-07 | Enter: stash only if `SOCK_FDS` marked (connect/accept). Exit: emit only HTTP-looking prefixes. Earlier SOCK_FDS-only emit path yielded sockio=0 during bring-up; content filter kept for SSH/noise. |
| Q5 | **A – kind-tagged variable events on existing `EVENTS`** | 2026-08-07 | Preserve transport; no second map / oversized envelope |
| Q6 | **A – leave `SockLatencyEvent` unchanged** | 2026-08-07 | HTTP via SockIO only; no P1 ABI coupling |
| Q7 | **B – dual/toggle UI** | 2026-08-07 | Keep P1 TCP latency view; add HTTP visibility |
| Q8 | **A – request-half exit ts → response-half exit ts** | 2026-08-07 | Application-visible HTTP latency from completed I/O |
| Q9 | **A – `METHOD + normalized_path`** | 2026-08-07 | Milestone “per-endpoint”; remote/role → Phase 4 |
| Q10 | **A – replace pure-digit path segments with `:id`** | 2026-08-07 | Deterministic cardinality control; UUID later if evidence |
| Q11 | **B – 60 s SM timeout** | 2026-08-07 | Align with Phase 1 rolling window |
| Q12 | **Axum + concurrent probe sample (not full 32×30s)** | 2026-08-07 | Measured via `overhead-phase2-quick.sh` (burst `http-probe`); full Q12 32-worker/30s script exists but row is the quick sample — do not treat as host-normalized &lt;2% proof. |
| Q13 | **A – Axum in `testdata/`** | 2026-08-07 | Injectable delay + status; p50 within max(±10 ms, ±10%) of injected delay |
| Q14 | **A – redact Auth/Cookie; metrics-only UI** | 2026-08-07 | No secret-logging habit; parse/agg only |
| Q15 | **A – `smoke_probe` opt-in** | 2026-08-07 | Attach only when `OBSAGENT_SMOKE_PROBE` is set (smoke0/1/2 scripts). Default off to avoid `try_to_wake_up` cost. |
| Q16 | **PASS – read/write/sendto/recvfrom attach** | 2026-08-07 | `smoke-milestone2.sh` PASS on WSL2 6.6; sendto/recvfrom required (strace) |

When a row is filled, Subphase work that depended on it may proceed.
