# Handoff — continue after Phase 2 session

**Purpose:** Drop this file into a new chat so another agent (or you) can resume without rediscovering context.

**Session dates:** 2026-08-07 (Phase 2 plan → execute → brutal review → fix-all)  
**Repo:** `c:\projects\eBPF-Observability-Agent` (Windows checkout; build/run on **WSL2 Ubuntu**)  
**Git tip (committed):** `main` @ `b7aeb06` — *feat: Phase 1 connect/accept latency MVP with RingBuf CLI*  
**Working tree:** **Phase 2 + review fixes are UNCOMMITTED** (many modified + untracked files). Do **not** assume clean `main` has HTTP awareness.  
**Remote:** local `main` still **ahead of `origin/main` by 5** (Phase 0+1 only). Phase 2 never committed unless you ask.  
**Prior handoff:** `docs/handoff/SESSION-2026-08-06-phase-1.md` (still valid for env + Phase 1 ABI; superseded for “what’s next”).

---

## 1. What this project is

Zero-instrumentation observability agent in **Rust + Aya (eBPF)**: reconstruct per-service HTTP/gRPC latency and a service map from kernel syscalls + TLS uprobes, target **&lt;2% CPU**, later k8s DaemonSet + OTLP.

Phases: **0** toolchain ✅ → **1** connect/accept latency ✅ → **2** HTTP ✅ (uncommitted) → **3** TLS → **4** prod → **S** stretch.

---

## 2. What’s done (do not redo)

### Phase 0 + 1 — committed @ `b7aeb06`

See prior handoff. Still true: `SockLatencyEvent` 48B, connect/accept TPs, RingBuf + DROPS, rolling TCP agg, headless/TUI.

### Phase 2 — Milestone 2 ✅ (this session; **uncommitted**)

| Deliverable | Where |
|---|---|
| ABI: `SockIoEvent` (288 B), `PendingIo`, `EventKind::SockIo`, `IoDir` | `common/src/lib.rs` |
| TPs: read/write + **sendto/recvfrom**; close; SOCK_FDS mark on connect/accept | `ebpf/src/main.rs` |
| Maps: `PENDING_IO`, `SOCK_FDS`, `IO_SCRATCH` (+ existing PENDING/EVENTS/DROPS) | `ebpf/src/main.rs` |
| Q4 dual filter: **SOCK_FDS on enter** + **HTTP magic on exit** | `try_enter_io` / `looks_like_http_fixed` |
| RingBuf demux by kind | `agent/src/decode.rs` |
| Correlation SM `(tgid, fd)`; latency = resp exit − req exit (Q8); 60s timeout (Q11) | `agent/src/correlate.rs` |
| httparse + path `:id` normalize; redact **off** hot path | `agent/src/http.rs` |
| HTTP rolling agg + 4xx/5xx % | `agent/src/http_agg.rs` |
| Dual UI: HTTP default, `t` toggles TCP (Q7); headless prints both | `agent/src/main.rs` |
| Axum injectable server + client probe | `testdata/latency-server`, `testdata/http-probe` |
| Gates | `scripts/smoke-milestone2.sh`, `correctness-phase2.sh`, overhead scripts |
| Plan + Q appendix Q1–Q16 | `docs/phases/phase-2-implementation-plan.md` |
| Checklist | `docs/phases/phase-2.md` (all checked) |
| TDD evidence | `docs/testing/phase-2.tdd.md` |
| Overhead row | `docs/overhead.md` Phase 2 |

### Review fixes applied (same uncommitted tree)

Brutal `/rust-patterns` review → user: **fix every finding**. Done with intentional skips:

| Finding | Design decision / fix |
|---|---|
| SOCK_FDS ignored (`let _marked`) | **Enforce on enter** — unmarked fd → no `PENDING_IO` |
| Enter stamped all fds / high CPU | Same — enter filter is the cheap gate |
| Correlator arms identical | Start only on **request-looking** prefix; complete only on **`HTTP/`** response |
| Server `sendmsg`/`writev` | **Documented limit**, not attached (client pairs via sendto/recvfrom) |
| Always 256 B `probe_read` | Kept (verifier fixed dest); comment in eBPF |
| Wire `u8` vs enums | ABI stays `u8`; userspace `EventKind`/`IoDir::from_u8` |
| Decode no kind/`prefix_len` cap | Kind cross-check + cap to `SOCK_IO_PREFIX_LEN` |
| Redact on hot path | Removed from `parse_exchange`; `redact_headers` kept for future export (`#[allow(dead_code)]`) |
| Hist rebuild every `rows()` | Same as Phase 1 agg; `ponytail:` comment — upgrade if profiled |
| Mutex poison swallowed | `lock_mut` → `unwrap_or_else(|p| p.into_inner())` |
| God `main` | **Not split** — leave until Phase 3+ |
| Duplicated accept exit | Merged via `peer_addr` helper |
| No reassembly | Documented in `correlate.rs` module docs |
| Overhead gate not full Q12 | Docs honest: quick sample, not &lt;2% proof |
| `smoke_probe` always on | **Opt-in:** `OBSAGENT_SMOKE_PROBE` set → attach; smoke0/1/2 scripts set it |

### Verified green (after review fixes, WSL Ubuntu)

```text
wsl-run.sh test-common     → 10 passed
wsl-run.sh test-agent      → 20 passed
wsl-run.sh smoke1          → PASS (with OBSAGENT_SMOKE_PROBE in script)
wsl-run.sh smoke2          → PASS programs, sockio, HTTP rows, unload, pins
wsl-run.sh correctness2    → PASS p50≈51.94ms vs delay=50ms (±10ms)
```

Earlier overhead sample (pre–enter SOCK_FDS enforce, with smoke_probe always on): **~10.7%** `ps` CPU, ~22.6 MiB RSS, drops=0 under `overhead-phase2-quick.sh`. **Re-sample before claiming improvement.**

---

## 3. Locked decisions (do not silently reverse)

Full table: `docs/phases/phase-2-implementation-plan.md` §10.

| ID | Choice | Notes |
|---|---|---|
| **Q1** | Prefix **256 B** | |
| **Q2** | read+write **+ sendto/recvfrom** | glibc `TcpStream` uses sendto/recvfrom only (strace) |
| **Q3** | Enter stash / exit emit `min(ret,N)` | |
| **Q4** | **SOCK_FDS enter + HTTP content exit** | Both required |
| **Q5** | Kind-tagged events on shared `EVENTS` | |
| **Q6** | Leave `SockLatencyEvent` unchanged | |
| **Q7** | Dual/toggle UI (`t`) | |
| **Q8** | Req-half exit → resp-half exit | |
| **Q9** | `METHOD + normalized_path` | |
| **Q10** | Digit segments → `:id` | |
| **Q11** | 60 s SM timeout | |
| **Q12** | Axum + load; row = **quick** sample | Not full 32×30s / not &lt;2% claim |
| **Q13** | Axum `testdata`; p50 within max(±10ms, ±10%) | |
| **Q14** | Redact Auth/Cookie for export; metrics-only UI | Redact **not** on parse hot path |
| **Q15** | `smoke_probe` **opt-in** via env | Default off |
| **Q16** | Attach PASS | smoke2 |

Phase 1 Qs still locked (IPv4-only, enter→exit connect latency ≠ TCP RTT, RingBuf 256KiB / HashMap 8192, etc.).

---

## 4. Environment (critical — easy to get wrong)

Same as Phase 1 handoff. Short form:

| Fact | Detail |
|---|---|
| Host | Windows; **never** build/run BPF on Windows |
| Linux | **`wsl -d Ubuntu`** (not `docker-desktop`) |
| Target dir | **`CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target`** |
| Source | `/mnt/c/projects/eBPF-Observability-Agent` |
| Root gates | `wsl -d Ubuntu -u root` + `scripts/wsl_exec.py` |
| CRLF | Use `wsl_exec.py` / strip before bash |
| Cargo runner | Override via `wsl-run.sh` (`runner=env`) or tests hang on `sudo -E` |

### Canonical commands

```bash
# From Windows PowerShell:
wsl -d Ubuntu -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh build
wsl -d Ubuntu -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh test-common
wsl -d Ubuntu -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh test-agent

# Root gates:
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh smoke0
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh smoke1
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh smoke2
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh correctness2
```

`wsl-run.sh` actions: `preflight|build|test-common|test-agent|smoke0|smoke1|smoke2|correctness2`.

### Env vars

| Var | Meaning |
|---|---|
| `OBSAGENT_HEADLESS=1` | Print metrics; no Ratatui |
| `OBSAGENT_SMOKE_PROBE=1` | Load/attach `smoke_probe` (smoke scripts set this) |
| `RUST_LOG` | aya-log / tracing |

### Manual agent

```bash
wsl -d Ubuntu -u root -- bash -lc '
  export CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target
  OBSAGENT_HEADLESS=1 RUST_LOG=warn $CARGO_TARGET_DIR/release/obsagent
'
# For smoke_probe / Phase 0 regression:
# OBSAGENT_SMOKE_PROBE=1 ...
```

---

## 5. Architecture as implemented (Phase 2)

```
connect/accept exit ≥0  → SOCK_FDS[fd] = 1
close                   → SOCK_FDS delete

sys_enter_{read,write,sendto,recvfrom}
  → if !SOCK_FDS[fd]: return
  → PENDING_IO[tid] = {buf_ptr, fd, dir}

sys_exit_*
  → take PENDING_IO
  → bpf_probe_read_user_buf full 256 B dest (valid len = min(ret,N))
  → if !looks_like_http: drop
  → EVENTS RingBuf SockIoEvent (or DROPS++)

userspace:
  decode(kind) → Latency | Io
  Io → Correlator(tgid,fd) → Exchange → parse_exchange → HttpAggregator
  Latency → Aggregator (Phase 1)
  TUI: HTTP | TCP (toggle t) ; headless: both
```

**Correlation limits (documented, not solved):**

- No TCP reassembly — first HTTP-looking chunk per half only  
- Mid-stream / non-leading slices never join an exchange  
- Server responses via **`sendmsg`/`writev` not probed** — Axum/client path works via sendto/recvfrom on the probe side  

**Correctness gate shape:** interleaved `http-probe --path` list (sequential-only slow probes were flaky).

---

## 6. Key files map

| Path | Role |
|---|---|
| `common/src/lib.rs` | Shared ABI (latency + sockio) |
| `ebpf/src/main.rs` | All BPF programs + maps (single ELF `probes`) |
| `agent/src/main.rs` | Load/attach, drain, TUI/headless, `lock_mut`, smoke opt-in |
| `agent/src/decode.rs` | Kind demux + prefix_len cap |
| `agent/src/correlate.rs` | HTTP SM + shape validation |
| `agent/src/http.rs` | Parse / normalize / redact (export-only) |
| `agent/src/http_agg.rs` | Per-endpoint HTTP metrics |
| `agent/src/agg.rs` | Phase 1 TCP agg |
| `testdata/latency-server` | Axum delay/status server |
| `testdata/http-probe` | Concurrent/multi-path HTTP client |
| `scripts/smoke-milestone2.sh` | Milestone 2 attach + HTTP rows |
| `scripts/correctness-phase2.sh` | Q13 p50 band |
| `scripts/overhead-phase2*.sh` | Overhead samples |
| `docs/phases/phase-2-implementation-plan.md` | Plan + Q1–Q16 |
| `docs/testing/phase-2.tdd.md` | Evidence (note: agent test count may say 17; post-fix = **20**) |
| `docs/phases/phase-3.md` | **Next checklist** |
| `docs/architecture/correlation.md` | Product correlation design |

### Untracked / modified at handoff time (expect this)

**Untracked:** `agent/src/{correlate,decode,http,http_agg}.rs`, `docs/handoff/`, `docs/phases/phase-2-implementation-plan.md`, `docs/testing/phase-2.tdd.md`, `scripts/{smoke-milestone2,correctness-phase2,overhead-phase2*}.sh`, `testdata/{http-probe,latency-server}/`, `benches/http-load.sh`, …

**Modified:** `Cargo.toml` / lock, `agent/`, `common/`, `ebpf/`, `docs/overhead.md`, `docs/phases/phase-2.md`, smoke0/1, `wsl-run.sh`, …

---

## 7. Explicit gaps / next work

**Before Phase 3 (recommended):**

1. **Commit Phase 2** when user asks (do not auto-commit). Suggested scope: all Phase 2 + review-fix files + this handoff.  
2. Optionally re-run `overhead-phase2-quick.sh` after SOCK_FDS enter + smoke opt-in; update `docs/overhead.md` if numbers move.  
3. Sync `docs/testing/phase-2.tdd.md` test counts (20 agent) if committing docs.

**Known limits (do not “fix” silently):**

- No `sendmsg`/`writev` → incomplete **server-side** response capture for some stacks  
- No reassembly / HTTP/2 / pipelining  
- Overhead **above** aspirational &lt;2% on quick burst sample  
- Accept4-only smoke still optional (Phase 1 note)  
- No CI workflow; no push to origin yet  
- IPv6 still deferred  

**Phase 3 owns** (`docs/phases/phase-3.md`, plan + locked Qs in `phase-3-implementation-plan.md`):

- OpenSSL uprobes + `SSL_set_fd`→fd → `TlsIo` → existing correlator (TLS-only latency)
- **Not** dual-plane TLS↔syscall wire merge, Go/rustls/BoringSSL, HTTP/2, OTLP, k8s

---

## 8. Skills / working agreements

User often attaches:

- **ponytail** — shortest working diff; YAGNI; ladder; no speculative abstractions  
- **karpathy-guidelines** — don’t assume; surface tradeoffs; surgical edits; verify  
- **rust-patterns** — idiomatic Rust; `?` over unwrap; enums; minimal `pub`  
- **tdd-workflow** / **intent-driven-development** — gates + evidence before claiming done  

**Do not invent answers** to open product questions; record in phase plan appendix.

Exa MCP was **not** configured; WebSearch used when needed.

**Commits / PRs:** only when user explicitly asks.

---

## 9. Suggested first message in the next chat

Copy-paste (pick one):

**A — commit then Phase 3:**

> Continue from handoff `docs/handoff/SESSION-2026-08-07-phase-2.md`.  
> Phase 2 + review fixes are done but **uncommitted** on top of `b7aeb06`.  
> First: create a git commit for Phase 2 (ask me to confirm message if needed).  
> Then start Phase 3 per `docs/phases/phase-3.md` with `/ponytail` `/karpathy-guidelines` — plan + lock Qs before coding.  
> Dev: `wsl -d Ubuntu`, `CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target`, root via `wsl -d Ubuntu -u root`.

**B — Phase 3 plan only (leave uncommitted):**

> From `docs/handoff/SESSION-2026-08-07-phase-2.md`, write a Phase 3 implementation plan (subphases + open questions) before any TLS code. Do not commit unless I ask.

**C — re-measure overhead only:**

> From Phase 2 handoff, rebuild and re-run `overhead-phase2-quick.sh`; update `docs/overhead.md` Phase 2 row; don’t start Phase 3.

**D — health check only:**

> Verify Phase 2 green from handoff: test-agent, smoke2, correctness2. Report PASS/FAIL only.

---

## 10. Session narrative (chronological)

1. Loaded Phase 1 handoff; prepared Phase 2.  
2. Wrote `phase-2-implementation-plan.md`; user locked Q1–Q15 (later Q16).  
3. Executed Phase 2 with TDD: ABI → decode/correlate/http → eBPF attach → smoke2 → correctness → overhead quick sample.  
4. Empirical revisions: **sendto/recvfrom** (Q2); HTTP content filter (Q4 bring-up); interleaved correctness paths.  
5. Brutal rust-patterns review → user: fix every finding.  
6. Applied cut list: SOCK_FDS enter, correlator shape, decode caps, redact off hot path, mutex, smoke_probe opt-in, docs honesty.  
7. Re-verified gates green. Wrote this handoff. **No Phase 2 commit yet.**

---

## 11. Quick health check (new session should run this first)

```text
1. git log -1 → expect b7aeb06 (Phase 2 still dirty unless committed)
2. git status → expect Phase 2 files modified/untracked
3. wsl … wsl-run.sh test-agent → 20 passed
4. wsl -u root … smoke2 → PASS lines including HTTP rows
5. wsl -u root … correctness2 → PASS p50 within Q13
```

If smoke2 fails: leftover BPF (`bpftool prog list`), kill stray `obsagent`/`latency-server`, confirm release binaries under `CARGO_TARGET_DIR`, confirm `OBSAGENT_SMOKE_PROBE` in smoke scripts if checking `smoke_probe`.

If sockio=0: confirm sendto/recvfrom attached; confirm connect/accept marked SOCK_FDS (client must actually connect through marked fds); confirm HTTP magic filter not rejecting probe traffic.

---

## 12. Deliberate non-goals left on the table

Skipped on purpose (say so before adding):

- `sendmsg`/`writev` attach  
- TCP stream reassembly  
- Split `main.rs` into modules  
- Incremental/windowed `hdrhistogram` without rebuild  
- Full Q12 32-worker × 30s as the only overhead row  
- Claiming host-normalized &lt;2% CPU from `ps` samples  
