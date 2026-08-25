# Original vision → 95% (Phases 6–12)

**Status:** planned. Phases 0–5 are the demo. This document is the **north star + locked Qs**.
Implement from the per-phase plans (same pattern as Phases 0–5):

| Phase | Checklist | How | TDD evidence |
|---|---|---|---|
| 6 Capture | [phase-6.md](phase-6.md) | [phase-6-implementation-plan.md](phase-6-implementation-plan.md) | [../testing/phase-6.tdd.md](../testing/phase-6.tdd.md) |
| 7 HTTP/2 + gRPC | [phase-7.md](phase-7.md) | [phase-7-implementation-plan.md](phase-7-implementation-plan.md) | [../testing/phase-7.tdd.md](../testing/phase-7.tdd.md) |
| 8 Dual-plane TLS | [phase-8.md](phase-8.md) | [phase-8-implementation-plan.md](phase-8-implementation-plan.md) | [../testing/phase-8.tdd.md](../testing/phase-8.tdd.md) |
| 9 Traces + Grafana | [phase-9.md](phase-9.md) | [phase-9-implementation-plan.md](phase-9-implementation-plan.md) | [../testing/phase-9.tdd.md](../testing/phase-9.tdd.md) |
| 10 Real-node map | [phase-10.md](phase-10.md) | [phase-10-implementation-plan.md](phase-10-implementation-plan.md) | [../testing/phase-10.tdd.md](../testing/phase-10.tdd.md) |
| 11 Overhead | [phase-11.md](phase-11.md) | [phase-11-implementation-plan.md](phase-11-implementation-plan.md) | [../testing/phase-11.tdd.md](../testing/phase-11.tdd.md) |
| 12 Profiles + claim lock | [phase-12.md](phase-12.md) | [phase-12-implementation-plan.md](phase-12-implementation-plan.md) | [../testing/phase-12.tdd.md](../testing/phase-12.tdd.md) |

Claim lock template: [../handoff/SESSION-VISION-95.md](../handoff/SESSION-VISION-95.md).

**Companion:** [ROADMAP.md](../ROADMAP.md) · [correlation.md](../architecture/correlation.md) ·
[tls-interception.md](../architecture/tls-interception.md) ·
[ring-buffer-backpressure.md](../architecture/ring-buffer-backpressure.md)

Do not start Phase 7 until Phase 6 Milestone 6 is green. Do not speak the resume sentence
until **Milestone 12 (claim lock)** is green.

If a phase plan amends a Q, update **this file and the phase §10** in the same handoff.

---

## 0. What “95% of what I had in mind” means

Original picture (the brief, not the later locked cuts):

1. Zero-instrumentation: attach to the kernel / SSL library, no app SDK.
2. Reconstruct **HTTP and gRPC** latency from syscalls + TLS uprobes.
3. **TLS plaintext content** correlated with the **syscall/wire timeline** (dual-plane).
4. **TLS handshake timing**, not only `SSL_read`/`SSL_write` HTTP.
5. **Live service map** across processes/containers on a **real node**.
6. **OpenTelemetry traces** (not only histograms) + Grafana that an outsider can read.
7. **Under 2% CPU** under a **documented, `perf`-measured** load — or the number is not claimed.
8. Low-overhead **sampling** when the RingBuf cannot keep up.
9. Stretch that was in the original mind: HTTP/2 frames, gRPC `:path`, **CPU stacks on the same timeline**.

### 95% unlocks this sentence (and no other)

> Built a zero-instrumentation observability agent in Rust using eBPF (Aya) that reconstructs
> per-service HTTP/gRPC latency and a live service map from kernel syscall and OpenSSL uprobe
> data, exports OTLP traces and metrics, and stays under 2% CPU on the documented load.

Until Milestone 12, that sentence is **still false**. Use the Phase 5 honest bullets.

### The remaining 5% (explicitly never in this plan)

These were adjacent to the pitch, not required to make the original sentence true:

| Left in the 5% | Why not 95% |
|---|---|
| Go `crypto/tls`, rustls, GnuTLS, BoringSSL-as-primary | Original v3 specified **OpenSSL hooks** |
| XDP / packet datapath / Hubble | Original product is syscall + uprobe, not CNI |
| `io_uring`, kTLS, Windows | Different attach physics |
| Full HPACK dynamic-table / CONTINUATION / push | Enough `:path`/`:status` for gRPC latency |
| Perfect HTTP/1.1 pipelining | Documented residual mis-pair |
| Multi-cluster, graph DB, SaaS backend | Not a vendor |
| In-kernel HTTP parse / in-kernel redaction | Verifier + CPU suicide |
| Always-complete traces (zero drops) | RingBuf physics; drops stay a metric |

If a subphase starts smelling like one of those rows, it is out of scope.

---

## 1. Distance today → after this plan

| Original piece | After Phase 5 | After Phase 12 |
|---|---|---|
| HTTP/1.1 reconstruction | Demo (no `writev`, prefix-only) | Real-server attach set + multi-chunk |
| gRPC / HTTP/2 | 0% | Stream-id FSM + `:path` latency |
| Dual-plane TLS | Explicitly skipped | Content from uprobes, timing from syscalls |
| Handshake timing | 0% | `SSL_do_handshake` attribute on first span |
| Service map | kind-in-Docker `proc:unknown` | Real-node cgroup → pod |
| OTLP traces | Metrics only | Sampled spans from exchanges |
| Grafana | JSON file, noisy scrape | Allow-listed dashboards on real scrape |
| &lt;2% CPU | Unproven (`ps` 10.7% burst) | `perf stat` gate on pinned load |
| Sampling | Drop-only | In-kernel probabilistic + drop counter |
| CPU profiles on timeline | 0% | 99 Hz + join to slow spans |
| Verifier diary | Empty | ≥2 real pastes |

**Weighted: ~50% → ~95% of the original picture.**

---

## 2. Locked decisions (do not silently reverse)

Same rule as Phases 3–5: if you need to change a Q, write it in the phase handoff **before**
coding the opposite.

| ID | Choice |
|---|---|
| **Q1 HTTP/2 key** | `(tgid, fd, stream_id)`. Fd-only pairing is **forbidden** once a connection is HTTP/2. |
| **Q2 HTTP/2 detect** | Connection preface `PRI * HTTP/2.0` **or** TLS ALPN `h2` (if visible) **or** first frame looks like HTTP/2 (**not** all-zero DATA: SETTINGS/PING/GOAWAY on stream 0, WINDOW_UPDATE, or HEADERS/RST/CONTINUATION on a nonzero stream). Mark the fd; do not run the HTTP/1.1 FSM on it. |
| **Q3 HPACK** | Userspace, hand-rolled: **static table + Huffman + bounded dynamic table (max 32 entries)** for `:method`, `:path`, `:status`, `:authority`. Not a spec-complete compressor. Unbounded dynamic table and CONTINUATION remain residuals. |
| **Q4 gRPC** | Method = HTTP/2 `:path` (`/package.Service/Method`). Optional 5-byte DATA length prefix for message count. No protobuf field decode. |
| **Q5 Dual-plane** | **Content plane** = `TlsIo` prefixes (parse HTTP/gRPC). **Timing plane** = sock I/O enter/exit `ts_ns` on the same `(tgid,fd)` when present. Join with a ±N ms window (default 5 ms). If no sock event, keep TLS-only latency (today’s M3 behavior). Never merge two HTTP parses. |
| **Q6 Handshake** | Uprobe `SSL_do_handshake` (+ `SSL_connect` / `SSL_accept` if needed). Emit `EventKind::TlsHandshake = 6`. Do **not** parse `ClientHello` in BPF. Dual-plane timing is `EventKind::SockIoTimes = 5` (Phase 8). |
| **Q7 TLS libraries** | OpenSSL 1.1 + 3.x only for 95%. Document Go/rustls as 5%. |
| **Q8 I/O attach** | Add `readv` / `writev` / `sendmsg` / `recvmsg`. Copy **first iovec** only, still 256 B cap. |
| **Q9 Reassembly** | Userspace per-`(tgid,fd)` (and stream_id for h2) byte buffer, **cap 8 KiB**, timeout 60 s. BPF still copies prefixes only. |
| **Q10 Traces** | Sampled OTLP/HTTP JSON **spans** from completed exchanges. Span name = `METHOD route`. Attributes: src, dst, status, protocol (`http/1.1`\|`h2`\|`grpc`). No W3C injection into target apps. Head sampling default 1/10; always-on only in correctness gates. |
| **Q11 Metrics** | Keep Phase 5 cumulative histograms. Add `rpc.grpc` route label when `:path` is gRPC. |
| **Q12 Identity gate** | Milestone 10 **requires a real Linux VM or cloud node** (or kind with rootful node PID namespace that matches BPF tgid). Kind-in-Docker-on-WSL is **not** the identity proof. |
| **Q13 Allow-list** | k8s mode: default **exclude** comm/cgroup prefixes for `dockerd`, `containerd`, `kubelet`, `kube-proxy` health, Docker API. Includelist optional (`OBSAGENT_COMM_ALLOW`). Cardinality is a product requirement, not a Grafana crop. |
| **Q14 Overhead load** | Pin in `benches/vision95-load.md`: **500 RPS HTTP/1.1 + 200 RPS h2/gRPC** against the demo pair, 60 s, 3 runs. Measure agent with **`perf stat -p <pid>`** (not `ps`). Headline: agent CPU **&lt; 2% of one host core** (mean of 3). If miss: sampling/allow-list/prefix — do not lower the load to fake it. |
| **Q15 Sampling** | Primary still drop-on-`reserve` + counter. **Add** in-kernel 1/N of **connections** (`SOCK_META` sticky `KEEP_IO`), covering `SockIo` / `TlsIo` / `SockIoTimes`. Draw once per `(tgid,fd)` at connect/accept (or first I/O of an **already marked** fd). I/O on an unmarked fd must not insert `SOCK_META`. Independent `% N` per event is rejected (breaks pairing and HPACK). Latency (`connect`/`accept`/`handshake`) always emit if the tgid is captured (`DENIED_TGID` deny-list, or `ALLOWED_TGID` when `OBSAGENT_COMM_ALLOW` is set). Auto N ∈ `{1,2,4,8,16}`; `OBSAGENT_SAMPLE_N` pins BPF N (spans stay `OBSAGENT_TRACE_SAMPLE`). Sample skip does not increment `DROPS`. |
| **Q16 Profiles** | `perf_event` stack samples ~**99 Hz**, userspace `blazesym`, join to **in-flight or just-completed** spans on same tgid whose time window contains the sample. Off by default. **Off during the Phase 11 2% run.** On for Milestone 12 and when `OBSAGENT_PROFILE=1`. |
| **Q17 IPv6** | Parse `sockaddr_in6` on connect/accept so maps are not IPv4-only. No Happy-Eyeballs product work. |
| **Q18 Verifier** | Every real rejection during 6–12 is pasted into `docs/verifier-rejection-log.md` with program name + fix. M12 requires **≥ 2 real entries** (not “classes”). If none occur, **force one**: temporarily put a 600 B struct on the BPF stack in a branch, capture the reject, then keep the PerCpuArray fix — only if a natural reject did not already land. Prefer natural. |
| **Q19 Server-side TLS** | Dual-plane + HTTP/2 must support **server** OpenSSL (`SSL_read` then `SSL_write`) without double-counting same-host loops. Attribution: if both ends are local, count **client** span only (keep M3 rule) **unless** `OBSAGENT_TLS_SERVER=1` for a dedicated server-demo gate. |
| **Q20 Export** | Drain still never `.await`s collector. Spans go to the same async exporter. Drop + `otlp_dropped` on full/4xx. |

---

## 3. Dependency graph

```text
Phase 6  Capture completeness (writev, reassembly, allow-list, IPv6)
   │
   ▼
Phase 7  HTTP/2 frames + gRPC :path          ← needs writev; needs reassembly buffer
   │
   ├──────────────► Phase 8  Dual-plane TLS + handshake
   │                         (h2 over OpenSSL uses 7+8 together)
   ▼
Phase 9  OTLP traces + Grafana (allow-listed)
   │
   ▼
Phase 10 Real-node identity + service map     ← can prep VM in parallel from 6
   │
   ▼
Phase 11 Overhead + sampling  (<2% gate)      ← after probes exist
   │
   ▼
Phase 12 CPU profiles on the same timeline
   │
   ▼
Milestone 12  Claim lock
```

Phase 10 VM bring-up may start during Phase 6. The **identity gate** still waits until
the agent on that node sees named src+dst.

---

## 4. Phase 6 — Capture completeness

**Execute:** [phase-6-implementation-plan.md](phase-6-implementation-plan.md) (subphases 6.0–6.8).

**Buys:** HTTP/1.1 (and later h2 DATA) from real servers, not only `read`/`write`/`sendto`.
Without this, Phase 7 is a toy parser on incomplete bytes.

**Why original:** “reconstruct from raw syscall and packet data”; nginx/Axum often `writev`/`sendmsg`.

### 6.1 Attach `readv` / `writev` / `recvmsg` / `sendmsg`

- Enter: stash fd + iov base (first iovec only) + dir.
- Exit: same `emit_io` path, 256 B cap, HTTP-magic / later h2 gate.
- Testdata: a server that **only** responds via `writev` (or curl/nginx). Correctness must see the response half.

**Gate:** `correctness6-writev` — `/slow` p50 still in band when the server is writev-backed.

### 6.2 Userspace reassembly buffer

- Per `(tgid, fd)` append until `httparse` Complete **or** 8 KiB **or** 60 s.
- Still no BPF parse.
- Mid-stream chunks that failed in-kernel HTTP magic: **optional** pass-through if fd already has pending request (so split headers work). Requires BPF to emit non-magic bytes **once the fd is in-flight** — add an `INFLIGHT` map bit set from userspace via a userspace→kernel map, **or** (simpler for 6.2) emit all prefixes on marked socks and filter in userspace. Prefer **userspace filter** if CPU allows; measure in 6.4 notes, finalize in Phase 11.

**Gate:** split-header fixture (request in two `write`s) produces one exchange.

### 6.3 Process allow / deny list

- Env: `OBSAGENT_COMM_DENY` (comma) + k8s defaults (Q13).
- Apply in userspace first (fast to ship). Promote hot denies to BPF `COMM`/`TGID` map in Phase 11 if overhead needs it.

**Gate:** headless run on a noisy host: Docker API series **not** in top-10 HTTP routes.

### 6.4 IPv6 sockaddr

- `read_sockaddr` v4 or v6; `SockMeta` grows (or parallel `SockMetaV6`). Keep RingBuf events small — peer still in map, not on every I/O event.

**Gate:** unit test + one connect to `::1` appears in service map as dst.

### Milestone 6

- [ ] 6.1–6.4 code + tests
- [ ] `writev` correctness gate
- [ ] Split-header reassembly gate
- [ ] Deny-list evidence in handoff
- [ ] Overhead **row** recorded (not the &lt;2% claim yet)

**Files (expected):** `ebpf/src/main.rs`, `common/` (`SockMeta`, maybe `INFLIGHT`), `agent/correlate.rs`, `agent/main.rs`, `scripts/correctness-phase6.sh`, `docs/overhead.md`.

**Non-goals:** HTTP/2, dual-plane, traces.

---

## 5. Phase 7 — HTTP/2 + gRPC

**Execute:** [phase-7-implementation-plan.md](phase-7-implementation-plan.md) (subphases 7.0–7.7).

**Buys:** the word **gRPC** in the resume sentence. This is the largest technical gap.

**Why original:** “HTTP latency, gRPC call graphs”; stretch S1.

### 7.1 Frame parser (userspace)

- 9-byte header: length, type, flags, stream id.
- Types needed: HEADERS, DATA, SETTINGS (ignore), WINDOW_UPDATE (ignore), PING (ignore), RST_STREAM (fail the stream), GOAWAY (evict conn).
- Hand-roll. No `h2` crate as the parser of record (interview clarity). Test vectors from RFC 7540 examples + grpcurl.

**Gate:** unit tests for HEADERS+DATA on stream 1 and 3 (two concurrent streams, no mis-pair).

### 7.2 Connection + stream correlator

- New type or mode in `correlate.rs`: `H2Conn` keyed `(tgid, fd)` → map `stream_id → pending headers/data`.
- Client: HEADERS (req) then HEADERS (resp) / DATA; latency = resp HEADERS `ts_ns` − req HEADERS `ts_ns` (END_HEADERS). Document DATA-only trailing as out of latency definition.
- HTTP/1.1 FSM must **not** run on h2 fds (Q2).

**Gate:** two parallel streams, two latencies, no cross-pair (the interview whiteboard).

### 7.3 HPACK enough for `:path` / `:method` / `:status`

- Static table + Huffman decode (Q3).
- If `:path` missing → drop series (don’t guess).

**Gate:** parse `:path=/hello.Greeter/SayHello` from a recorded frame blob.

### 7.4 gRPC testdata + correctness

- Small tonic or grpc-go **cleartext h2c** server (`/slow` with 50 ms delay) + client.
- Histogram / span labeled `grpc` + method path.
- p50 vs 50 ms same band as HTTP/1.1 (`max(±10 ms, ±10%)`).

**Gate:** `correctness7-grpc` PASS.

### 7.5 HTTP/2 over OpenSSL (h2)

- Same parser on `TlsIo` prefixes + reassembly buffer.
- ALPN not required if preface/frames visible in plaintext uprobes.

**Gate:** `correctness7-h2-tls` — one unary RPC over HTTPS/h2, p50 in band.

### Milestone 7

- [ ] 7.1–7.5
- [ ] Document residual: CONTINUATION, trailers-only, full HPACK dynamic table
- [ ] TUI / OTLP series for `grpc` methods
- [ ] **Never** say “call graph” unless service map edges exist (Phase 10); here it is **per-method latency**

**Non-goals:** streaming RPC timelines, protobuf decode, grpc-web.

---

## 6. Phase 8 — TLS as originally specified

**Execute:** [phase-8-implementation-plan.md](phase-8-implementation-plan.md) (subphases 8.0–8.5). `EventKind::SockIoTimes = 5`, `TlsHandshake = 6`.

**Buys:** original v3 — “correlate with the plaintext syscall timeline” + handshake timing.

**Why original:** TLS was the hard part; M3 shipped content-only.

### 8.1 Dual-plane join

- Stop skipping sock I/O **timing** on TLS fds. Options (pick one in 8.1, lock in handoff):

  **A (preferred):** sock probes on TLS fds emit a **tiny** `SockIoTimes` event (no prefix) — `ts_ns`, fd, dir, ret. Content stays `TlsIo`.

  **B:** keep prefixes off (ciphertext) but still emit timing-only records.

- Userspace: parse from `TlsIo`; set span `tls.content_latency_ns` (today) and `tls.wire_latency_ns` (syscall pair) when join succeeds.
- Correctness: `/slow` 50 ms — **content** p50 in band. Wire latency may be smaller; assert `wire ≤ content + slack` and both non-zero.

**Gate:** `correctness8-dual` prints both clocks; content p50 in band; join rate &gt; 90% on the OpenSSL demo.

### 8.2 Handshake uprobes

- `SSL_do_handshake` enter/exit → `TlsHandshake` event (kind=6).
- CLI + OTLP histogram `tls.handshake.duration_milliseconds`.
- Testdata: first request on a new conn (disable session reuse).

**Gate:** handshake p50 &gt; 0 and less than HTTP `/slow` p50 on local OpenSSL.

### 8.3 Custom BIO / missing `SSL_set_fd`

- If fd unknown: try `SSL_get_fd` uretprobe **or** drop + counter `tls_unmapped`.
- Document custom BIO as residual 5%-adjacent; don’t build a BIO walker.

**Gate:** `tls_unmapped` metric exists; OpenSSL demo stays 0.

### 8.4 Server-side OpenSSL (Q19)

- `observe` (read→write) for TLS when `OBSAGENT_TLS_SERVER=1`.
- Default same-host still client-only to avoid 2×.

**Gate:** dedicated server-only demo process; unary HTTPS visible without a local client probe.

### Milestone 8

- [ ] Dual-plane events + join
- [ ] Handshake histogram
- [ ] Security note updated (still privileged plaintext; now also syscall timing on TLS fds)
- [ ] Overhead row (expect CPU up; do not claim &lt;2% here)

**Non-goals:** rustls/Go, decrypting the wire, kTLS.

---

## 7. Phase 9 — Traces + Grafana you can show

**Execute:** [phase-9-implementation-plan.md](phase-9-implementation-plan.md) (subphases 9.0–9.5).

**Buys:** original “reconstruct application-level traces” + Phase 4 Grafana as a real artifact.

### 9.1 Span model

- One span per completed HTTP/1.1 exchange or h2/gRPC stream.
- Trace id: splitmix64`(tgid, fd, stream_id, t_start_ns)` (stable for that exchange, not W3C from the app).
- Handshake: one-shot attribute `obsagent.tls.handshake_ns` on the first sampled span for that fd if handshake `ts_ns ≤ t_start_ns`. **Not** a child span.
- Export OTLP/HTTP JSON `/v1/traces` (same collector). Unit test: valid JSON, `status.code`, duration.

**Gate:** collector accepts traces (2xx); one span in debug exporter / Tempo/Jaeger **or** collector logging exporter in kind/VM.

### 9.2 Head sampling

- Default 1/10 (Q10). Correctness sets `OBSAGENT_TRACE_SAMPLE=1`.
- Metric `obsagent_traces_sampled_total` / `obsagent_traces_not_sampled_total`.

### 9.3 Allow-listed Grafana

- Dashboard: HTTP p50/p99, gRPC p50/p99, handshake, `events_dropped`, `otlp_dropped`, service-map table.
- Provision in `deploy/grafana/` **and** run against a scrape that uses Q13 filters.
- Screenshot rule: no Docker `/v1.54` as the hero series.

**Gate:** `deploy/grafana/dashboards/obsagent.json` matches **actual** metric names from Phase 5+7+8; handoff screenshot of HTTP + gRPC panels.

### 9.4 Service-map panel

- Prometheus metrics already have src/dst. Add a Grafana table. Full identity is Phase 10.

**Gate:** table shows demo `frontend → api` **or** `ip:port` pairs without kube-proxy dominance.

### Milestone 9

- [x] Traces 2xx + one visible span (`smoke9` sink + `trace_s>0`)
- [ ] Grafana live (not only JSON-in-git) — dashboard stems gated in unit/`smoke9`; screenshot is handoff
- [x] Sampling counters (`obsagent.traces.*` / headless `trace_s` `trace_ns` `trace_drop`)

**Non-goals:** Tempo HA, tail-based sampling, trace-context propagation into apps.

---

## 8. Phase 10 — Real-node service map

**Execute:** [phase-10-implementation-plan.md](phase-10-implementation-plan.md) (subphases 10.0–10.5).

**Buys:** “live service map across multiple processes/containers.” Kind-in-Docker cannot prove this.

### 10.1 Topology

- One **real** Linux VM or cloud instance (Ubuntu 22.04/24.04, BTF, kernel 5.15+).
- Kubernetes: k3s or kind **without** the WSL nested-PID lie (k3s on the VM is the default).
- Document kernel version, BTF yes, cgroup v2.

**Gate:** BPF `tgid` exists in **the same** `/proc` the agent mounts.

### 10.2 Identity

- `identity.rs` cgroup → pod uid → kube index name.
- Src **and** dst named for the demo (`frontend` → `api`).
- If ClusterIP hides pod IP: index the **Service** object (ClusterIP → Service `ns/name`). Do **not** guess a replica from EndpointSlice — `connect()` recorded the VIP, not the backend pod.

**Gate:** scrape/labels `src=demo/frontend-…` and `dst=demo/api-…` (or equivalent). `proc:unknown` **not** the demo series.

### 10.3 DaemonSet e2e

- Same caps as today. Smoke: `KIND_E2E` analogue `VM_E2E=1` or `K3S_E2E=1`.
- Multi-service demo: HTTP/1.1 + gRPC + OpenSSL HTTPS.

**Gate:** `K3S_E2E=1` (or named script) green; handoff with `kubectl` + scrape excerpt.

### 10.4 Map cardinality

- Deny-list on. Edge cap 4096 still. Overflow `_other` scraped.

**Gate:** edge count for demo namespace ≥ 1 named edge; host noise not &gt; 50% of series.

### Milestone 10

- [ ] Named src+dst on a real node
- [ ] Handoff: topology diagram (PID ns, /proc, cgroup)
- [ ] Update interview pack: **kind PID lie is historical**, real-node is the proof

**Non-goals:** multi-node mesh, Cilium identity, Istio.

---

## 9. Phase 11 — Overhead, sampling, the 2% number

**Execute:** [phase-11-implementation-plan.md](phase-11-implementation-plan.md) (subphases 11.0–11.6). Load pin: [../../benches/vision95-load.md](../../benches/vision95-load.md) *(file exists; scripts land in 11.1)*.

**Buys:** the last word in the original resume sentence. Do this **after** 7–10 probes exist.

### 11.1 Load protocol

- Write `benches/vision95-load.md` + script: 500 RPS HTTP/1.1 + 200 RPS h2/gRPC, 60 s, 3 runs (Q14).
- Baseline: load **without** agent. Then with agent.
- Record: `perf stat` CPU of agent, RSS, `events_dropped`, sample rate.

### 11.2 In-kernel sampling (Q15)

- Sticky `KEEP_IO` per `(tgid,fd)`. Gate I/O **before** prefix copy. `SockIoTimes` uses the same bit.
- Connect/accept/handshake never sampled away (deny-list can still skip the whole tgid).
- Userspace drop-counters unchanged (`reserve` fail only).

**Gate:** under **overload** (e.g. 5k RPS), `events_dropped` rises **or** sample_n increases; agent stays bounded; histograms still move.

### 11.3 BPF capture maps (`DENIED_TGID` / `ALLOWED_TGID`)

- Required this phase (hostPID). Userspace fills thread-group leaders from `/proc`. Deny-list: self tgid + deny comms. Allow-only (`OBSAGENT_COMM_ALLOW`): `ALLOW_ONLY` + `ALLOWED_TGID` — do not insert the rest of the host into `DENIED_TGID`.

### 11.4 The number

- If mean agent CPU **&lt; 2% of one core** on Q14 load: **claim unlocked** (with date, kernel, load line).
- If not: iterate 11.2–11.3, prefix 128 B trial, disable profiles (profiles are Phase 12). **Do not** claim &lt;2% on Phase 1 connect-only load.
- If still not: **fail the claim** and change the sentence to the measured number. 95% of the *vision* can survive as “measured X% on the documented load” — but then Milestone 12 uses that number, not 2%.

**Gate:** `docs/overhead.md` Vision-95 row with `perf stat`, three runs, load script SHA/path.

### Milestone 11

- [ ] Protocol + three runs
- [ ] Sampling behavior demonstrated
- [ ] Either &lt;2% **or** explicit measured replacement (blocks the original 2% wording)

---

## 10. Phase 12 — CPU profiles on the same timeline

**Execute:** [phase-12-implementation-plan.md](phase-12-implementation-plan.md) (subphases 12.0–12.5).

**Buys:** original stretch — “this endpoint is slow” → “this function is hot” without redeploy.

### 12.1 `perf_event` sampling

- ~99 Hz, user+kernel stacks if cheap enough; user stacks minimum.
- Off by default (`OBSAGENT_PROFILE=1`).

### 12.2 Symbolize

- `blazesym` (or equivalent) in userspace. No BPF symbolizer.

### 12.3 Join to spans

- For spans with latency &gt; threshold (e.g. p95 or `≥ 20 ms`): attach top-N frames whose sample `ts` ∈ `[t_start, t_end]` and `tgid` matches.
- Export as span event or span attribute `profile.top_frame` (keep OTLP small — top 5 frames).

**Gate:** `/slow` (50 ms sleep in a named function) shows that function in the joined frames **most of the time** (document hit rate; 50%+ is enough for a demo, not a profiler product).

### Milestone 12 (claim lock) — also Phase 12 exit

Phase 12 code + **claim lock checklist** (next section).

**Non-goals:** Parca compatibility, continuous fleet profiling SaaS, Java JIT maps.

---

## 11. Milestone 12 — Claim lock (95% gate)

All of the following must be true in one handoff doc (`docs/handoff/SESSION-VISION-95.md`):

| # | Evidence | Unlocks |
|---|---|---|
| 1 | `correctness6-writev` + split-header | “reconstruct from syscalls” on real I/O |
| 2 | `correctness7-grpc` + `correctness7-h2-tls` | **gRPC** / HTTP/2 |
| 3 | `correctness8-dual` + handshake histogram | original **v3 dual-plane** + handshake |
| 4 | OTLP traces 2xx + Grafana screenshot (allow-listed) | **traces**, not only metrics |
| 5 | Real-node named `frontend → api` | **live service map** |
| 6 | `perf stat` Vision-95 row | **&lt;2%** or the honest measured number |
| 7 | Sampling under overload (drop or 1/N) | original “sampling and aggregation” |
| 8 | `/slow` stack join (profile on) | original stretch S2 |
| 9 | `verifier-rejection-log.md` ≥ 2 real pastes | original “verifier” hardness |
| 10 | README + resume bullets rewritten to the **proven** sentence | stop lying |

**After this gate you may claim 95%.** You may not claim Pixie, all TLS libraries, zero drops, or multi-cluster.

---

## 12. Claim fragments vs phase (use on résumé only when green)

| Fragment | First legal after |
|---|---|
| Zero-instrumentation HTTP/1.1 (OpenSSL HTTPS) | already (M3) — keep |
| `writev`/`sendmsg` completeness | M6 |
| HTTP/2 + gRPC unary latency | M7 |
| Dual-plane TLS + handshake | M8 |
| OTLP traces + Grafana | M9 |
| Live pod-named service map | M10 |
| Under 2% CPU (documented load) | M11 **only if the number hit** |
| Endpoint → hot function | M12 |
| Full original sentence | Claim lock |

---

## 13. Suggested calendar (solo, focused)

Not a contract. Use to sequence, not to pad a résumé with dates.

| Phase | Focused weeks | Part-time (evenings) |
|---|---|---|
| 6 Capture | 1 | 2 |
| 7 HTTP/2 + gRPC | 2–3 | 4–6 |
| 8 Dual-plane + handshake | 1–1.5 | 2–3 |
| 9 Traces + Grafana | 1 | 2 |
| 10 Real node + identity | 1–2 (infra variance) | 2–4 |
| 11 Overhead + sampling | 1 | 2 |
| 12 Profiles + claim lock | 1–1.5 | 2–3 |
| **Total** | **~8–12 weeks** | **~16–22 weeks** |

Phase 7 is the long pole. Phase 10 is the infra pole. Do not parallelize 7 with “more docs.”

---

## 14. Engineering rules (same as 0–5)

1. Lock Qs in the phase handoff if you deviate.
2. TDD gate per subphase: unit first, then smoke, then correctness with injected delay.
3. BPF stays dumb: no HTTP/2 parse in-kernel, no HPACK in-kernel.
4. Drain never blocks on export.
5. Never log TLS prefixes. Redact before traces (headers already; h2 `:authorization` if present).
6. WSL remains OK for **compile + HTTP/1.1 unit tests**. Identity and &lt;2% **do not** use WSL kind-in-Docker as proof.
7. Interview pack updates **after** the gate, not before — don’t script claims that aren’t green.

---

## 15. First action (Phase 6.1)

1. Add tracepoints for `sys_enter/exit_writev` and `sys_enter/exit_sendmsg` (then readv/recvmsg).
2. First-iovec prefix copy; reuse `emit_io`.
3. Testdata server that answers only with `writev`.
4. `correctness6-writev` cloned from `correctness3`.
5. One overhead row.

Stop there. Do not open HTTP/2 until Milestone 6 is ticked on [ROADMAP.md](../ROADMAP.md).
