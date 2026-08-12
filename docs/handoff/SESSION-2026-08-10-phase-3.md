# Handoff — continue after Phase 3 session

**Full tour:** [`docs/architecture/PROJECT-DEEP-DIVE.md`](../architecture/PROJECT-DEEP-DIVE.md) — architecture, every major code flow, file catalog, mermaid diagrams.

**Purpose:** Drop this file into a new chat so another agent (or you) can resume without rediscovering context.

**Session dates:** 2026-08-10 (Phase 3 plan → lock Qs → implement → gates → `/code-review` → fix high/medium)  
**Repo:** `c:\projects\eBPF-Observability-Agent` (Windows checkout; build/run on **WSL2 Ubuntu**)  
**Git tip (committed):** `main` @ `49e7af5` — *SSL_set_fd → SSL\*→fd SSL_read/write (+ \_ex) → TlsIo → existing correlator → HTTP agg/CLI*  
**Working tree:** **Review fixes UNCOMMITTED** on top of `49e7af5` (4 files). Do **not** assume tip alone has client-only TLS pairing / inode dedupe / `SSL_free` cleanup.  
**Remote:** `main` matches `origin/main` at `49e7af5` (Phase 3 base pushed). Dirty fixes not pushed.  
**Prior handoffs:**  
- `docs/handoff/SESSION-2026-08-06-phase-1.md` — env + Phase 1 ABI  
- `docs/handoff/SESSION-2026-08-07-phase-2.md` — Phase 2 HTTP (still valid; superseded for “what’s next”)

---

## 1. What this project is

Zero-instrumentation observability agent in **Rust + Aya (eBPF)**: reconstruct per-service HTTP latency (and later a service map) from kernel syscalls + TLS uprobes, target **&lt;2% CPU**, later k8s DaemonSet + OTLP.

Phases: **0** toolchain ✅ → **1** connect/accept ✅ → **2** HTTP ✅ → **3** TLS ✅ (base committed; review fixes dirty) → **4** prod → **S** stretch.

---

## 2. What’s done (do not redo)

### Phase 0–2 — committed (see prior handoffs)

Cleartext HTTP via read/write/sendto/recvfrom + SOCK_FDS + correlator + httparse + dual UI. Gates: `smoke2`, `correctness2`.

### Phase 3 — Milestone 3 ✅ (committed @ `49e7af5`)

| Deliverable | Where |
|---|---|
| ABI: `EventKind::TlsIo=4`, `TlsIoEvent` = SockIo twin (288 B / 256 B), `PendingTls` (32 B + `outlen_ptr`) | `common/src/lib.rs` |
| Maps: `SSL_FD`, `TLS_FDS`, `PENDING_TLS` (+ later dirty: `FD_SSL`, `PENDING_TLS_EX`) | `ebpf/src/main.rs` |
| Uprobes: `SSL_set_fd` (+ rfd/wfd), `SSL_write`/`read`, **`SSL_write_ex`/`SSL_read_ex`** | `ebpf/src/main.rs` |
| Q8: skip Phase 2 sock I/O when fd in `TLS_FDS` | `try_enter_io` |
| Soft-fail attach keeps cleartext working | `attach_openssl_uprobes` in `agent/src/main.rs` |
| Decode `TlsIo` demux | `agent/src/decode.rs` |
| TlsIo → correlator → parse → HttpAggregator; headless `tlsio=` | `agent/src/main.rs` |
| Python HTTPS fixtures (stdlib ssl → system OpenSSL, **not rustls**) | `testdata/https-latency-server.py`, `https-probe.py` |
| Gates | `scripts/smoke-milestone3.sh`, `correctness-phase3.sh`, `overhead-phase3-quick.sh` |
| Plan + locked Qs | `docs/phases/phase-3-implementation-plan.md` |
| Checklist | `docs/phases/phase-3.md` (all checked) |
| TDD evidence | `docs/testing/phase-3.tdd.md` |
| Overhead row | `docs/overhead.md` Phase 3 (~3.5% `ps` CPU quick sample; not &lt;2% claim) |

### Empirical revision (recorded — do not reverse)

CPython `_ssl` uses **`SSL_write_ex` / `SSL_read_ex`** (+ `SSL_set_fd`), not classic `SSL_write`/`SSL_read`. Attach **both** families. Testdata stays Python stdlib ssl.

Rust `openssl` crates abandoned (`libssl-dev` apt hung); Python fixtures are the path.

### Review fixes applied (UNCOMMITTED — must ship with next commit)

Brutal `/code-review` + `/rust-patterns` → user: **fix high and medium**. Done:

| Sev | Finding | Fix |
|---|---|---|
| High | Same-host OpenSSL client+server **2× HTTP counts** | `Correlator::observe_client` — TLS only pairs **write→read**; cleartext keeps `observe` (client or server) |
| High | `/lib` + `/usr/lib` same inode → double attach | Dedupe candidates by `(dev, ino)` before attach |
| Med | OpenSSL 1.1 hard-required `_ex` | Classic required; `_ex` soft-load + soft-attach |
| Med | `SSL_FD` never cleaned on close | Reverse map `FD_SSL`; clear on `close` + optional `SSL_free` uprobe |
| Med | Shared `PENDING_TLS` classic vs `_ex` clash | Separate `PENDING_TLS` / `PENDING_TLS_EX` |
| Med | Identical Io/TlsIo drain arms | Explicit branches: Io→`observe`, TlsIo→`observe_client` |

**Dirty files:**

```text
M agent/src/correlate.rs
M agent/src/main.rs
M docs/testing/phase-3.tdd.md
M ebpf/src/main.rs
```

### Verified green (after review fixes, WSL Ubuntu, **after `wsl-run.sh build`**)

```text
wsl-run.sh test-common     → 12 passed
wsl-run.sh test-agent      → 23 passed (includes client_only_* tests)
wsl-run.sh smoke3          → PASS (tlsio, HTTP rows, unload, pins)
wsl-run.sh correctness3    → PASS p50≈50.95ms vs 50ms; GET /slow count=5 (was 10)
```

**Important:** `correctness3` / `smoke3` use the **release** binary under `CARGO_TARGET_DIR`. After source edits, run **`wsl-run.sh build`** before root gates or you validate a stale binary.

Post-fix correctness signature:

```text
http_60s=15 sockio=0 tlsio=60 drops=0
GET /slow count=5 ... p50≈50.95ms
```

`tlsio=60` still includes server halves; only client halves feed HTTP agg (`count=5` for `repeat=5`).

---

## 3. Locked decisions (do not silently reverse)

Full table: `docs/phases/phase-3-implementation-plan.md`.

| ID | Choice | Notes |
|---|---|---|
| **Q1** | `SSL_set_fd` (+ rfd/wfd) → `SSL*`→fd; drop emit if fd unknown | |
| **Q2** | **TLS-only** latency — no dual-plane TLS↔syscall merge | |
| **Q3** | `EventKind::TlsIo`, SockIo layout twin (288 B / 256 B) | |
| **Q4** | 256 B prefix | |
| **Q5–Q6/Q12** | Try-attach `libssl.so.3` / `1.1`; soft-fail; cleartext must keep working | |
| **Q7** | Enter-stash / exit-emit | `_ex` length via `*arg3` when ret==1 |
| **Q8** | Skip Phase 2 sock I/O on TLS-marked fds | |
| **Q9** | HTTPS testdata must use OpenSSL/libssl (not rustls) | Python fixtures |
| **Q10** | p50 within max(±10ms, ±10%) | same band as P2 |
| **Q11** | Metrics-only; never log raw prefixes | |
| **Q14** | Keep Phase 2 cleartext path on | |
| **Post-review** | TLS HTTP pairing **client-only** | Product: outbound request latency, not server SM |

Phase 1–2 Qs still locked (IPv4-only, SOCK_FDS enter + HTTP magic, sendto/recvfrom, smoke_probe opt-in, etc.).

---

## 4. Environment (critical — easy to get wrong)

Same as Phase 1/2. Short form:

| Fact | Detail |
|---|---|
| Host | Windows; **never** build/run BPF on Windows |
| Linux | **`wsl -d Ubuntu`** (not `docker-desktop`) |
| User for build/tests | `sahil` (cargo/rustup) |
| Root gates | `wsl -d Ubuntu -u root` + `scripts/wsl_exec.py` |
| Target dir | **`CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target`** |
| Source | `/mnt/c/projects/eBPF-Observability-Agent` |
| CRLF | Always `wsl_exec.py` / strip before bash (`wsl-run.sh` has CRLF on `/mnt/c`) |

### Canonical commands

```bash
# From Windows PowerShell:
wsl -d Ubuntu -u sahil -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh build
wsl -d Ubuntu -u sahil -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh test-common
wsl -d Ubuntu -u sahil -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh test-agent

# Root gates:
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh smoke2
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh smoke3
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh correctness2
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh correctness3
```

`wsl-run.sh` actions: `preflight|build|test-common|test-agent|smoke0|smoke1|smoke2|smoke3|correctness2|correctness3`.

### Env vars

| Var | Meaning |
|---|---|
| `OBSAGENT_HEADLESS=1` | Print metrics; no Ratatui |
| `OBSAGENT_SMOKE_PROBE=1` | Load/attach `smoke_probe` (smoke0/1/2/3 set this as needed) |
| `RUST_LOG` | aya-log / tracing (`warn` in gates) |

---

## 5. Architecture as implemented (Phase 3)

```
SSL_set_fd / rfd / wfd
  → SSL_FD[ssl*] = fd
  → FD_SSL[(tgid,fd)] = ssl*     (review fix)
  → TLS_FDS[(tgid,fd)] = 1       (Q8 skip sock I/O)

close(fd)
  → clear SOCK_FDS, TLS_FDS, FD_SSL → SSL_FD

SSL_free(ssl*)                   (optional uprobe)
  → clear SSL_FD + reverse + TLS_FDS

SSL_write / SSL_read             → PENDING_TLS[tid]
SSL_write_ex / SSL_read_ex       → PENDING_TLS_EX[tid]  (separate map)
  exit: lookup fd; prefix min(ret|outlen, 256); emit TlsIo

userspace:
  decode → Latency | Io | TlsIo
  Io    → Correlator::observe        (write→read or read→write)
  TlsIo → Correlator::observe_client (write→read only)
       → parse_exchange → HttpAggregator
```

**Pipeline one-liner:**  
`SSL_set_fd → SSL*→fd → SSL_read/write (+_ex) → TlsIo → correlator (client-only) → httparse → HttpAggregator → CLI`

---

## 6. Key files map

| Path | Role |
|---|---|
| `common/src/lib.rs` | ABI: latency + SockIo + TlsIo + PendingTls |
| `ebpf/src/main.rs` | All BPF + OpenSSL uprobes + maps |
| `agent/src/main.rs` | Attach (inode dedupe, soft `_ex`), drain, TUI/headless |
| `agent/src/decode.rs` | Kind demux |
| `agent/src/correlate.rs` | `observe` + **`observe_client`** |
| `agent/src/http.rs` / `http_agg.rs` | Parse / HTTP metrics (unchanged path) |
| `testdata/https-*.py` | OpenSSL HTTPS server + probe |
| `scripts/smoke-milestone3.sh` | Milestone 3 gate |
| `scripts/correctness-phase3.sh` | Q10 p50 band |
| `docs/phases/phase-3-implementation-plan.md` | Plan + Q appendix |
| `docs/testing/phase-3.tdd.md` | Evidence |
| `docs/phases/phase-4.md` | **Next checklist** (if present) / ROADMAP |

---

## 7. Explicit gaps / next work

**Immediate (this dirty tree):**

1. **Commit review fixes** when user asks (do not auto-commit). Scope: the 4 dirty files + this handoff.  
2. Optionally bump `phase-3.tdd.md` agent test count **21 → 23**.  
3. Re-run `smoke2` after commit if you want cleartext regression on the fixed binary (was green earlier in session).

**Known limits (do not “fix” silently without product lock):**

- No dual-plane TLS↔syscall wire merge (Q2)  
- Go / rustls / BoringSSL not first-class  
- No HTTP/2, reassembly, pipelining  
- Split rfd/wfd last-write-wins in `SSL_FD`  
- Overhead quick sample ~3.5% — not &lt;2% proof  
- Headless `events_60s={tcp_total}` label still misleading (pre-existing)  
- Empty leftover `testdata/https-*` rust dirs (if any) are noise  

**Phase 4 owns** (only when user asks): service map / OTLP / DaemonSet / prod hardening per `docs/ROADMAP.md` + phase-4 docs. **Do not start Phase 4 until asked.**

---

## 8. Skills / working agreements

User often attaches:

- **ponytail** — shortest working diff; YAGNI  
- **karpathy-guidelines** — don’t assume; surgical; verify  
- **rust-patterns** — idiomatic Rust; `?` over unwrap  
- **tdd-workflow** / **intent-driven-development** — gates + evidence  

**Do not invent** product answers; record in phase plan appendix.  
**Commits / PRs / push:** only when user explicitly asks.

---

## 9. Suggested first message in the next chat

Copy-paste (pick one):

**A — commit review fixes then stop:**

> Continue from `docs/handoff/SESSION-2026-08-10-phase-3.md`.  
> Phase 3 base is `49e7af5`; review fixes are **uncommitted**.  
> Create a git commit for the 4 dirty files + this handoff. Do not start Phase 4.

**B — health check only:**

> From Phase 3 handoff: `build`, `test-agent`, `smoke3`, `correctness3`. Expect `/slow count=5`. Report PASS/FAIL only.

**C — start Phase 4 (only if intentional):**

> From `docs/handoff/SESSION-2026-08-10-phase-3.md`, plan Phase 4 before coding (service map / OTLP / DaemonSet). Lock open Qs first. Commit Phase 3 review fixes first if still dirty.

**D — push after commit:**

> Commit Phase 3 review fixes per handoff, then push `main` to origin (only if I confirm).

---

## 10. Session narrative (chronological)

1. Loaded Phase 1/2 handoffs; Phase 2 already on `main` (merge + tip).  
2. Designed Phase 3; locked Q1–Q12 (TLS-only M3, no dual-plane).  
3. Implemented: ABI → eBPF uprobes → decode/wire → Python HTTPS fixtures → smoke3/correctness3/overhead.  
4. Empirical: attach `_ex` for CPython.  
5. User committed/pushed Phase 3 base → `49e7af5`.  
6. `/code-review`: found 2× HTTP counts + inode double-attach + map cleanup gaps.  
7. Fixed high+medium; rebuilt release; correctness3 shows `count=5`. Wrote this handoff. **Review fixes not committed.**

---

## 11. Quick health check (new session should run this first)

```text
1. git log -1 → expect 49e7af5 (or later if review fixes committed)
2. git status → expect 4 modified files UNLESS committed
3. wsl … wsl-run.sh build          # required if dirty
4. wsl … wsl-run.sh test-agent     → 23 passed
5. wsl -u root … smoke3            → PASS
6. wsl -u root … correctness3      → PASS; GET /slow count=5 (not 10)
```

If `count=10` again: stale release binary or `observe_client` missing — rebuild.  
If `tlsio=0`: confirm `_ex` attached (`bpftool prog list | grep ssl`); confirm Python OpenSSL (`ssl.OPENSSL_VERSION`); confirm `SSL_set_fd` ran.  
If cleartext broke: soft-fail attach regression — check `attach_openssl_uprobes` still returns without aborting agent.

---

## 12. Deliberate non-goals left on the table

Skipped on purpose (say so before adding):

- Dual-plane TLS↔syscall correlation engine  
- Go / rustls / BoringSSL first-class support  
- HTTP/2, gRPC framing, TCP reassembly  
- Server-side TLS HTTP metrics (client-only by design post-review)  
- Full &lt;2% CPU proof from long load  
- Phase 4 OTLP / k8s DaemonSet / service map  
- Splitting god `main.rs`  
