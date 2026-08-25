# Phase 11 — Implementation Plan (Overhead + sampling)

Companion to [phase-11.md](phase-11.md). Locked: [VISION-95.md](VISION-95.md) Q14, Q15 (sticky-fd amendment).

**Phase 11 buys:** a defensible CPU number. Profiles (Phase 12) stay **off** during the 2% gate (`OBSAGENT_PROFILE` unset).

**Gate from Phase 10:** measure on the **real node** that runs the agent, not WSL kind. Unit maps and tests do not wait on M10.

---

## 0. Out of Phase 11

Lowering RPS to fake 2%. Claiming Phase 1 connect-only 1.6% `ps`. Enabling CPU profiling during the headline run. Per-event `prandom % N` on SockIo/TlsIo. Growing the RingBuf. Prefix 128 as default (trial only if 11.4 misses).

---

## 1. Locked decisions

| ID | Choice |
|---|---|
| **P11-Q1** | Load: **500 RPS HTTP/1.1 + 200 RPS h2/gRPC**, 60 s, **3** runs. Script + `benches/vision95-load.md`. |
| **P11-Q2** | Metric: `perf stat -p <agent_pid>` CPU of the **agent process**. Headline = mean of 3 of `(agent_cpu_sec / wall_sec) * 100` as **% of one core**. Footnote: eBPF probe time runs on syscall CPUs. |
| **P11-Q3** | Target: mean **&lt; 2.0**. If 2.0–5.0: iterate sampling/deny-BPF/prefix. If still ≥ 2.0: **change the resume number**; do not claim 2%. |
| **P11-Q4′** | Sticky `KEEP_IO` on `SOCK_META` (bits 1–2). `SAMPLE_N` Array. `n≤1` emit all I/O. Draw once per `(tgid,fd)`. `SockIoTimes` follows the bit. Latency/handshake never sampled away. |
| **P11-Q5** | Userspace `events_dropped` still from `DROPS` (reserve fail only). Add `obsagent.sample_n` gauge. Sample skip ≠ drop. |
| **P11-Q6** | Overload script: 5k RPS HTTP/1.1, 15 s. PASS if dropped **or** sample_n ≥ 2, and at least some HTTP series still increment. RSS bounded. |
| **P11-Q7′** | Capture maps **this phase**: deny-list `DENIED_TGID` (self tgid + deny comms) or allow-only `ALLOWED_TGID` + `ALLOW_ONLY`. 1 s `/proc` sweep of **thread-group leaders** (not tids) on a task off the RingBuf drain. Lazy deny insert only in deny-list mode, and only on successful insert. Prefix 128 still optional (11.5). |
| **P11-Q8** | Prefix 128 B trial only if still miss; default remains 256 unless handoff amends VISION Q (prefix). |
| **P11-Q9** | Loadgen: **vegeta v12.13.0** (HTTP/1.1) + **ghz v0.121.0** (gRPC). Wide connection pool (`-workers`/`--connections`). |
| **P11-Q10** | Headline is `perf stat -p` only. No cgroup `cpu.stat` as a second official %. |

Auto N: start 1; `drops_delta>0` → `min(16, n*2)`; 30 s of zero deltas → `max(1, n/2)`. `OBSAGENT_SAMPLE_N` pins and disables auto. Spans stay `OBSAGENT_TRACE_SAMPLE`.

---

## 2. Architecture (sampling)

```text
emit / stash / SSL enter:
  if !tgid_captured(tgid): return   # deny-list: DENIED_TGID; allow-only: ALLOWED_TGID
  connect/accept/handshake: always emit (if captured)
  at SOCK_META insert (connect/accept): draw KEEP_IO unless already decided
  SockIo / TlsIo / SockIoTimes: existing SOCK_META only — unmarked fd does not insert
  else reserve or DROPS++
```

Userspace: drain 1 s tick reads DROPS and sets SAMPLE_N. A **separate** task sweeps `/proc` (thread-group leaders only) into `DENIED_TGID` or `ALLOWED_TGID`. `obsagent.sample_n` updates only after a successful SAMPLE_N map write.

Documented in `docs/architecture/ring-buffer-backpressure.md`.

---

## 3. Subphases

### 11.0 — Preconditions

Profile flag off. M10 is the **measurement** gate for 11.4, not a compile gate.

### 11.1 — Load protocol doc + scripts

**Work:** `benches/vision95-load.md`, `scripts/overhead-vision95.sh`, `scripts/overload-vision95.sh`.

**Verify:** scripts run **without** agent (baseline RPS actually hits 500/200) on a node that has the demo.

### 11.2 — BPF SAMPLE_N (sticky)

**Work:** map + sticky flags + enter-path gate; userspace setter; metric.

**Verify:** unit tests — many fds at N=10 keep ~1/10; single-fd I/O is all-or-nothing. Auto double/halve.

### 11.2b — Capture maps (`DENIED_TGID` / `ALLOWED_TGID`)

**Work:** maps + `/proc` leader sweep (separate task) + lazy deny insert (deny-list only, insert Ok).

**Verify:** filter tests with a fake proc tree including a non-leader tid; kubelet comm is in the deny set; allow-only lists the allowed tgid only.

### 11.3 — Overload gate

**Work:** 5k RPS script.

**Verify:** drop or N↑; process RSS bounded (no unbounded userspace queue).

### 11.4 — Headline three runs

**Work:** `perf stat` × 3 on the M10 node. Fill `docs/overhead.md` Vision-95 table.

**Verify:** either &lt;2% **or** written replacement % and ROADMAP note “2% wording failed”.

### 11.5 — Optional prefix trial

Only if 11.4 miss. Then re-run 11.4.

### 11.6 — Close

Tick code items in phase-11.md / TDD. Do **not** tick Milestone 11 until 11.4 exists.

---

## 4. Success criteria

1. Protocol pinned and reproducible.
2. Sampling + deny maps implemented and unit-tested.
3. Number published with kernel + load line + date **when 11.4 runs**.
4. 2% sentence legal iff mean &lt; 2.0.

---

## 5. Appendix — §10

| ID | Answer | Date | Notes |
|---|---|---|---|
| P11-Q9 | vegeta v12.13.0 + ghz v0.121.0 | 2026-08-20 | HTTP/1.1 + gRPC |
| P11-Q10 | perf -p only | 2026-08-20 | no cgroup headline |
| Headline % | *(fill on 11.4)* | | mean of 3 |
