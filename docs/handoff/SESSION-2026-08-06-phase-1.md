# Handoff — continue after Phase 1 session

**Purpose:** Drop this file into a new chat so another agent (or you) can resume without rediscovering context.

**Session dates:** 2026-08-06 → 2026-08-07 (plan + Phase 1 execution)  
**Repo:** `c:\projects\eBPF-Observability-Agent` (Windows checkout; build/run on WSL2 Ubuntu)  
**Branch tip:** `main` @ `b7aeb06` — *feat: Phase 1 connect/accept latency MVP with RingBuf CLI*  
**Remote:** local `main` is **ahead of `origin/main` by 5 commits** (not pushed unless you push)

---

## 1. What this project is

Zero-instrumentation observability agent in **Rust + Aya (eBPF)**: reconstruct per-service HTTP/gRPC latency and a service map from kernel syscalls + TLS uprobes, target **&lt;2% CPU**, deployable as a k8s DaemonSet with OTLP later.

**Resume-signal product claim (README):** eBPF agent that reconstructs latency/service map from syscall + TLS uprobe data with &lt;2% overhead.

Phases: **0** toolchain → **1** connect/accept latency CLI → **2** HTTP → **3** TLS → **4** prod (OTLP/DaemonSet) → **S** stretch.

---

## 2. What’s done (do not redo)

### Phase 0 — Milestone 0 ✅

- Cargo workspace: `agent/` (`obsagent`), `ebpf/` (`obsagent-ebpf`, bin `probes`), `common/` (`obsagent-common`)
- Smoke kprobe on `try_to_wake_up` → `aya-log` → clean unload
- Scripts: `scripts/preflight.sh`, `scripts/smoke-milestone0.sh`
- Evidence: `docs/testing/phase-0.tdd.md`
- Plan (historical): `docs/phases/phase-0-implementation-plan.md`

### Phase 1 — Milestone 1 ✅ (this session)

| Deliverable | Where |
|---|---|
| Event ABI (`SockLatencyEvent` 48B, `PendingEnter`, kinds, map size consts) | `common/src/lib.rs` |
| Tracepoints: `enter_connect` / `exit_connect` / `enter_accept4` / `exit_accept4` | `ebpf/src/main.rs` |
| Maps: `PENDING` HashMap(8192), `EVENTS` RingBuf(256KiB), `DROPS` Array(1) | `ebpf/src/main.rs` |
| Keep `smoke_probe` (Q9) | `ebpf/src/main.rs` + agent attach |
| Tokio RingBuf drain + drop scrape | `agent/src/main.rs` |
| Rolling 60s agg (hdrhistogram p50/p95/p99), key = remote IP:port + direction | `agent/src/agg.rs` |
| Ratatui table + sparkline (TTY); headless if `OBSAGENT_HEADLESS` or non-TTY | `agent/src/main.rs` |
| Smoke gate | `scripts/smoke-milestone1.sh` |
| Overhead sample | `docs/overhead.md`, `benches/connect-load.sh`, `scripts/overhead-phase1.sh` |
| Implementation plan + Q appendix filled | `docs/phases/phase-1-implementation-plan.md` |
| TDD evidence | `docs/testing/phase-1.tdd.md` |
| Checklist | `docs/phases/phase-1.md` (all checked; accept smoke optional note) |

**Verified green (on WSL Ubuntu):**

- `wsl-run.sh test-common` → 4 passed  
- `wsl-run.sh test-agent` → 5 passed  
- `smoke0` + `smoke1` → all PASS  
- Overhead: ~**1.6%** CPU (`ps`), ~**22.5 MiB** RSS, drops=0 under `connect-load.sh 50`

---

## 3. Locked decisions (do not silently reverse)

Filled in `docs/phases/phase-1-implementation-plan.md` §10:

| ID | Choice | Meaning |
|---|---|---|
| **Q1** | PASS | `sys_enter/exit_connect` + accept4 TPs attach/fire on this kernel |
| **Q2** | n/a | Q1 passed — no kprobe fallback needed |
| **Q3** | **sockaddr** via `bpf_probe_read_user` | CO-RE / `aya-tool` **deferred** |
| **Q4** | IPv4 only | IPv6 = Phase 2+ |
| **Q5** | Remote endpoint **+ direction** (connect/accept) | No PID in agg key |
| **Q6** | `hdrhistogram` | Matches OVERVIEW |
| **Q7** | Latency = **syscall enter→exit only** | `-EINPROGRESS` ≠ TCP established; TCP state = Phase 2+ |
| **Q8** | RingBuf **256 KiB**, HashMap **8192** | |
| **Q9** | **Keep** `smoke_probe` alongside Phase 1 programs | Remove only after Milestone 1 gate fully replaces M0 |
| **Q10** | `benches/connect-load.sh` | Load definition locked |
| **Q11** | Privilege: **`wsl -d Ubuntu -u root`** | `setcap` later (ops hardening, not milestone) |

Also still locked from Phase 0: one BPF ELF (`probes`), embed via `OUT_DIR/probes`, monotonic ns (`bpf_ktime_get_ns`), RingBuf + drop counter backpressure.

---

## 4. Environment (critical — easy to get wrong)

| Fact | Detail |
|---|---|
| Host | Windows; **never** build/run BPF on Windows |
| Linux target | **`wsl -d Ubuntu`** (default WSL distro is often `docker-desktop` — **no bash**) |
| Kernel | `6.6.114.1-microsoft-standard-WSL2`, BTF present |
| Build user | `sahil`; toolchain in `/home/sahil/.cargo` |
| Target dir | **`CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target`** (not `/tmp` — WSL wipes `/tmp`) |
| Source tree | `/mnt/c/projects/eBPF-Observability-Agent` (9p; OK for source) |
| Privileged runs | `wsl -d Ubuntu -u root` (Q11); smoke scripts detect uid=0 |
| Scripts CRLF | Windows checkout → strip CR before bash. Helper: `scripts/wsl_exec.py` |
| Cargo runner | `.cargo/config.toml` has `runner = "sudo -E"` → **tests hang** without override. Use `wsl-run.sh test-*` which sets `runner=env` |

### Canonical commands

```bash
# From Windows PowerShell:
wsl -d Ubuntu -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh preflight
wsl -d Ubuntu -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh build
wsl -d Ubuntu -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh test-common
wsl -d Ubuntu -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh test-agent

# Root gates:
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh smoke0
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py wsl-run.sh smoke1
wsl -d Ubuntu -u root -- python3 /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl_exec.py overhead-phase1.sh
```

`wsl-run.sh` sources `/home/sahil/.cargo/env` even when run as root.

### Run the agent manually

```bash
wsl -d Ubuntu -u root -- bash -lc '
  export CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target
  # Headless (scripts / no TTY):
  OBSAGENT_HEADLESS=1 RUST_LOG=info $CARGO_TARGET_DIR/release/obsagent
  # Or TTY Ratatui: run interactively inside Ubuntu, then q / Ctrl-C
'
```

---

## 5. Architecture as implemented (Phase 1)

```
sys_enter_connect  → PENDING[tid] = {ts, daddr, dport}
sys_exit_connect   → latency = now-ts → EVENTS RingBuf (or DROPS++)
sys_enter_accept4  → PENDING[tid] = {ts, sockaddr_ptr}
sys_exit_accept4   → read peer sockaddr if ret>=0 → EVENTS

userspace: AsyncFd(RingBuf) → Aggregator(60s) → Ratatui | headless println
           periodic DROPS[0] scrape → UI "drops="
```

**Tracepoint offsets** (this kernel’s format files — baked into `ebpf/src/main.rs`):

- enter sockaddr ptr @ offset **24**
- exit ret @ offset **16**

**IPv4 address display:** `daddr_be` is raw `sin_addr.s_addr` memory bytes; label uses `Ipv4Addr::from(daddr_be.to_ne_bytes())` + `u16::from_be(dport_be)`. Don’t “fix” endian without retesting labels.

**Latency semantics (Q7):** document in UI: syscall enter→exit; not TCP handshake RTT.

---

## 6. Key files map

| Path | Role |
|---|---|
| `common/src/lib.rs` | Shared ABI |
| `ebpf/src/main.rs` | All BPF programs + maps (single ELF) |
| `agent/src/main.rs` | Load/attach, RingBuf, TUI/headless |
| `agent/src/agg.rs` | Rolling window + tests |
| `agent/build.rs` | `aya-build` → nightly BPF build |
| `docs/phases/phase-1-implementation-plan.md` | Phase 1 how-to + Q appendix |
| `docs/phases/phase-2.md` | **Next checklist** |
| `docs/architecture/*` | Full-product design (correlation, TLS, backpressure) |
| `docs/testing/phase-1.tdd.md` | Proof of Phase 1 |

---

## 7. Explicit gaps / follow-ups (before or during Phase 2)

**Optional Phase 1 polish (not blocking Phase 2):**

1. Accept4 **smoke** — programs attached; `smoke1` only generates **connect** traffic  
2. Forced RingBuf overflow test (drop counter automation still “partial”)  
3. Host-normalized overhead via `perf` (current number is `ps` %CPU)  
4. Remove `smoke_probe` only after deciding M1 gate fully replaces M0  
5. `setcap cap_bpf,cap_perfmon` least-privilege proof (Phase 0 Step 9.2 deferred)  
6. CI workflow still not added  
7. Push 5 local commits to `origin` when ready  

**Phase 2 owns (do not sneak into Phase 1):**

- Socket `read`/`write`/`recvfrom`/`sendto` bounded prefix capture  
- Correlation SM on `(pid, fd, 4-tuple)` — `docs/architecture/correlation.md`  
- Userspace `httparse`, path normalize, per-endpoint HTTP metrics  
- Correctness vs injectable-latency server in `testdata/`  
- New overhead row for Phase 2  

Start from: `docs/phases/phase-2.md`. Prefer writing a Phase 2 implementation plan (same style as Phase 0/1 plans) **before** coding; lock open questions, don’t invent ABI fields “for later.”

---

## 8. Skills / working agreements used this session

When continuing, user often attaches:

- **ponytail** — shortest working diff; YAGNI; no speculative abstractions  
- **karpathy-guidelines** — don’t assume; surface tradeoffs; surgical edits  
- **tdd-workflow** — tests/gates before claiming done; evidence in `docs/testing/*.tdd.md`  
- **intent-driven-development** — acceptance criteria; blocking Qs before ABI freeze  
- **coding-standards** / **backend-patterns** — as applicable  

**Do not invent answers** to open product questions; record them in the phase plan appendix like Q1–Q11.

Exa MCP was **not** configured; WebSearch / Aya book used for API checks instead.

---

## 9. Suggested first message in the next chat

Copy-paste:

> Continue eBPF Observability Agent from handoff `docs/handoff/SESSION-2026-08-06-phase-1.md`.  
> Phase 0+1 done at `b7aeb06`. Next: Phase 2 HTTP awareness per `docs/phases/phase-2.md`.  
> Use `/ponytail` `/karpathy-guidelines` — no assumptions; write a Phase 2 implementation plan (phases/subphases + open questions) before coding unless I say execute immediately.  
> Dev target: `wsl -d Ubuntu`, `CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target`, root via `wsl -d Ubuntu -u root`.

Or if polishing Phase 1 first:

> From handoff `docs/handoff/SESSION-2026-08-06-phase-1.md`, add accept4 coverage to `smoke-milestone1.sh` and document evidence; don’t start Phase 2 yet.

---

## 10. Session narrative (what we did chronologically)

1. Audited repo vs Phase 1 checklist — Phase 0 done, Phase 1 zero code.  
2. Wrote `docs/phases/phase-1-implementation-plan.md` (inventory, architecture, subphases, **open Q1–Q11**).  
3. User locked Q4–Q9 + Q11.  
4. Executed Phase 1 with TDD: ABI tests → agg tests → eBPF + agent → smoke1 → overhead.  
5. Recorded Q1 PASS, Q3=sockaddr; committed `b7aeb06`.

---

## 11. Quick health check (new session should run this first)

```text
1. git log -1 → expect b7aeb06 (or descendant)
2. wsl -d Ubuntu … wsl-run.sh preflight → OK
3. wsl -d Ubuntu … wsl-run.sh test-agent → 5 passed
4. wsl -d Ubuntu -u root … smoke1 → 4 PASS lines
```

If smoke1 fails: check leftover BPF (`bpftool prog list`), kill stray `obsagent`, confirm `CARGO_TARGET_DIR` points at the built binary.
