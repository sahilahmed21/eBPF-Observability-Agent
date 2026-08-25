# Phase 8 — Implementation Plan (Dual-plane TLS + handshake)

Companion to [phase-8.md](phase-8.md). Locked: [VISION-95.md](VISION-95.md) Q5–Q7, Q19.

**Phase 8 buys:** original v3. Milestone 3 was TLS-**only** latency and **skipped** sock I/O on TLS fds. This phase **reopens Q2 from Phase 3** (explicitly). Dual-plane is now in scope.

**Gate from Phase 7:** M7 green so h2-over-TLS can join too. HTTP/1.1 OpenSSL path is the primary correctness gate.

---

## 0. Inventory

| M3 behavior | M8 target |
|---|---|
| `is_tls_fd` → skip `try_enter_io` | Skip **prefix** copy; still emit timing |
| Latency = TlsIo half exit→exit | Content latency = TlsIo; wire = SockIoTimes pair |
| No handshake probe | `SSL_do_handshake` enter/exit |
| Drop if `SSL_FD` miss | Counter `tls_unmapped`; optional `SSL_get_fd` |

### Out of Phase 8

Go/rustls/GnuTLS, kTLS, decrypting the wire, BIO walkers, `&lt;2%` claim, traces (Phase 9).

---

## 1. Locked decisions

| ID | Choice |
|---|---|
| **P8-Q1** | **Option A:** sock probes on TLS fds emit `EventKind::SockIoTimes = 5` — **no prefix**. Layout ≤ 48 B. Content remains `TlsIo` (kind 4). |
| **P8-Q2** | `EventKind::TlsHandshake = 6` — enter stash ts, exit emit `{fd, latency_ns, ts_ns, ret}`. |
| **P8-Q3** | Join in userspace: **content-primary**. Match preceding sock Write/Read on `(tgid,fd)` in `[t−W, t]` (default W=5 ms, `OBSAGENT_JOIN_MS`). Fallback once to `(t, t+W]` on the response half. If no match, keep TLS-only latency (M3). **Never** parse ciphertext. |
| **P8-Q4** | Correctness uses **content** p50 vs 50 ms delay. Also assert `wire_ns > 0`, `wire_ns ≤ content_ns + 20 ms`, join rate **&gt; 90%** on the demo. |
| **P8-Q5** | Handshake: `SSL_do_handshake` required. Add `SSL_connect`/`SSL_accept` only if Q15-style attach experiment shows the demo doesn’t hit `do_handshake`. |
| **P8-Q6** | Handshake histogram name: `tls_handshake_duration_milliseconds` (Prometheus) / OTLP equivalent. Testdata: new conn, `SSL_OP_NO_TICKET` / disable session reuse. |
| **P8-Q7** | Unmapped SSL*: increment `tls_unmapped`; do not crash. Optional uretprobe `SSL_get_fd` — timebox 0.5 day; if not, document residual. |
| **P8-Q8** | Default TLS HTTP pairing stays **client-only**. `OBSAGENT_TLS_SERVER=1` enables read→write for a dedicated server-only gate. |
| **P8-Q9** | `looks_like_http_fixed` still must **not** run on ciphertext. SockIoTimes has no prefix — no magic needed. |
| **P8-Q10** | Phase 3 Q8 (“skip sock I/O on TLS fds”) is **amended**: skip **SockIo prefixes** only. |

### Open

Answered 2026-08-19 — see §10.

---

## 2. Architecture

```text
TLS fd:
  SSL_write/read  → TlsIo (plaintext prefix)     → parse HTTP/h2
  write/sendmsg   → SockIoTimes (ts, dir, ret)   → wire clock
  SSL_do_handshake → TlsHandshake

userspace:
  content_exchange = correlator(TlsIo | h2)
  DualPlane.note(SockIoTimes)
  DualPlane.join(content, preceding sock, [t−W, t]) → wire latency
```

### ABI

```text
EventKind::SockIoTimes = 5     // ~32–48 B, NO prefix
struct SockIoTimesEvent {
    kind: u8, dir: u8, _pad: u16,
    fd: i32, pid: u32, tgid: u32,
    ret: i64, ts_ns: u64,
}

EventKind::TlsHandshake = 6
struct TlsHandshakeEvent {
    kind: u8, _pad0: [u8; 7],
    pid: u32, tgid: u32,
    fd: i32, _pad1: u32,
    ret: i64, latency_ns: u64, ts_ns: u64,
}
```

Update `decode.rs` demux. Keep `SockIoEvent` 288 B frozen.

### BPF change

Today `try_enter_io` returns early on `is_tls_fd`. Change:

```text
if is_tls_fd(fd) {
    // stash for timing-only emit on exit (no user buf copy)
} else {
    // existing prefix path
}
```

Need `PendingIo` to carry a `timing_only: u8`. Same `PENDING_IO` map (one syscall per tid). Extra `PENDING_IO_TIMES` map was rejected (2026-08-19 architecture review).

---

## 3. Subphases

### 8.0 — Preconditions

**Work:** M7 green. `correctness3` + `correctness7-h2-tls` PASS.

---

### 8.1 — ABI + decode

**Work:** kinds 5 and 6; size tests; `decode.rs`.

**Verify:** `test-common`; kind 5/6 roundtrip; 288 B SockIo unchanged.

---

### 8.2 — BPF timing-only + userspace join

**Work:** `PENDING_IO_TIMES`; emit `SockIoTimes`; drain join module `agent/src/dual_plane.rs`. Metrics: content + wire histograms (or extra attributes on the same series — **lock:** separate histogram `http_client_wire_duration_milliseconds` plus keep content as today’s `http_client_duration_milliseconds`).

**Verify:** unit tests for join window hit/miss. `correctness8-dual`.

---

### 8.3 — Handshake uprobes

**Work:** enter/exit `SSL_do_handshake`; attach next to existing OpenSSL probes; aggregator + OTLP.

**Verify:** handshake p50 &gt; 0 on first request new conn; P8-Q11 recorded.

---

### 8.4 — Unmapped + server flag

**Work:** `tls_unmapped` Array or userspace counter from dropped TLS without fd. `OBSAGENT_TLS_SERVER`. Dedicated server-demo script (optional if timebox).

**Verify:** demo unmapped = 0; metric exists.

---

### 8.5 — Security + overhead + close

**Work:** `docs/security.md` — agent still sees plaintext; now also timestamps ciphertext syscalls (no extra secrets). Overhead row. Tick phase-8.md. `phase-8.tdd.md`.

**Verify:** checklist complete.

---

## 4. Success criteria

1. `correctness8-dual` content p50 in band, join rate &gt; 90%.
2. Handshake histogram non-zero on new TLS conn.
3. Cleartext HTTP still works (TLS skip of prefixes, not of all sock probes on non-TLS).
4. No ciphertext in logs.

---

## 5. Appendix — §10

| ID | Answer | Date | Notes |
|---|---|---|---|
| P8-Q11 | `SSL_do_handshake` | 2026-08-19 | Python stdlib ssl demo: hs=10 on 5 new conns (client+server). No `SSL_connect` attach. |
| P8-Q12 | preceding Write@t_start + Read@t_end (client) | 2026-08-19 | Asymmetric window; tickets after SSL_read do not steal |
