# Handoff — Phase 7 HTTP/2 + gRPC (2026-08-19)

**Paste into a new chat:** *Read `docs/handoff/SESSION-2026-08-19-phase-7.md` and continue from §Next. Do not re-litigate locked Qs. Do not claim VISION-95 resume sentence. Do not commit unless asked. Do not lift Q8 (first iovec, 256 B).*

**Full tour:** [`docs/architecture/PROJECT-DEEP-DIVE.md`](../architecture/PROJECT-DEEP-DIVE.md)  
**North star:** [`docs/phases/VISION-95.md`](../phases/VISION-95.md)  
**Checklist:** [`docs/phases/phase-7.md`](../phases/phase-7.md)  
**Plan + §10:** [`docs/phases/phase-7-implementation-plan.md`](../phases/phase-7-implementation-plan.md)  
**TDD evidence:** [`docs/testing/phase-7.tdd.md`](../testing/phase-7.tdd.md)  
**Prior:** [`SESSION-phase-6.md`](SESSION-phase-6.md) (M6 green; do not redo capture)

**Repo:** `c:\projects\eBPF-Observability-Agent` (Windows checkout; **build/run only on WSL2 Ubuntu**)  
**Session dates:** 2026-08-18 implement Phase 7 → e2e RED → 2026-08-19 core review → fix blockers → e2e GREEN  
**Working tree:** **UNCOMMITTED.** Do not assume `main` has Milestone 7. Do **not** commit unless the user asks.  
**Remote:** dirty local work not pushed.

---

## 0. Environment (will page you if ignored)

| Fact | Detail |
|---|---|
| Distro | **Always** `wsl -d Ubuntu`. Default WSL distro is `docker-desktop` — **wrong**. |
| Root / BPF | `wsl -d Ubuntu -u root`. WSL root has no sudo password. |
| Cargo target | `CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target` (set in `scripts/wsl-run.sh`) |
| Runner | `wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh <cmd>` |
| Release vs tests | Unit tests = debug. Gates = **release** binary. After source edits: `wsl-run.sh build` **and** `cargo build --release -p grpc-slow` before root gates. |
| CRLF | `wsl-run.sh` strips scripts to `/tmp` before bash. Do not run `.sh` from `/mnt/c` without stripping. |
| Ready line | Agent prints `obsagent ready` on **stdout** (flushed) **after** OpenSSL uprobes attach. TLS gates **must** wait for it. `RUST_LOG=warn` hides `info!("Phase 7 agent running…")`. |
| Do not scan `/proc/*/maps` for libssl | Tried in this session; **hangs on WSL**. Attach well-known `/lib` + `/usr/lib` paths + `OBSAGENT_LIBSSL`. Same inode is deduped. |

```bash
# unit
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh test-common
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh test-agent

# release + testdata
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh build
wsl -d Ubuntu -- bash -lc 'source /home/sahil/.cargo/env; export CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target; cd /mnt/c/projects/eBPF-Observability-Agent; cargo build --release -p grpc-slow'

# gates (root)
wsl -d Ubuntu -u root -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh correctness7-grpc
wsl -d Ubuntu -u root -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh correctness7-h2-tls
```

`wsl-run.sh build` is `cargo build --release` (default-members: `agent` + `common` only). **`grpc-probe` / `grpc-slow` are not in default-members.**

---

## 1. What this project is

Zero-instrumentation observability agent in **Rust + Aya (eBPF)**. Reconstruct per-service HTTP/**gRPC** latency from kernel syscalls + OpenSSL uprobes. Later: dual-plane TLS, traces, real-node map, &lt;2% CPU, profiles.

**VISION-95 resume sentence is still false** until Milestone 12. Use Phase 5 honest bullets. Do not say dual-plane, traces, or &lt;2% as shipped.

Phases: **0–6 done** (M6 review-fixed). **7 done this session (M7 ticked).** **8 not started.**

---

## 2. What this session did (chronology)

1. **Implement Phase 7** (locked architecture, not the rejected “reassemble then h2::feed”): demux **before** HTTP/1.1 reassembly; leftover `(tgid, fd, dir)`; correlator `(tgid, fd, stream_id)`; hand-rolled frames + HPACK; sticky BPF `INFLIGHT`; testdata tonic h2c + Python OpenSSL h2.
2. **Unit tests green, e2e RED:** `h2_conns>0` `h2_xchg=0` on gRPC; `tlsio=0` on h2-TLS. Milestone 7 **not** ticked.
3. **Core review** (production-grade, no rewrite): **NEEDS FIX / NOT READY**. Canvas: Cursor canvases `phase-7-core-review.canvas.tsx` (optional UI; chat review was the record).
4. **User: “fix issues.”** Implemented skip-cursor leftover, HPACK/Huffman correctness, detect tightening, fd-lifetime, capture-shaped tests, probe + gate races. **Did not lift Q8.**
5. **E2E GREEN.** ROADMAP Phase 7 boxes ticked. Docs updated.
6. Hung WSL diag (`_diag-h2-tls`, `/proc` maps scan) was killed (exit 9). Dead end; do not revive.

---

## 3. Milestone 7 status — SHIPPED (uncommitted)

| Gate | Result | Date |
|---|---|---|
| `test-common` | 16 passed | 2026-08-18 |
| `test-agent` | **81 passed** | 2026-08-19 |
| `correctness7-grpc` | **PASS** `grpc POST /slow.Slow/Sleep` count=5 **p50=52.43ms** (delay 50, band 40–60) `h2_xchg=5` `sockio=53` | 2026-08-19 |
| `correctness7-h2-tls` | **PASS** `h2 GET /slow` count=5 **p50=51.09ms** `tlsio=50` `h2_xchg=5` | 2026-08-19 |

Band: `max(±10 ms, ±10%)` of injected 50 ms.

Headless TLS signature after fix:

```text
obsagent ready
tlsio=50 http_60s=5 h2_xchg=5
h2 GET /slow count=5 p50=51.09ms
```

`h2_hreq` / `h2_hresp` can be **10** on TLS (client + server both parse; `client_only` keeps **5** exchanges). `h2_conns=0` at snapshot is **expected**: close() + `evict_closed` drops userspace `H2Conn` after the RPC.

---

## 4. Locked decisions (do not silently reverse)

VISION-95 Q1–Q4 + Phase 7 §10. If you must change a Q, write it in VISION-95 **and** phase-7 §10 **before** coding the opposite.

| ID | Lock |
|---|---|
| **Q1 / P7-Q1** | Key = `(tgid, fd, stream_id)`. After h2 detect, **forbid** HTTP/1.1 `observe` on that fd. |
| **Q2 / P7-Q2** | Detect = preface **or** SETTINGS/PING/GOAWAY on stream 0 **or** WINDOW_UPDATE **or** HEADERS/RST/CONTINUATION on a nonzero stream. **All-zero 9 bytes are DATA, not a start.** Amended 2026-08-19 (was “type 0–9, length ≤16 KiB”). |
| **Q3 / P7-Q6** | HPACK: static + Huffman + **32-entry** dynamic + **4096-byte** RFC table (fail closed if size update &gt; 4096). Not spec-complete. |
| **Q4 / P7-Q7** | gRPC method = `:path`. No protobuf. |
| **Q8 I/O** | `readv`/`writev`/`sendmsg`/`recvmsg`: **first iovec only, 256 B.** Do **not** lift without a new architecture lock. Sticky `INFLIGHT` copies later **syscalls**, not later iovecs of the **same** `writev`. |
| **Q9 leftover** | Userspace cap **8 KiB**. Honor it with a **skip-cursor** (do not store DATA). Do **not** “fix” by bumping the cap. |
| **P7-Q5** | Latency = resp HEADERS END_HEADERS `ts_ns` − req HEADERS END_HEADERS `ts_ns`. DATA ignored for latency. |
| **P7-Q8 testdata** | **tonic server** h2c; **client in testdata** (not required to be tonic). h2-TLS = **Python `ssl` → libssl**, not rustls/tonic. |
| **Q19 / TlsIo** | Default **client_only** (Write→Read). `OBSAGENT_TLS_SERVER=1` is Phase 8. |
| **No new `EventKind`** | No BPF `H2_FDS`. Close is visible via `SOCK_META`/`INFLIGHT` maps. |
| **No `h2` crate** as parser of record. |

Architecture (keep):

```text
SockIo / TlsIo prefix
        │
        ├─ already h2-marked OR looks_like_h2? ─► H2Registry::feed  (leftover per dir + skip)
        │                                              │
        │                                              ├─ stream_id N HEADERS :method → pending
        │                                              └─ stream_id N HEADERS :status → H2Exchange
        │
        └─ else HTTP/1.1 Reassembler → Correlator (unchanged)
```

BPF still dumb: `http_magic` includes `PRI `; sticky `INFLIGHT` until close / userspace unmark / 60 s. Do **not** clear INFLIGHT on each stream.

---

## 5. Why e2e was red (root causes — do not “fix” again the wrong way)

### 5.1 gRPC `h2_xchg=0` with a `write_all` probe

Parser was fine (`probe_shaped_write_pairs_status`). Live failure:

1. **Q8:** tonic **client** `writev` puts HEADERS in iov[1+] — never copied. Testdata **must not** use tonic as the client without a Q8 amendment. Current `grpc-probe` uses `write()` (not `writev`).
2. **Even `write_all` of preface+HEADERS (~127 B &lt; 256) failed** because the probe did **one `read(256)`** and returned. Tonic server sends **SETTINGS first**; the 50 ms HEADERS come later. Agent saw request HEADERS, never `:status`. **Fix:** probe **reads frames until response HEADERS** (same as the Python h2-TLS probe). Split writes: (1) preface+SETTINGS → `PRI` marks INFLIGHT; (2) HEADERS+DATA on a second `write()` so sticky INFLIGHT copies them under Q8.

Do **not** treat a stuffed-256-B `write_all` as proof that real gRPC clients work.

### 5.2 h2-TLS `tlsio=0`

Uprobes **do** attach (`classic + 4/4 _ex` on `/lib/x86_64-linux-gnu/libssl.so.3`, same inode as `/usr/lib/...`). CPython 3.14 `_ssl` imports `SSL_set_fd` + `SSL_write_ex` / `SSL_read_ex`.

**Race:** OpenSSL attach takes **~4 s** (ELF symbol resolve × ~10 uprobes). Gates used `sleep 3` then probe. Tracepoints were already on → TCP connect/accept in RingBuf. **Uprobes not attached yet** → no `TlsIo`. Drain later saw TCP, `tlsio=0`.

**Fix:** `println!("obsagent ready")` + flush **after** `attach_openssl_uprobes`. Scripts wait for that line (not `info!`, which `RUST_LOG=warn` hides).

Also applied to `correctness-phase3.sh`, `smoke-milestone3.sh`, `smoke-milestone7.sh`, `correctness-phase7-grpc.sh`.

**Wrong fix (reverted):** scan `/proc/*/maps` for libssl. Hangs on WSL. Diagnostic `_diag-h2-tls.sh` deleted.

---

## 6. Core review → what was actually fixed

Review classification was **NEEDS FIX / NOT READY**. After fixes + green gates, leftover skip-cursor + capture tests + live HEADERS are in. Remaining residuals are **documented**, not blockers for M7.

| Sev | Finding | Fix (not a patch) |
|---|---|---|
| Blocker | Live contract unmet | Probe waits for HEADERS; gates wait `obsagent ready` |
| Blocker | 8 KiB leftover `clear()` on 16 KiB DATA → desync | Skip-cursor in `conn.rs` `push_bytes` / `drain_frames` |
| Blocker | Tests inject assembled frames via `io()` | `feed_slices` 256 B; `skip_16kib_data_then_status_still_pairs`; `capture_slices_preface_then_headers_pair` |
| Major | Q8 first iovec | **Not lifted.** Probe uses `write()` + sticky INFLIGHT. Real tonic **clients** still miss HEADERS in iov[1+] |
| Major | close() vs userspace H2Conn 60 s | `evict_closed` on tick vs `SOCK_META` \|\| `INFLIGHT`; full preface resets HPACK |
| Major | HPACK ignore table-size | Honor updates; RFC size + 32-entry cap; &gt;4096 fail closed + `Decoder::reset` |
| Major | Huffman EOS → `Some` | EOS in string / bad padding / leftover &gt;7 bits → `None` |
| Major | `:status` without pending silent | `H2Stats.unpaired_status`; headless `h2_unpair=` |
| Major | one `header_block` per conn | `DirPipe` per Read/Write |
| Major | `looks_like_frame` all-zeros | P7-Q2 tightened |
| Major | testdata `write_all` workaround | Replaced with two writes + frame drain |
| Minor | `debug!` logs `:path` | Log method/status/latency only |

**Keep as-is:** demux before HTTP/1.1 reassembly; stream_id pairing; two HPACK decoders per direction; PRI in `http_magic`; sticky inflight; TlsIo `client_only`.

---

## 7. Code map (where to look)

| Path | Role |
|---|---|
| `agent/src/h2/mod.rs` | Demux exports |
| `agent/src/h2/frame.rs` | 9-byte header; `looks_like_h2` / `looks_like_http11` |
| `agent/src/h2/hpack.rs` | Static table 61; Huffman RFC 7541 B; dynamic 32 / 4096 B |
| `agent/src/h2/conn.rs` | `H2Registry`, `DirPipe` leftover+skip+decoder+CONTINUATION, pairing |
| `agent/src/main.rs` | `ingest_http_io` demux; `handle_h2_exchange`; tick `evict_stale`+`evict_closed`; `attach_openssl_uprobes`; `obsagent ready` |
| `ebpf/src/main.rs` | `emit_io_kind` 256 B first iovec; `PRI ` via `http_magic`; `unmark_sock_fd` on close |
| `common/src/lib.rs` | `http_magic` includes `PRI `; `SOCK_IO_PREFIX_LEN=256` |
| `testdata/grpc-slow/` | tonic **server** + `grpc-probe` (raw h2c client) |
| `testdata/h2-tls-server.py` / `h2-tls-probe.py` | OpenSSL h2 (ALPN `h2`) |
| `scripts/correctness-phase7-grpc.sh` | M7 gate |
| `scripts/correctness-phase7-h2-tls.sh` | M7 gate |
| `scripts/smoke-milestone7.sh` | smoke |
| `scripts/wsl-run.sh` | `correctness7-grpc`, `correctness7-h2-tls`, `smoke7` |

### Leftover machine (must understand)

`DirPipe { leftover, skip, decoder, header_block }`. Incoming bytes: consume `skip` first (discard). Parse 9-byte header. If frame is DATA/control **or** HEADERS `total > 8 KiB`: do not store payload; set `skip = remaining`. Never `clear()` a live stream because a frame is large.

### Drain path

`DecodedEvent::Io` → `ingest_http_io(..., client_only=false)`.  
`DecodedEvent::TlsIo` → `ingest_http_io(..., client_only=true)` + `tlsio++`.  
If marked h2 and prefix looks like HTTP/1.1 start-line → `drop_conn` (fd reuse).

Tick (1 s): reassembly evict; `H2Registry::evict_stale` (60 s); `evict_closed` if **neither** `SOCK_META` nor `INFLIGHT` has the key; `clear_inflight` for unmarked fds.

---

## 8. Testdata contracts

**grpc-slow server:** tonic, h2c, `/slow.Slow/Sleep`, env `PORT`, sleep in handler.  
**grpc-probe:** two `write_all`s (preface+SETTINGS, then HEADERS+DATA); `read_exact` frames until HEADERS stream 1 `END_HEADERS`. `--port --repeat --delay-ms`.  
**h2-tls:** Python 3 stdlib `ssl` (must `ldd` to libssl). Server `wrap_socket` + ALPN `h2`. Probe `sendall` preface+SETTINGS+HEADERS then read until HEADERS.

A **real tonic client** will still hide HEADERS behind Q8 `writev`. That is an accepted residual until a Q8 amendment.

---

## 9. Docs touched this session (already written)

- `docs/phases/phase-7.md` — checklist + M7 PASS (duplicate Milestone 7 section removed)
- `docs/phases/phase-7-implementation-plan.md` — §10 P7-Q2, P7-Q13 skip-cursor
- `docs/phases/VISION-95.md` — Q2 detect clarification
- `docs/architecture/correlation.md` — skip-cursor, fd reuse, detect
- `docs/testing/phase-7.tdd.md` — 81 tests + both e2e PASS
- `docs/ROADMAP.md` — Phase 7 boxes ticked

---

## 10. Residuals (honest — not M7 blockers)

- Q8 first-iovec 256 B: tonic/hyper `writev` HEADERS in iov[1+] **never seen**
- HPACK not spec-complete (32 entries / 4096 B; ignore most SETTINGS)
- CONTINUATION only if the assembled block fits 8 KiB
- Trailers-only / streaming RPC timelines: out of M7
- `classify_protocol`: path has `.` and `/` → `"grpc"` (heuristic)
- Custom BIO / no `SSL_set_fd`: no `TlsIo` (M3 residual)
- Go `crypto/tls`, rustls: out of 95%
- Dual-plane: **Phase 8**, not shipped
- `h2_hreq` counts both directions on same-host TLS; exchanges are client_only
- Huffman decoder is a linear scan of 257 symbols (fine at demo RPS)

---

## 11. Next (Phase 8)

**Do not start Phase 8 until this handoff is the source of truth and M7 stays green.**

Phase 8: [`docs/phases/phase-8.md`](../phases/phase-8.md) · plan [`phase-8-implementation-plan.md`](../phases/phase-8-implementation-plan.md) · TDD [`../testing/phase-8.tdd.md`](../testing/phase-8.tdd.md)

**Buys:** TLS **content** from `TlsIo`, **wire timing** from syscalls on the same `(tgid,fd)` (`SockIoTimes`, no ciphertext prefix), `SSL_do_handshake` histogram. Join window default ±5 ms. `correctness8-dual` content p50 in band + join rate &gt; 90%.

Locked: VISION Q5, Q6, Q7, Q19. Phase 3 Q8 (skip sock I/O on TLS fds) is **amended** in Phase 8: skip **prefixes** only, keep timing.

HTTP/1.1 dual-plane can be built after M6; **ship M8 after M7** so one gate covers h2-over-TLS too.

Do not claim dual-plane or handshake in README until M8 is green.

---

## 12. What the next chat should NOT do

- Lift Q8 “just for tonic”
- Bump leftover cap to 16 KiB instead of skip-cursor
- Scan `/proc/*/maps` for libssl
- Use rustls or tonic+TLS as h2-TLS testdata
- Sleep 3 s and assume uprobes are attached
- Tick VISION-95 claim lock
- Commit unless the user explicitly asks
- Redo Phase 0–6
- Parse HTTP/2 in BPF or send h2 bytes through the HTTP/1.1 reassembler

---

## 13. Suggested first message for the next chat

> Read `docs/handoff/SESSION-2026-08-19-phase-7.md`. Milestone 7 is green but **uncommitted**. Continue with Phase 8 dual-plane TLS per `docs/phases/phase-8-implementation-plan.md`. WSL `-d Ubuntu` only. Do not lift Q8. Do not commit unless I ask.

If they want a commit first: ask, then commit the Phase 7 tree as one or more commits matching repo style (why, not what). Dirty tree is large (Phase 6 leftovers + Phase 7 + docs + testdata).
