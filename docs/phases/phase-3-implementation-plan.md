# Phase 3 — Implementation Plan

Companion to [phase-3.md](phase-3.md) (the checklist). This is the *how*: inventory, architecture
for OpenSSL HTTPS only, ordered subphases with verification. **Q1–Q12 are locked** (2026-08-10);
Q13–Q15 remain for execution.

**Phase 3 buys one thing:** the same per-endpoint HTTP metrics as Phase 2, but for **HTTPS**
processes that encrypt in userspace (OpenSSL), via uprobes on `SSL_read` / `SSL_write`, plus
documented redaction policy. No OTLP/DaemonSet (Phase 4), no HTTP/2, no Go `crypto/tls`, no
GnuTLS/BoringSSL-as-first-class, no dual-plane TLS↔syscall timing engine (document as later).

Gate from Phase 2: Milestone 2 achieved (commit `c2ad86a` / merge `c938f28`). Sock I/O TPs,
`(tgid,fd)` correlator, httparse, HTTP agg, dual UI, Axum HTTP testdata are proven. TLS uprobes
and HTTPS correctness are **not**.

---

## 0. Inventory — done vs missing

### Done (Phases 0–2)

| Item | Evidence |
|---|---|
| Workspace `agent/` + `ebpf/` + `common/`; one ELF `probes` | builds via `wsl-run.sh` |
| RingBuf `EVENTS` + `DROPS` + kind demux | `decode.rs`; kinds Connect/Accept/SockIo |
| Connect/accept + sock I/O TPs; `SOCK_FDS` enter filter + HTTP magic exit | `ebpf/src/main.rs` |
| `SockLatencyEvent` 48B; `SockIoEvent` 288B / 256B prefix | `common/` |
| Correlator `(tgid,fd)` + 60s timeout; httparse + `:id` normalize | `correlate.rs`, `http.rs` |
| `redact_headers` (Auth/Cookie/Set-Cookie) — **export-only**, not hot path | `http.rs` |
| HTTP + TCP UI toggle; headless both | `main.rs` |
| HTTP smoke/correctness/overhead scripts | `smoke2`, `correctness2`, `overhead-phase2*` |
| TLS design docs (not code) | [tls-interception.md](../architecture/tls-interception.md), [correlation.md](../architecture/correlation.md), [PROBE_MAP.md](../architecture/PROBE_MAP.md), [security.md](../security.md) |

### Missing (nothing below is implemented)

| Item | Current state |
|---|---|
| Uprobe/uretprobe on `SSL_write` / `SSL_read` | Not in `ebpf/` |
| Libssl path discovery (1.1 vs 3.x) + attach | Not in `agent/` |
| `SSL*` → fd (or alternate correlator key) | Not present |
| TLS event ABI / `EventKind` extension | Not in `common/` |
| Feed TLS prefixes into correlator → HTTP agg | Not wired |
| HTTPS testdata (**must** link OpenSSL, not rustls) | Only plaintext Axum server |
| Smoke/correctness/overhead for TLS | Not present |
| Redaction evidence + security review checklist tick | Policy exists; M3 gate missing |

### Explicitly out of Phase 3

| Item | Owner |
|---|---|
| OTLP, DaemonSet, cgroup→pod identity, service map | Phase 4 |
| HTTP/2 / gRPC stream demux | Stretch |
| Go `crypto/tls`, GnuTLS, BoringSSL-first-class, static-linked OpenSSL | Document unsupported |
| Full dual-plane “TLS content + syscall wire timing” merge | **Out of M3** (locked Q2); post-Phase-3 if ever |
| In-kernel redaction | Never for M3 — too hot / fragile |
| Split god `main.rs` | Deferred (Phase 2 review skip) |
| IPv6 | Still deferred |

**DB / schema:** none. This agent is in-process maps + RingBuf + rolling histograms. No schema
changes. Phase 4 may add OTLP attribute conventions — not Phase 3.

---

## 1. Scope

### In scope (Milestone 3)

| # | Deliverable | Why |
|---|---|---|
| D1 | Resolve + attach `SSL_write` / `SSL_read` on OpenSSL 1.1 **and** 3.x paths present on host | Checklist |
| D2 | Bounded plaintext prefix via `bpf_probe_read_user` (same N=256 unless Q reopens) | Content for HTTPS |
| D3 | `SSL_set_fd` → `SSL*`→fd; drop TLS emit if fd unknown; key `(tgid,fd)` | Pair req↔resp (Q1) |
| D4 | Demux TLS events → existing `Correlator` → `parse_exchange` → `HttpAggregator` | Reuse, don’t fork HTTP path |
| D5 | Redaction: keep export helper; prove it in tests; document when it runs | Checklist + security.md |
| D6 | HTTPS test service (self-signed OK) that **actually calls OpenSSL** | Correctness |
| D7 | Smoke + correctness + overhead row for Phase 3 | Same gate style as P1/P2 |
| D8 | Security section review note (caps: +`CAP_SYS_PTRACE` for uprobes) | Checklist |

### Not in Phase 3

Anything in the out-of-phase table. Also: redesign Phase 2 sock I/O ABI “for TLS later”; rewriting
correlation.md mid-flight without locking Qs.

---

## 2. Questions — Q1–Q12 locked; Q13–Q15 open

### Locked (2026-08-10) — do not silently reverse

| # | Locked choice |
|---|---|
| **Q1** | `SSL_set_fd` (+ rfd/wfd) → `SSL*`→fd map; key correlator `(tgid,fd)`; **drop** TLS emit if fd unknown |
| **Q2** | **TLS-only** content **and** latency for M3 — **no** dual-plane TLS↔syscall wire-timing merge |
| **Q3** | `EventKind::TlsIo`; layout **twin** of `SockIoEvent` (288 B / 256 B prefix) |
| **Q4** | Prefix **256 B** |
| **Q5** | Try-attach host `libssl.so.3` and `libssl.so.1.1` (candidate paths; no `/proc` maps scan required for M3) |
| **Q6** | Soft-fail if libssl unavailable (warn + continue) |
| **Q7** | Enter-stash / exit-emit for both `SSL_read` and `SSL_write` |
| **Q8** | Skip Phase 2 sock I/O on TLS-marked fds |
| **Q9** | HTTPS testdata **must** use OpenSSL/`libssl` (`ldd` evidence); not rustls |
| **Q10** | Same correctness band as Phase 2 (p50 within max(±10ms, ±10%)) |
| **Q11** | Metrics-only UI; **never** log raw prefixes |
| **Q12** | Missing libssl must **not** break cleartext HTTP (Phase 2 path stays on) |

**Scope discipline (locked with Q2):** do not add Go, rustls, BoringSSL-first-class, HTTP/2, OTLP,
Kubernetes, DB, nested-syscall attribution, custom-BIO fd recovery, or async-runtime special cases
in Phase 3. Those are different products.

### Still open (answer during execution)

| # | Question | Blocks | How to resolve |
|---|---|---|---|
| **Q13** | Overhead load definition for Phase 3 row | 3.6 | Write script first; honest `ps` caveat |
| **Q14** | Implied by Q12 — confirm P2 path stays fully active | 3.0 | Default **yes** (record in §10) |
| **Q15** | Do `SSL_*` symbols resolve + uprobe attach on this WSL2 Ubuntu OpenSSL? | 3.2 | Attach experiment; record PASS/FAIL |

**Rule:** do not invent answers for Q13–Q15. Do not reopen Q1–Q12 without an explicit amend.

---

## 3. Decisions already locked — not re-opened

| # | Decision | Source |
|---|---|---|
| L2–L4 | One BPF ELF; embed via OUT_DIR; `bpf_ktime_get_ns` | Phase 0 |
| T1–T2 | RingBuf + drop counter | OVERVIEW / backpressure |
| P2-Q1 | Prefix 256 B | Phase 2 |
| P2-Q4 | SOCK_FDS enter + HTTP magic exit (plaintext plane) | Phase 2 |
| P2-Q5 | Kind-tagged shared `EVENTS` | Phase 2 |
| P2-Q6 | `SockLatencyEvent` frozen | Phase 2 |
| P2-Q8/Q11 | HTTP latency = half exit→exit; 60s SM timeout | Phase 2 |
| P2-Q9/Q10 | `METHOD + normalized_path`; digit → `:id` | Phase 2 |
| P2-Q14/Q15 | Redact off hot path; smoke_probe opt-in | Phase 2 |
| C2–C3 | HTTP/1.1 non-pipelined; H2 = known failure | correlation.md |
| S1 | OpenSSL first; other stacks later/limitation | OVERVIEW / tls-interception.md |

---

## 4. Architecture (Phase 3)

### 4.1 Component flow (proposed)

```
┌─────────────────────────────────────────────────────────────────┐
│ Target process (OpenSSL)                                        │
│   SSL_set_fd(ssl, fd)  ──uprobe──► SSL_FD[ssl*] = fd   (Q1)     │
│   SSL_write(ssl, buf, n)                                        │
│     enter: stash {buf, ssl*, dir=Write} by tid                  │
│     exit:  lookup fd; read min(ret,N); HTTP magic?; emit TlsIo  │
│   SSL_read(ssl, buf, n)                                         │
│     enter: stash {buf, ssl*, dir=Read}                          │
│     exit:  same (plaintext now in buf)                          │
└───────────────────────────────┬─────────────────────────────────┘
                                │ RingBuf (kind=TlsIo)
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│ Userspace                                                       │
│   decode → TlsIo → adapt to SockIo-shaped observe()             │
│         → Correlator(tgid,fd) → Exchange                        │
│         → parse_exchange → HttpAggregator (same table as P2)    │
│   Phase 2 SockIo path unchanged for cleartext HTTP              │
│   UI: same HTTP view (TLS + cleartext rows coexist)             │
└─────────────────────────────────────────────────────────────────┘
```

**Maps (additive):**

| Map | Role |
|---|---|
| `SSL_FD` | `ssl*` → `fd` (from `SSL_set_fd` / rfd / wfd) |
| `PENDING_TLS` | tid → `{buf_ptr, ssl*, dir}` (enter/exit pair) |
| existing | `EVENTS`, `DROPS`, `SOCK_FDS`, `PENDING_IO`, `PENDING` |

### 4.2 Dual-plane correlation — explicitly out of M3 (Q2 locked)

Older architecture notes described TLS = content, syscall = wire timing, with tid/ts dedup.
**Milestone 3 does not implement that.** Solving TLS plaintext + wire timing + nested syscalls +
custom BIOs + async runtimes is a different product (“general TLS/network correlation engine”).

M3 pipeline (locked):

```text
SSL_set_fd → SSL*→fd → SSL_read/write uprobes → TlsIo → existing correlator
  → existing HTTP parser → existing aggregator → existing CLI
```

Latency = TLS half exit→exit (same SM semantics as Phase 2 SockIo). Dual-plane wire timing is
post-Phase-3 only if product asks.

### 4.3 Reuse vs redesign

| Reuse as-is | Thin adapter | Redesign (avoid) |
|---|---|---|
| `Correlator`, `Exchange`, timeout | `TlsIo` → observe API | New HTTP SM for TLS |
| `parse_exchange`, normalize, `HttpAggregator` | decode kind branch | Parallel TLS agg tables |
| RingBuf + DROPS + headless/TUI HTTP view | attach helper for libssl paths | New transport / second RingBuf |
| `redact_headers` | call sites when logging/export appears | In-kernel redaction |
| Phase 2 cleartext path | optional SOCK_FDS skip for TLS fds (Q8) | Remove sock I/O programs |

### 4.4 ABI (Q3/Q4 locked)

```text
EventKind::TlsIo = 4
TlsIoEvent { kind, dir, prefix_len, fd, pid, tgid, ret, ts_ns, prefix[256] }
  // layout twin of SockIoEvent (288 B)
```

No `ssl_ptr` on the RingBuf event: if `SSL_FD` lookup fails, drop (Q1).

### 4.5 Attach / discovery (Q5/Q6/Q12 locked)

1. Try-attach candidate paths for `libssl.so.3` and `libssl.so.1.1` (e.g. under
   `/lib/x86_64-linux-gnu/` on Ubuntu).
2. For each path that exists: `SSL_set_fd` / `SSL_set_rfd` / `SSL_set_wfd`, plus
   `SSL_read` / `SSL_write` enter+exit (Q7).
3. Missing lib / symbols → warn, continue; cleartext Phase 2 must still work (Q6/Q12).
4. CAP note: uprobes need `CAP_SYS_PTRACE` (or root) — confirm in security.md during 3.4.

### 4.6 Security

| Concern | M3 stance |
|---|---|
| Agent sees all hooked plaintext on node | Same as product claim; treat as privileged |
| Secrets in captured prefixes | Metrics-only UI; `redact_headers` before any off-node/log (Q11) |
| Attack surface | Prefer least attach scope (Q5); no payload in aya-log |
| Unsupported TLS stacks | Clear warn; no silent “HTTPS covered” claim |

### 4.7 Edge cases / reliability / scale

| Case | Effect / mitigation |
|---|---|
| rustls / Go crypto/tls / BoringSSL custom | No events — document; Q9 testdata must use OpenSSL |
| `SSL_set_fd` never called (custom BIO) | No fd → drop TLS event if Q1 requires fd |
| Short `SSL_read` / renegotiation | Enter/exit stash; HTTP magic may fail → drop (same as P2) |
| HTTP/2 over TLS | Mis-pair / miss — known failure |
| Many processes × libssl attach | Uprobe cost; measure Q13; allowlist later |
| RingBuf pressure (TLS + cleartext) | Shared DROPS; same 256 KiB until measured |
| Libssl upgrades / path move | Re-resolve on start; no live reload in M3 |
| WSL path quirks | Same as P1/P2: Ubuntu distro, root for attach |

### 4.8 Verified external notes (not product decisions)

- Aya: `UProbe::attach(Some("SSL_write"), 0, lib_path, pid_opt)` pattern is established
  (public Aya HTTPS sniffer writeups).
- Production agents (eCapture, Pixie, qtap) use **`SSL_set_fd` maps** and/or fragile SSL struct
  offsets; struct-offset chasing is a version farm — avoid for M3.
- Exa MCP was **not** configured this session; used WebSearch for attach + SSL→fd patterns.

---

## 5. Subphases

Every subphase: intent → work → verify. Done = verify passes.

### 3.0 — Preconditions · gate

**Intent:** Phase 2 still green; lock Q14.

**Work:** `test-common`, `test-agent`, root `smoke2` (+ optional `correctness2`). Answer Q14.

**Verify:** tests pass; smoke2 PASS; Q14 in §10.

---

### 3.1 — TLS ABI in `common/` · foundation

**Depends on:** Q3, Q4.

**Work:** Extend `EventKind`; add `TlsIoEvent` (or chosen shape); size/align tests; decode stub
rule. Do **not** change `SockIoEvent` / `SockLatencyEvent` layouts.

**Verify:** test-common; P2 size tests still pass; Q3/Q4 recorded.

---

### 3.2 — eBPF uprobes + attach · kernel path

**Depends on:** Q1, Q5–Q8, Q12, Q15.

**Work:**

1. Maps `SSL_FD`, `PENDING_TLS`.
2. Uprobes: set_fd family (if Q1); SSL_write/SSL_read enter/exit per Q7.
3. Emit TlsIo only when fd known (if required) + HTTP magic (reuse logic / shared helper).
4. Userspace: discover lib paths; attach; soft-fail (Q12).
5. Record Q15 attach experiment on WSL Ubuntu.

**Verify:** programs load; at least one libssl path attached; Q15 PASS/FAIL written.

**Stop:** Q15 FAIL → document blocker (wrong lib / symbol) before more userspace work.

---

### 3.3 — Userspace demux + correlate · wire-up

**Depends on:** Q1, Q2.

**Work:** decode `TlsIo` → feed correlator (adapter); ensure cleartext SockIo unchanged.
Optional Q8: mark TLS fds to skip sock I/O enter.

**Verify:** unit tests for adapter + correlator with synthetic TlsIo; test-agent green.

---

### 3.4 — Redaction + security · policy evidence

**Depends on:** Q11.

**Work:** Expand/confirm `redact_headers` tests; document “when called” in security.md /
phase-3 checklist note. No hot-path redact in parse.

**Verify:** unit tests; security.md reviewed checkbox ready.

---

### 3.5 — HTTPS testdata + correctness · gate

**Depends on:** Q9, Q10.

**Work:** OpenSSL-backed HTTPS server + client probe; `smoke-milestone3.sh`;
`correctness-phase3.sh` (injected delay).

**Verify:** smoke3 PASS (tls events + HTTP rows); correctness3 p50 band; `ldd` evidence libssl.

---

### 3.6 — Overhead · measurement

**Depends on:** Q13.

**Work:** Named load script; sample; `docs/overhead.md` Phase 3 row (honest caveats).

**Verify:** row filled; drops noted; no false &lt;2% claim from a burst sample alone.

---

### 3.7 — Checklist + TDD evidence · close

**Work:** Tick [phase-3.md](phase-3.md); write `docs/testing/phase-3.tdd.md`; fill §10.

**Verify:** all checklist items have evidence pointers.

---

## 6. Locked summary (quick reference)

See §2 locked table. Progression stays:

```text
Phase 1  kernel connection observability
Phase 2  HTTP over plaintext sockets
Phase 3  HTTP over HTTPS/OpenSSL (TLS-only latency)
Phase 4  production / k8s / service map / OTLP
```

---

## 7. Success criteria (Milestone 3)

1. OpenSSL HTTPS traffic produces HTTP endpoint rows (method/path, latency, 4xx/5xx %) in the
   same UI/agg as Phase 2.
2. Cleartext HTTP (Phase 2) still passes smoke2/correctness2.
3. Redaction helper tested; security.md reviewed for uprobe caps + plaintext trust boundary.
4. smoke3 + correctness3 green on WSL Ubuntu; overhead row recorded honestly.
5. Unsupported stacks documented (no pretend coverage).

---

## 8. Appendix — §10 answers

| ID | Answer | Date | Notes |
|---|---|---|---|
| Q1 | `SSL_set_fd` → fd; drop if unknown | 2026-08-10 | Reuse `(tgid,fd)` correlator |
| Q2 | TLS-only latency | 2026-08-10 | No dual-plane / nested-syscall engine |
| Q3 | `EventKind::TlsIo`, SockIo twin | 2026-08-10 | 288 B layout |
| Q4 | 256 B prefix | 2026-08-10 | Same as P2 Q1 |
| Q5 | Try-attach `libssl.so.3` / `libssl.so.1.1` | 2026-08-10 | Candidate paths |
| Q6 | Soft-fail if libssl unavailable | 2026-08-10 | Warn + continue |
| Q7 | Enter-stash / exit-emit | 2026-08-10 | Both SSL_read and SSL_write |
| Q8 | Skip sock I/O on TLS-marked fds | 2026-08-10 | Via SSL_FD |
| Q9 | HTTPS testdata must use OpenSSL/libssl | 2026-08-10 | `ldd` evidence |
| Q10 | Same correctness band as Phase 2 | 2026-08-10 | max(±10ms, ±10%) |
| Q11 | Metrics-only; never log raw prefixes | 2026-08-10 | Redact helper for export later |
| Q12 | Missing libssl must not break cleartext | 2026-08-10 | P2 path stays on |
| Q13 | `overhead-phase3-quick.sh` (8 rounds × https-probe 3 paths) | 2026-08-10 | avg ~3.5% `ps` CPU; ~22.6 MiB RSS; drops=0. Not full long harness / not &lt;2% claim |
| Q14 | **yes** (implied by Q12) | 2026-08-10 | Keep P2 plaintext path active |
| Q15 | **PASS** | 2026-08-10 | `SSL_set_fd` + `SSL_*` + `SSL_*_ex` attach on WSL Ubuntu OpenSSL 3.5; CPython uses `_ex` |
