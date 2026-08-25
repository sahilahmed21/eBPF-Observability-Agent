# Phase 6 — Implementation Plan (Capture completeness)

Companion to [phase-6.md](phase-6.md). Locked Qs from [VISION-95.md](VISION-95.md) Q8, Q9, Q13, Q17.

**Phase 6 buys one thing:** the HTTP/1.1 plane sees **real** socket I/O (`writev`/`sendmsg`/…) and split writes, with cardinality control. No HTTP/2, no dual-plane, no traces.

**Gate from Phase 5:** Milestone 5 green (`test-agent`, `smoke3`, `correctness3`, `smoke4`). `writev`/`sendmsg` are **not** attached. Reassembly does **not** exist. `SOCK_META` is 8 B IPv4-only.

---

## 0. Inventory — done vs missing

### Done (Phases 0–5)

| Item | Evidence |
|---|---|
| Sock I/O TPs: `read`/`write`/`sendto`/`recvfrom` | `ebpf/src/main.rs` |
| HTTP magic in BPF before RingBuf emit | `looks_like_http_fixed` |
| Correlator `(tgid,fd)`, httparse, 60 s timeout | `agent/src/correlate.rs` |
| `SOCK_META` 8 B IPv4 | `common/` |
| Correctness3 `/slow` p50 band | `docs/testing/phase-5.tdd.md` |
| Comm deny-list | **missing** |
| IPv6 | **missing** (`AF_INET` only) |

### Missing

| Item | Current state |
|---|---|
| `sys_enter/exit_{writev,readv,sendmsg,recvmsg}` | Not attached |
| First-iovec / `msghdr` probe-read | Not present |
| Reassembly buffer | Single-chunk prefixes only |
| Emit non-magic bytes on marked fds | Magic drop in BPF |
| `OBSAGENT_COMM_DENY` | Not present |
| `SockMeta` v6 | 8 B v4 |

### Explicitly out of Phase 6

HTTP/2, gRPC, dual-plane TLS, handshake, OTLP traces, real-node identity, `&lt;2%` claim, CPU profiles, Go/rustls, XDP, `io_uring`, BPF deny-map (Phase 11).

---

## 1. Locked decisions (do not silently reverse)

| ID | Choice |
|---|---|
| **P6-Q1** | Attach **all four**: `readv`, `writev`, `sendmsg`, `recvmsg` (VISION Q8). |
| **P6-Q2** | Copy **first iovec only**, 256 B cap. `writev(fd, iov, n)`: probe-read `iov[0]`. `sendmsg`: probe-read `msghdr` then `msg_iov[0]`. x86_64 offsets recorded in `ebpf/` comments (same style as `ENTER_FD_OFF`). |
| **P6-Q3** | On `SOCK_META`-marked fds, **do not** apply `looks_like_http_fixed` in BPF. Emit prefixes; userspace filters + reassembles. Unmarked fds: no emit (unchanged). |
| **P6-Q4** | Reassembly: userspace map `(tgid, fd)` → buf, cap **8 KiB**, timeout **60 s**. Append until `httparse` Complete **or** cap **or** timeout. Then existing correlator. |
| **P6-Q5** | Deny-list in **userspace** this phase. Env `OBSAGENT_COMM_DENY` (comma). k8s defaults: `dockerd`, `containerd`, `kubelet`, `kube-proxy`. Optional `OBSAGENT_COMM_ALLOW` (if set, allow-only). Filter **before** correlate. |
| **P6-Q6** | `SockMeta` expands for IPv6 (see §4.2). `SockLatencyEvent` stays **48 B IPv4**. v6 connect still fills `SOCK_META`; latency table may show `daddr_be=0` for v6. |
| **P6-Q7** | Testdata: in-repo server that **must** respond via `writev` (`ldd`/strace or explicit `libc::writev`). Reuse `/slow` 50 ms. |
| **P6-Q8** | Correctness band unchanged: p50 within `max(±10 ms, ±10%)` of 50 ms. |
| **P6-Q9** | Keep Phase 2–5 attach set. Additive probes only. |
| **P6-Q10** | Overhead: named script + `docs/overhead.md` Phase 6 row. **Not** the Vision-95 `&lt;2%` gate. |

### Still open (answer during execution, write in §10)

| # | Question | Blocks | How to resolve |
|---|---|---|---|
| **P6-Q11** | Exact `msghdr` / `iovec` offsets on this kernel | 6.2 | `pahole` / format file / one-off C dump; paste into `ebpf/` comment |
| **P6-Q12** | Does `sys_enter_writev` fire for the testdata server on WSL? | 6.2 | strace + smoke6 |
| **P6-Q13** | Phase 6 overhead load (clone phase-2-quick vs writev server) | 6.7 | Write script first |

---

## 2. Architecture

### 2.1 I/O path (additive)

```text
read/write/sendto/recvfrom     (existing)
readv/writev/sendmsg/recvmsg   (new)
        │
        ▼ enter: stash {fd, buf_ptr from iov[0], dir} in PENDING_IO
        ▼ exit:  copy min(ret, 256); if SOCK_META hit → emit SockIo (no HTTP magic)
        ▼
userspace decode → comm deny-list → reassemble(tgid,fd) → correlator → httparse
```

### 2.2 `SockMeta` v6 layout (lock)

Replace 8 B struct with **24 B**:

```text
#[repr(C)]
struct SockMeta {
    family: u8,          // AF_INET=2, AF_INET6=10
    flags: u8,           // FLAG_HAS_ADDR
    dport_be: u16,
    daddr: [u8; 16],     // v4: bytes [0..4] = s_addr network order, rest 0
    _pad: [u8; 4],       // align 8 → 24 B
}
```

Update `with_peer` → `with_peer_v4` / `with_peer_v6`. `format_ip_port` dual-stack. Size test **must** change from 8 → 24. `unsafe impl Pod`.

### 2.3 `iovec` / `msghdr` (x86_64 Linux, confirm P6-Q11)

```text
struct iovec { iov_base: u64, iov_len: u64 }  // 16 B

struct msghdr {
    msg_name: u64,       // 0
    msg_namelen: u32,    // 8
    _pad: u32,           // 12
    msg_iov: u64,        // 16  ← probe-read this, then iovec[0]
    msg_iovlen: u64,     // 24
    ...
}
```

If P6-Q11 disagrees, **amend this section in the handoff** before shipping offsets.

### 2.4 Reassembly API

New module `agent/src/reassemble.rs`:

```text
fn push(&mut self, ev: &SockIoEvent) -> Option<SockIoEvent>
// returns a synthetic event with concatenated prefix when Complete or cap
```

Feed **that** into `Correlator::observe`. TLS `TlsIo` uses the same buffer keyed `(tgid, fd)` (prepare for Phase 7/8; no dual-plane yet).

### 2.5 Files (expected)

| Path | Change |
|---|---|
| `common/src/lib.rs` | `SockMeta` 24 B; `AF_INET6`; size tests |
| `ebpf/src/main.rs` | four TPs; iov/msghdr read; drop HTTP magic on marked fds |
| `agent/src/main.rs` | attach TPs; deny-list; reassemble before correlate |
| `agent/src/reassemble.rs` | **new** |
| `agent/src/filter.rs` | **new** — comm allow/deny |
| `agent/src/peer_cache.rs` / `identity` / `service_map` | v6 labels |
| `testdata/writev-server/` | **new** (or flag on latency-server) |
| `scripts/smoke-milestone6.sh` | **new** |
| `scripts/correctness-phase6.sh` | **new** |
| `scripts/wsl-run.sh` | `smoke6` / `correctness6` arms |
| `docs/overhead.md` | Phase 6 row |
| `docs/handoff/SESSION-phase-6.md` | deny-list evidence, offsets |

---

## 3. Subphases

Every subphase: intent → work → verify. Done = verify passes.

### 6.0 — Preconditions · gate

**Intent:** Phase 5 still green.

**Work:** `wsl-run.sh test-common`, `test-agent`, `correctness3`. Add `smoke6`/`correctness6` stubs to `wsl-run.sh` that fail until scripts exist **or** add arms when scripts land in 6.6.

**Verify:** correctness3 PASS. Tick this subphase in the TDD file.

---

### 6.1 — `SockMeta` IPv6 ABI · foundation

**Depends on:** P6-Q6.

**Work:** Expand `SockMeta`; `read_sockaddr` v4 **or** v6 in BPF (`AF_INET6=10`). `connect`/`accept4` fill v6. Userspace `format_ip_port` + unit tests (`::1`, v4-mapped). Do **not** change `SockLatencyEvent` size.

**Verify:** `test-common` — `SOCK_META_SIZE == 24`; old 8 B test **updated**. Unit: v4 still formats `1.2.3.4:80`.

---

### 6.2 — Vectored / msg syscall probes · kernel path

**Depends on:** P6-Q1, Q2, Q3, Q11, Q12.

**Work:**

1. Confirm offsets (P6-Q11); comment in `ebpf/src/main.rs`.
2. `try_enter_iov` / `try_enter_msghdr` → same `PENDING_IO`.
3. Reuse `try_exit_io` → `emit_io_kind`.
4. **Remove** `looks_like_http_fixed` skip for marked fds (P6-Q3). Keep it off unmarked (no emit anyway).
5. Attach four TPs in `agent/src/main.rs`.

**Verify:** programs load (`bpftool` / smoke). P6-Q12 PASS/FAIL in §10. If FAIL, stop and fix testdata/offsets.

**Verifier:** if stack overflow on `msghdr`, use `PerCpuArray` scratch. Paste any reject into `docs/verifier-rejection-log.md`.

---

### 6.3 — Reassembly · userspace

**Depends on:** P6-Q4.

**Work:** `reassemble.rs` with unit tests:

- two chunks `"GET /slow HTTP/1.1\r\n"` + `"Host: x\r\n\r\n"` → one parseable request
- cap at 8 KiB
- timeout evicts

Wire in drain **before** `Correlator::observe`.

**Verify:** `test-agent` new tests GREEN. Existing correlate tests still pass.

---

### 6.4 — Comm deny-list · cardinality

**Depends on:** P6-Q5.

**Work:** `filter.rs`; read env at startup; skip events whose `/proc/<tgid>/comm` matches deny (use `IdentityResolver` comm). Defaults when `OBSAGENT_K8S_DENY=1` or when running with kube config.

**Verify:** unit tests for parse + match. Headless: inject fake comm `dockerd` → no HTTP row.

---

### 6.5 — Writev testdata · fixture

**Depends on:** P6-Q7.

**Work:** Server whose response path calls `writev`. Document strace one-liner in testdata README. `/slow` sleeps 50 ms **in the handler**, then writev.

**Verify:** `strace -e writev` shows the response. HTTP client still gets 200.

---

### 6.6 — Smoke + correctness · gate

**Depends on:** P6-Q8.

**Work:**

- `scripts/smoke-milestone6.sh`: programs listed include `enter_writev`/`exit_writev`; sockio &gt; 0 against writev-server.
- `scripts/correctness-phase6.sh`: clone correctness3; target writev-server; assert p50 band; assert `count` matches probe N.
- Split-header: `http-probe` or a tiny client that writes the request in two `write`s; assert one HTTP row.
- Wire `wsl-run.sh smoke6` / `correctness6`.

**Verify:** `correctness6` PASS; split-header PASS line in script output.

---

### 6.7 — Overhead · measurement

**Depends on:** P6-Q10, Q13.

**Work:** `scripts/overhead-phase6.sh`; fill `docs/overhead.md`. Honest `ps` or `perf` — **no** `&lt;2%` claim.

**Verify:** row exists.

---

### 6.8 — Close

**Work:** Tick [phase-6.md](phase-6.md); fill [phase-6.tdd.md](../testing/phase-6.tdd.md); handoff `docs/handoff/SESSION-phase-6.md`; fill §10.

**Verify:** ROADMAP Phase 6 boxes checked.

---

## 4. Success criteria (Milestone 6)

1. Writev-backed `/slow` p50 in band.
2. Split-header → one exchange.
3. `::1` connect appears as dst on the map (unit or smoke).
4. Deny-list unit tests + one noisy-comm skip.
5. correctness3 still PASS (regression).
6. Overhead row recorded.

---

## 5. Appendix — §10 answers

| ID | Answer | Date | Notes |
|---|---|---|---|
| P6-Q1–Q10 | locked above | 2026-08-18 | VISION-95 |
| P6-Q11 | *(fill)* | | msghdr offsets |
| P6-Q12 | *(fill)* | | writev TP fires? |
| P6-Q13 | *(fill)* | | overhead script name |
