# Investigation: replay seam, 87% overhead, LLM tail capture

Date: 2026-11-03. Repo HEAD `661484b` plus uncommitted changes noted in git status.
Labels: **Source** (read in code), **Doc** (official documentation), **Log** (checked-in artifact), **Derived** (arithmetic/reasoning), **Not run** (proposed).

**Execution limit.** No experiment was executed. `wsl.exe -e bash` failed with `Wsl/Service/HCS_E_CONNECTION_TIMEOUT` (distro `Ubuntu` is Stopped), and the eBPF agent builds and runs only on Linux. Everything below is static code reading, one checked-in log, and one documentation fetch. Section 10 lists the experiments that would settle what I could not.

---

## 1. Executive summary

Three decisions from `NEXT_PHASES.md`, investigated together because they share one answer: **build a replayable userspace pipeline first**.

1. **Replay seam.** The seam is much smaller than I estimated in `NEXT_PHASES.md`. `Correlator`, `Reassembler`, `H2Registry` and `HttpAggregator` already take `now: Instant` and a `SockIoEvent` as parameters and touch no BPF maps. Only two things couple the ingest path to the kernel: `clear_inflight` (a map delete) and `lookup_peer` (a map read). Extract the ~60 lines in `ingest_http_io` (`agent/src/main.rs:1189-1247`) into a function that *returns* "inflight keys to clear" instead of calling the map. Estimated 3–5 h, not 6–8 h. **Recommended.**
2. **The 87%.** I withdraw my earlier claim that the per-event `HashSet` clone at `main.rs:389` is a likely contributor. The set holds only the agent's own tgid/tids (`filter.rs:234-261`), so the clone is tens of integers. Arithmetic from `overhead-run1.log` shows the agent spends **~1.37 M instructions per request** (Derived, §3.2). No single suspect I found in the source comes near that. Do not fix anything until a profile exists. The replay harness from decision 1 is also the cheapest profiler.
3. **LLM tail capture.** Today the exchange latency ends when response **headers** complete, so for a streamed LLM reply it measures time-to-headers, not total time (`reassemble.rs:114-124`, `main.rs:1242`). Token counts sit in the SSE body, which the kernel stops emitting after headers because userspace clears the `INFLIGHT` flag. Anthropic's `message_delta.usage` is cumulative but, in at least one documented example, absent. Recommendation: a **conditional decision**. Run a 2 h feasibility spike (which runtime attaches, h1.1 or h2, is `usage` in the tail) before touching the BPF ABI, then choose between "timing-only" and "TLS tail capture".

**Principal risks.** Replay logs of TLS plaintext contain `x-api-key` / `Authorization` headers inside the 256 B prefix, so the recorder must redact at write time (§8). The replay validates only the userspace half; kernel-side filtering, sampling and `INFLIGHT` form a closed loop that a log cannot reproduce.

---

## 2. Problem definition and current state

**Decisions (verbatim framing):**
- D1: How do we make the pipeline replayable and testable in CI without root/BPF?
- D2: What actually causes ~87% of one core at 500 HTTP + ~200 gRPC rps, and what should be done first?
- D3: How do we obtain total latency and token usage for streamed LLM calls given a 256 B prefix and verifier limits?

**Constraints.** Rust + Aya; Linux ≥5.15 with BTF; ~50–85 h of effort before 2026-11-30; one developer; Windows host with WSL2 as the only Linux (currently not starting).

**Assumptions (material):** A1 the checked-in logs are representative of HEAD (Log says HEAD at claim lock 2026-08-25; later commits changed docs only, per `git log`). A2 `overhead-run1.log` is typical of runs 2–3 (not read). A3 LLM clients reach the agent through OpenSSL `SSL_read/SSL_write` (unverified for Node).

**Current state (Source):**
- Kernel: syscall tracepoints and OpenSSL uprobes emit `SockIoEvent` (288 B, 256 B prefix) into a 256 KiB RingBuf (`common/src/lib.rs:20,46,171-191`). Per event the BPF code zero-fills the prefix, probe-reads it, runs a 256-iteration zero-tail loop, then copies 288 B into the ring (`ebpf/src/main.rs:440-466`).
- Emission gate: an event is emitted only if the prefix looks like HTTP or the fd is `INFLIGHT` (`ebpf/src/main.rs:456-459`). Userspace clears `INFLIGHT` after each flushed message (`agent/src/main.rs:1242`), so body reads after the headers are not emitted.
- Userspace: one task drains the ring (`main.rs:355-493`), takes ~20 `Mutex` locks per event across `identity`, `reassembler`, `h2`, `correlator`, `sock_meta`, `inflight`, aggregators, and exports.

---

## 3. Fundamentals and mechanics

### 3.1 What is deterministic in the userspace pipeline (D1)

| Stage | Pure given inputs? | Evidence |
|---|---|---|
| `decode_event` | yes | `decode.rs:40-101`, `read_unaligned` of fixed-size records |
| `classify_ingest` | needs `comm` string | `filter.rs:121-135` |
| `comm` | **no**: reads `/proc/<tgid>/comm`, 30 s cache | `identity.rs:120-141`, `CACHE_TTL` at `identity.rs:12` |
| `Reassembler::push`, `Correlator::observe*`, `H2Registry::feed` | yes, `now` injected | `reassemble.rs:39`, `correlate.rs:125-131`, `main.rs:1207` |
| `lookup_peer`, `evict_closed` | **no**: BPF map reads | `main.rs:786-806`, `536-545` |
| Trace span timestamps | **no**: `SystemTime::now()` | `trace_export.rs:94-101` |
| Span/trace IDs | yes, hash of `(tgid,fd,stream,ts)` | `trace_export.rs:64-81` |
| HashMap iteration order | **no** (std `RandomState`) | `correlate.rs:212`, `reassemble.rs:91` |

### 3.2 What the checked-in overhead log says (D2)

`docs/handoff/artifacts/logs/overhead-run1.log`, run 1 only:
- 56 339.95 ms task-clock in 60.05 s → 0.9 CPUs. (Log)
- 57 584 407 179 instructions, 42 244 841 804 cycles, IPC 1.4. (Log)
- 21 752 context switches in 60 s = 386/s. (Log)
- 30 000 HTTP requests at 500 rps; gRPC 199.93 rps → ~12 000 calls. (Log, lines 6 and 22)

**Derived:** ~42 000 requests → **57.58 G / 42 000 ≈ 1.37 M instructions per request.** Context switches at 386/s mean the process is not sleeping and waking per event; it is CPU-bound in compute, not wakeup-bound. The log does not print the ring-event rate, so instructions-per-*event* cannot be derived (gap G1).

The cycles line reads 0.7 GHz because WSL2 exposes virtualized counters; treat absolute frequency with suspicion (Source: `perf` output only).

### 3.3 Where streamed-LLM data lives (D3)

Doc (fetched 2026-11-03, https://platform.claude.com/docs/en/build-with-claude/streaming): `message_start` carries `usage.input_tokens` and a small `output_tokens`; `message_delta` events carry `usage` that is **cumulative**; an example `message_delta` without a `usage` field also appears in the page. So: input tokens arrive at the start of the body, output tokens at the end, and neither is guaranteed to be where a fixed 256 B head-of-read would see it.

---

## 4. Architecture and data flow (representative operation)

HTTP/1.1 request, cleartext, as implemented (Source):

1. `sys_enter_write` stashes `(fd, buf_ptr, len)`; `sys_exit_write` calls `emit_io_kind` (`ebpf/src/main.rs:410`). Requires `existing_fd_keep_io(fd)` (the sampling decision).
2. Prefix copied into per-CPU `IO_SCRATCH`, emitted to `EVENTS` only if HTTP magic or already `INFLIGHT`; `mark_inflight` on magic.
3. Userspace `rb.next()` → `decode_event` → comm lookup → `classify_ingest` → `ingest_http_io` (`main.rs:355-472`).
4. `ingest_http_io`: comm filter → h2 detection → `Reassembler::push` (joins until `\r\n\r\n`, cap 8 KiB) → `clear_inflight` map delete → `Correlator::observe` (pairs request/response on `(tgid,fd)`) (`main.rs:1157-1247`).
5. On pairing: `lookup_peer` (cache, else BPF map read), `parse_exchange`, `publish_parsed` → `HttpAggregator`, `ServiceMap`, `ExportHub` (`main.rs:612-784`).

Failure propagation: a full ring → `bump_drop()` (`ebpf/src/main.rs:461-464`) → counter → 1 s tick doubles `SAMPLE_N` (`main.rs:503-519`, `sample.rs:40`).

---

## 5. Source-code findings

### D1 — replay seam

- **Guaranteed by code:** `Correlator`, `Reassembler`, `H2Registry` have no I/O and take `now` as an argument. They are already unit-tested with synthetic events (`correlate.rs` 8 tests, `reassemble.rs` 6, `h2/conn.rs` 11 per grep).
- **The coupling is two functions.** `clear_inflight` (`main.rs:1149-1155`) and `lookup_peer` (`main.rs:786-806`), plus the 1 s eviction block that reads `sock_meta`/`inflight` (`main.rs:532-564`).
- **Closed loop (Derived).** The kernel decides what to emit using `INFLIGHT`, `SOCK_FDS`, `SAMPLE_N`, and tgid deny maps; userspace writes some of these. A recorded ring log is the *output* of that loop. Replaying it exercises only the userspace half and cannot reproduce kernel decisions under different userspace behaviour.
- **Hidden nondeterminism:** `Instant::now()` at the drain site (`main.rs:383`) → replace with a virtual clock derived from `ts_ns`. `SystemTime::now()` in span building → compare metrics, not span timestamps, in golden tests.
- **Latent bug found while reading (Source):** `emit_io_kind` drops the event silently if `bpf_probe_read_user_buf` fails (`ebpf/src/main.rs:442-444`). The read is always a full 256 B regardless of `prefix_len`, so a short buffer ending at an unmapped page boundary can fail with no drop counter. Impact unquantified.

### D2 — per-event work, ranked by static plausibility (none measured)

| # | Item | Location | Static cost assessment |
|---|---|---|---|
| a | `HashSet` clone + extend per event | `main.rs:389-390` | Tens of ids. Negligible. **Withdrawn as a lead.** |
| b | Full `retain` scan of state maps on every event | `correlate.rs:141,211-214`; `reassemble.rs:40,89-99` (also allocates a `Vec`); `trace_export.rs:341,359` | O(live states) per event. Small if live states are tens; grows with concurrent connections. Unmeasured. |
| c | `bpf(MAP_DELETE_ELEM)` syscall per flushed message under a mutex | `main.rs:1242`, `1149-1155` | ~1 syscall per request/response; hundreds per second; small unless contended. |
| d | 1 Hz `/proc` sweep: reads `comm`/`status` for every process | `filter.rs:186-202`, `main.rs:1115-1143` | O(processes) file reads/s. A few ms/s on a k3s node. Not 870 ms/s. |
| e | HPACK/Huffman decode and frame parsing per HEADERS | `h2/hpack.rs`, `h2/conn.rs` | Unmeasured; plausible hot spot for the gRPC share. |
| f | ~20 mutex lock/unlock pairs per event; `SockIoEvent` (288 B) copied by value through several calls | `main.rs:1157-1178`, `decode.rs:91` | Uncontended locks are cheap (tens of ns). Only 386 context switches/s suggests *little* blocking contention. |
| g | Event amplification: every read/write on an `INFLIGHT` fd emits 256 B; h2 frames (SETTINGS, WINDOW_UPDATE, PING) also pass the magic/inflight gate | `ebpf/src/main.rs:456-459` | Could multiply events per request. Unmeasured; needs the ring-event rate (G1). |

None of a–f individually explains 1.37 M instructions per request. (g) is the only item that could scale the per-request cost by a large factor, and it is the one with no data.

### D3 — capture constraints

- `SOCK_IO_PREFIX_LEN = 256`; `io_copy_len` clamps to it (`common/src/lib.rs:46,55-64`).
- The BPF zero-tail loop is 256 iterations per event (`ebpf/src/main.rs:449-453`); enlarging the prefix multiplies this and the ring bytes.
- `PerCpuArray<SockIoEvent>` scratch (`ebpf/src/main.rs:131`) is the existing stack-limit workaround, confirmed by the two induced entries in `docs/verifier-rejection-log.md`.
- HTTP/2 `DATA` frames are not used for latency ("DATA is ignored", `docs/architecture/correlation.md`); no SSE or chunked-encoding parser exists.

---

## 6. Production engineering notes

- Prototype vs production: the agent is a single-process prototype. `Arc<Mutex<…>>` with one drain task is acceptable until profiling shows contention.
- Replay logs are the same sensitivity as raw traffic (§8).
- Recording must not run in the drain hot path synchronously; write via a bounded channel, count drops.

---

## 7. Alternatives compared

### D1 — replay boundary

| Option | Works because | Gives up |
|---|---|---|
| **A. Pure-function seam (recommended).** Extract ingest middle; replay `SockIoEvent` streams through reassembler/correlator/h2/aggregator with a virtual clock. | Components already pure; minimal diff. | Not end-to-end through identity/k8s/export; no kernel behaviour. |
| B. Trait over BPF maps and run all of `main.rs` unchanged. | Full-path replay. | Touches ~10 function signatures that pass `&Mutex<AyaHashMap<…>>`; larger diff, more risk; exporter nondeterminism remains. |
| C. Synthetic generators only (extend unit tests). | Zero refactor. | No real-traffic fidelity; does not answer "does recorded `/slow` reproduce p50?". |

Reject B for now. Keep C as the baseline already present.

### D2 — approach

| Option | Verdict |
|---|---|
| Fix suspects (a–f) from reading | **Reject.** Withdrawn lead (a); others are not shown to matter. |
| Profile production run with `perf record` | Correct, needs Linux + load generators. |
| Profile **replay** of a recorded log | Cheapest; separates "userspace compute" from "syscalls, ring, kernel". Needs D1. |

### D3 — capturing totals and tokens

| Option | Mechanism | Limit |
|---|---|---|
| O0 Timing only | Use existing timing-only `SockIoTimes` events on TLS fds (`common/src/lib.rs:199-212`, `dual_plane.rs`) for the last response read. | No tokens; stream-end inference by heuristic. Unverified that timestamps beyond headers are retained. |
| O1 Larger head prefix | Raise `SOCK_IO_PREFIX_LEN`. | Misses the tail where output usage lives; multiplies ring bytes for all HTTP. |
| **O2 Tail capture on TLS reads for flagged fds** | At `SSL_read` exit copy the last T bytes (T≈512) of each read; keep `INFLIGHT` for fds the userspace flags as streaming. | ABI + BPF change; chunked/h2 framing must be parsed; verifier work. |
| O3 Parse in-kernel | — | Reject: verifier-hostile, no benefit. |

---

## 8. Failure modes

| # | Scenario | Mechanism | Symptom | Mitigation |
|---|---|---|---|---|
| F1 | Replay log leaks credentials | Prefix of a request contains `x-api-key:` / `Authorization:`; TLS plaintext is recorded | Secrets in a committed fixture | Redact header values at record time; reject recording unless `--redact` default on; golden fixtures from the testdata servers only |
| F2 | Tail event dropped under ring pressure | `EVENTS.reserve` fails → `bump_drop` (`ebpf/src/main.rs:461-464`) | Missing output tokens | Emit `usage_unknown`, never `0`; count per-call |
| F3 | `message_delta` without `usage` | Doc example | Tokens null | Tolerant parser; test with doc fixtures |
| F4 | fd reuse inside 60 s | `Correlator` key `(tgid,fd)` with `TIMEOUT` 60 s (`correlate.rs:22`) | Wrong pairing across connections | Replay fault test: close/reopen same fd within window; measure mis-pair rate |
| F5 | Cross-CPU ring ordering | Ring order is reserve order, not `ts_ns` order | Request consumed after response | Reorder-injection test in replay; compare exchange recovery |
| F6 | Chunked / h2 framing in body tail | SSE wrapped in chunk sizes or `DATA` frames | Parser misses `message_delta` | De-chunker; reuse `H2Registry` frame parser for DATA |
| F7 | Silent probe-read failure near page end | `ebpf/src/main.rs:442-444` | Lost events, no counter | Add counter (small BPF change); out of scope until measured |

---

## 9. Recommendation

**D1 — implement option A.**
```rust
// proposal, not existing code
pub struct Ingested { exchanges: Vec<Exchange>, h2: Vec<H2Exchange>, clear_inflight: Vec<(u32,i32)> }
pub fn ingest_pure(ev: &SockIoEvent, now: Instant, from_tls: bool, client_only: bool,
                   r: &mut Reassembler, h2: &mut H2Registry, c: &mut Correlator) -> Ingested
```
`ingest_http_io` becomes: filter, call `ingest_pure`, apply `clear_inflight`, then `handle_exchange` as today. Behaviour must not change. Invariant: same input sequence + same clock → same `Ingested` sequence.

Replay driver (`obsagent replay`, or a test helper): read records → `decode_event` → `ingest_pure` with `now = base + (ts_ns - first_ts)` → `parse_exchange` → `HttpAggregator`. Side inputs (comm, peer) come from a small sidecar in the log, recorded at write time.

Log format: header `{version, kernel, sample_n, drop_count}` then `[u32 len][record bytes]`, plus `Side {key, comm|peer}` records. Redact at write (F1).

**D2 — measure, do not fix.** Order: (1) record a real `/slow` + gRPC run; (2) replay it under `perf stat` / `perf record` to get userspace-only instructions per event; (3) compare to production-run instructions per request. If replay cost per event × events/request explains the 1.37 M figure, the cost is userspace compute and the profile names the function; if not, the remainder is syscalls/ring/kernel, and `bpf_stats` (`kernel.bpf_stats_enabled=1`, `bpftool prog show`) and `strace -c` come next.

**D3 — conditional.** Spike first (§10 step 5). If the target runtime is HTTP/1.1 + SSE over OpenSSL and `usage` appears in the last read, build O2 minimal: tail of `SSL_read` only, T=512, gated by a userspace-set streaming flag. If not, ship O0 (TTFB + wire-last-byte from `SockIoTimes`) and report tokens as unavailable.

**Explicitly excluded:** trait over all BPF maps (option B), in-kernel parsing, larger global prefix, any optimization before a profile.

---

## 10. Validation and roadmap

| Step | Task | Prereq | Acceptance | Reduces |
|---|---|---|---|---|
| 1 | Extract `ingest_pure` | none | `cargo test` unchanged; new test feeds a hand-built event list and asserts exchange list | D1 coupling |
| 2 | Recorder with redaction | 1 | Fixture from `testdata/` `/slow` replays to p50 within the existing 50.7 ms band; no `x-api-key` bytes in fixtures (`grep`) | F1 |
| 3 | Replay determinism test | 2 | 20 replays, identical exchange list and histogram; sorted output | HashMap order, clock |
| 4 | Replay under `perf stat`/`perf record` on real Linux (not WSL2 if possible) | 2 | instructions/event and top-5 functions reported | G1, D2 |
| 5 | LLM feasibility spike (2 h) | WSL up | Report: uprobe attach works for Python `anthropic`/`openai` and Node (or not); h1.1 vs h2 via observed frames; content-type and `usage` position in captured bytes | A3, D3 |
| 6 | Production profile `perf record -g -p` under the pinned load | WSL up + loadgen | Fraction of cycles per function; ring events/s | D2 |
| 7 | Fault injection on replay: drop, reorder, fd-reuse | 3 | Recovery-vs-loss table | F4, F5 |
| 8 | O2 or O0 per step 5 | 5 | ±5% latency vs server-side truth on mock stream | D3 |

Must-have before the deadline: 1–4. Defer: 7–8 if the spike fails or time is short.

---

## 11. Evidence ledger

| Claim | Type | Evidence | Counter / limit | Confidence | Consequence | Gap |
|---|---|---|---|---|---|---|
| Correlator/reassembler/h2 are pure with injected `now` | Source | `correlate.rs:125`, `reassemble.rs:39`, `main.rs:1207` | `comm`, peer, span time are not pure | High | Small seam | none |
| Self-id set is tens of ids | Source | `filter.rs:234-261` | Thread count unmeasured | High | Lead (a) withdrawn | tid count |
| ~1.37 M instructions per request in run 1 | Log + Derived | `overhead-run1.log` lines 6, 22, 57 and perf block | Runs 2–3 unread; WSL2 counters | Medium | Cause is not a micro-cost | run 2–3 |
| Agent is CPU-bound, not wakeup-bound | Log + Derived | 386 cs/s at 0.9 CPUs | Could still hide lock spinning | Medium | Focus on compute | profile |
| Exchange latency ends at response headers | Source | `reassemble.rs:114-124`, `correlate.rs` | HTTP/2 path differs | High | Streamed total ≠ current metric | h2 behaviour |
| Kernel stops emitting body reads after headers | Source | `main.rs:1242` + `ebpf/src/main.rs:456-459` | Dual-plane timing events keep flowing on TLS | Medium | Tail capture needs a new gate | confirm |
| Anthropic usage is cumulative; may be absent in `message_delta` | Doc | streaming page (fetched 2026-11-03) | Page shows both forms | High | Tolerant parser | version drift |
| Client protocol (h1.1 vs h2) for LLM SDKs | — | not verified | — | Low | Decides parser scope | spike |
| Replay logs contain secrets | Derived | request prefix holds headers | — | High | Redaction required | none |

---

## 12. Verdict

**What we must understand:** the userspace pipeline is already almost a pure function; the kernel-userspace loop (`INFLIGHT`, sampling) is not and a log cannot capture it; "latency" here means time-to-response-headers.

**Recommended approach:** extract `ingest_pure`, record with redaction, replay for determinism tests and as a profiling harness; measure before optimizing; make the LLM-capture design contingent on a 2 h spike.

**Why:** the checked-in numbers (1.37 M instr/request) contradict the single hot-path suspect I earlier named, so measurement is the only defensible next step; the pure-function seam is the smallest change that gives CI-runnable tests.

**Biggest trade-offs:** replay covers userspace only; fixtures from real traffic need redaction work; O2 buys tokens at the cost of an ABI and BPF change.

**Biggest risks / unresolved:** G1 (ring events/s unknown), whether the sweep, h2 decode or event amplification dominates the CPU, Node uprobe feasibility, run-to-run variance of the 87%.

**What to avoid:** optimizing `main.rs:389`; a trait layer over every BPF map; enlarging the global prefix; claiming a cause for the 87% before a profile.

**Next steps:** §10 steps 1–3 now (no Linux needed for 1 and most of 3 via synthetic fixtures); start WSL (`wsl --shutdown`, retry) for steps 4–6.

**Correction to `NEXT_PHASES.md`:** its §1 item 3 and Phase C called the `main.rs:389` clone a "likely contributor". This investigation shows that is unsupported; see §5 D2.

---

## References

- Repo: `agent/src/main.rs` (355-493, 612-806, 1088-1264), `agent/src/correlate.rs`, `agent/src/reassemble.rs`, `agent/src/decode.rs`, `agent/src/identity.rs`, `agent/src/filter.rs`, `agent/src/trace_export.rs`, `ebpf/src/main.rs` (410-470), `common/src/lib.rs`, `docs/handoff/artifacts/logs/overhead-run1.log`, `docs/verifier-rejection-log.md`, `docs/architecture/correlation.md`.
- Anthropic, Streaming messages: https://platform.claude.com/docs/en/build-with-claude/streaming (fetched 2026-11-03; the docs.anthropic.com URL 301-redirects here).
- Earlier research with URLs and dates: `NEXT_PHASES.md` §3.
- Not read: `overhead-run2.log`, `overhead-run3.log`, `agent/src/h2/*.rs` internals, `export.rs`, `trace_export.rs` beyond lines 1-120 and the eviction call sites.
