# eBPF Observability Agent — spoken script and Q&A

**Use when:** “walk me through the eBPF project,” screens, hiring-manager deep dive, mock loops.  
**Companion:** [CONCEPTS_GRILL.md](CONCEPTS_GRILL.md) · [RESUME_AND_STORY.md](RESUME_AND_STORY.md) · [RUST_GRILL.md](RUST_GRILL.md)

**How to use:** memorize §1 and §2 until you can say them while drawing. Rehearse §3 once with a timer. §4+ is **open-book prep**, not a speech. If you recite Q&A like a FAQ page, you will sound coached — answer in your own words using the **facts**.

**Drill plan (5 days):**

| Day | Speak | Lookup |
| --- | --- | --- |
| 1 | §1 default 30s × 10 | glossary in CONCEPTS |
| 2 | §2 2–3 min × 5 with a box diagram | correlation.md |
| 3 | §2b hiring manager | OTLP buckets |
| 4 | Mock round A (§13) | kind PID |
| 5 | Mock round B + “what next” | never-claim list |
| +Rust JD | SCRIPT §1c + RUST_GRILL §16–§19 | `main.rs` Arc/Mutex, `decode.rs` unsafe |

---

## 1. 30-second script (default)

I built a **Rust + Aya** observability agent so you get HTTP latency **without an SDK** in the app. Kernel tracepoints see `connect`/`accept`/`read`/`write`. For HTTPS I attach **OpenSSL uprobes** so I see plaintext at the library, not ciphertext on the wire.

Userspace correlates halves on **`(process, file descriptor)`**, parses HTTP with `httparse`, redacts auth cookies, and exports **cumulative OTLP histograms**. In kind, Prometheus scraped `GET /` with count 99 and **zero** RingBuf/OTLP drops after I fixed a JSON 400. I do **not** claim HTTP/2, rustls, or production pod identity on kind-in-Docker.

---

## 1b. 30-second (SRE / platform JD)

Node-local DaemonSet: BPF + PERFMON + ptrace, BTF and host proc mounted. Reconstruct HTTP/1.1 client duration, service-map edges, self-metrics `events_dropped` / `otlp_dropped`. Collector `:8889`. Privileged agent is the trust boundary — TLS intercept is explicit, metrics-only UI.

---

## 1c. 30-second (Rust / systems JD)

Three crates: `no_std` `common` ABI, `bpfel` programs, Tokio agent. Verifier-safe BPF is dumb — 256 B prefixes, 8 B sock metadata, 256 KiB RingBuf, drop on `reserve` fail. Complexity is the correlator and OTLP. CO-RE via host BTF; image does not ship a kernel.

**Language add-on (say if they are a Rust interviewer):** the agent shares the correlator with `Arc<Mutex<_>>` across a RingBuf drain task and a TUI. Export snapshots the histogram under the lock, then `.await`s HTTP **outside** so the drain never blocks on the collector. Decode is `read_unaligned` after a length check — a small `unsafe` island. Full walk: [RUST_GRILL.md](RUST_GRILL.md).

---

## 1d. 30-second (security-adjacent JD)

It’s a privileged node agent, not a sidecar. OpenSSL uprobes see HTTP plaintext; we never log prefixes, we redact Authorization and cookies before OTLP, and we skip dual-plane syscall+TLS merge. Completeness is not an SLA — RingBuf drops are a first-class metric.

---

## 1e. 30-second (if they already know eBPF)

Aya CO-RE agent: tracepoints + OpenSSL uprobes, kind-tagged RingBuf, `(tgid,fd)` HTTP/1.1 FSM, httparse userspace, cumulative OTLP JSON. Kind scrape green after a collector 400 (unclosed JSON + Prometheus-style bucketCounts). Nested PID on kind-in-Docker labeled `proc:unknown`. Stretch HTTP/2 not done.

---

## 1f. 15-second elevator (career fair)

Rust eBPF agent: HTTP latency without an SDK, OpenSSL for HTTPS, histograms into Prometheus on kind. Honest gaps: not HTTP/2, not all TLS libraries.

---

## 2. 2–3 minute script (default)

**Problem.** SDKs don’t get adopted. I still wanted per-route latency and a service map.

**Kernel plane.** Tracepoints stash pointers on syscall enter, emit a kind-tagged record on exit. Connect/accept fill `SOCK_META[(tgid,fd)]`. I/O copies a bounded prefix. If the RingBuf can’t reserve, we **increment a drop counter** — no silent loss, no kernel wait.

**Userspace.** Decode by first-byte `EventKind`. Correlator: pending half on `(tgid, fd)`, 60 s timeout. Latency is response-half **exit** minus request-half **exit**. HTTP parse, path normalize, redact. TLS: `SSL_set_fd` then `SSL_read`/`write`; **client write→read only** so a local OpenSSL client+server doesn’t double-count. Skip syscall I/O on TLS-marked fds — **no dual-plane**.

**Export.** In-RAM cumulative histograms, POST OTLP JSON every 10 s. Drain never blocks on the collector. Kind: collector 400 until I closed the JSON object and sent **per-bucket** counts (sum equals `count`), not Prometheus running sums.

**Honest limits.** HTTP/1.1 pipelining can mis-pair. HTTP/2 needs stream IDs — not shipped. Kind-in-Docker: BPF PIDs are the WSL host; `/host/proc` is the kind node → `proc:unknown`. Destination names can still come from **pod IP index**. CPU &lt;2% is a target, not a measured production number.

---

## 2b. 5-minute version (hiring manager)

| Time | Beat |
| --- | --- |
| 0:00–0:10 | Constraint: no SDK, OpenSSL-only TLS, metrics not traces |
| 0:10–0:50 | Whiteboard: probe → maps → RingBuf → correlate → httparse → OTLP |
| 0:50–1:05 | Latency = resp exit − req exit, not wire RTT |
| 1:05–1:20 | RingBuf drop + counter |
| 1:20–1:40 | TLS: library boundary; privileged; redact; client-only |
| 1:40–1:55 | correctness3 p50 ≈ 50.8 ms vs 50 ms |
| 1:55–2:20 | Kind scrape + OTLP 400 scar |
| 2:20–2:35 | kind PID lie |
| 2:35–2:45 | Stretch: HTTP/2, CPU stacks; overhead not proven |
| 2:45–3:00 | Ask: “Want the FSM on the board or the OTLP JSON?” |
| 3:00–5:00 | Their questions — you stop pitching |

If they are bored at 0:40, skip TUI/hdrhistogram and jump to OTLP 400 — that’s the senior-sounding scar.

---

## 2c. 8-minute version (deep systems)

Add after the FSM: `looks_like_request` / `looks_like_response`; replace-pending on a new request; why 48 B vs 288 B; why `SockMeta` is not on every event; why Phase 1 connect latency ≠ HTTP latency; EINPROGRESS; two aggregators (60s hdr vs lifetime histogram). Then stop. Do not narrate every file in `agent/src`.

---

## 3. Whiteboard — say this while you draw

1. Box **app** — “I don’t link anything here.”  
2. Box **BPF** — “bounded, no parser.” Arrow: events.  
3. Box **`(tgid, fd)` pending** — “HTTP/1.1 is write then read on a client.”  
4. Box **httparse** — “userspace; redact before export.”  
5. Box **histogram** — “cumulative over process life; OTLP buckets are per-bucket.”  
6. Side note: “RingBuf full → drop + metric.”  
7. Side note: “HTTPS = uprobe, not decrypt.”

If they want code names: `ebpf/`, `correlate.rs`, `http.rs`, `metrics_registry.rs`, `export.rs`.

**If the marker dies:** talk the same seven points. Don’t freeze.

**If they draw HTTP/2 first:** “My FSM is HTTP/1.1. Multiplexing is Stretch S1 — I’d key `(tgid, fd, stream_id)` after a userspace frame parse. I did not ship that.” Then go back to 1.1.

---

## 3b. Demo voiceover (if they want a video / live kind)

0–10s: “WSL kernel, kind cluster `obsagent`. Not a real node.”  
10–25s: “DaemonSet caps, collector `:8889`.”  
25–50s: “Series `GET /` to ClusterIP — count 99 on the scrape I saved. Ignore Docker `/v1.54` and kube `/readyz`.”  
50–70s: “`events_dropped` and `otlp_dropped` at 0 **that** scrape.”  
70–90s: “`src=proc:unknown` is expected on kind-in-Docker. Dst names from pod IPs.”  
90–110s: “Local gate: `/slow` p50 tracks 50 ms.”  
End: “HTTP/2 not here. CPU &lt;2% not claimed.”

---

## 4. Q&A — product / scope

**Q. How is this not Pixie / Cilium?**  
A. Pixie is a product with protocol decoders, memory, and a cluster control plane. Cilium Hubble is datapath + flow. I have a **node-local reconstruction** of HTTP/1.1 (and OpenSSL HTTPS) into metrics. Completeness and protocol coverage are narrower on purpose.

**Q. Do you emit traces?**  
A. Milestone 5 is **metrics**. Traces were deferred. Don’t invent spans or W3C `traceparent` injection.

**Q. Multi-cluster? Service mesh?**  
A. One kind cluster, no Istio. Service map is in-RAM edges, cap 4096.

**Q. Why metrics not logs?**  
A. Prefixes can hold secrets. Metrics-only UI was a Phase 3 lock. Logs of raw HTTP would be a product and a compliance incident.

**Q. Who is the user?**  
A. For the spike: me, and an interviewer. For a product: SRE who cannot get SDK coverage. I did not do user research.

**Q. Why not just use OpenTelemetry auto-instrumentation?**  
A. Auto-instr is per language/runtime and still a deploy. eBPF is **node-wide** and **wrong more often**. Different trade.

**Q. Why Rust not bcc/Python?**  
A. I wanted a single shippable agent, CO-RE via Aya, and unit tests on the FSM without a kernel. bcc is fine for one-off tools.

**Q. Is this open-source unique?**  
A. No. Pixie, Kindling, Grafana Beyla, Cilium, etc. exist. The value of *this* repo is that I can defend **every** limit.

---

## 5. Q&A — eBPF / kernel

**Q. Why not kprobes on `tcp_sendmsg`?**  
A. I attached **stable-ish tracepoints** on the syscall path plus OpenSSL uprobes. `tcp_*` is more kernel-version coupling and verifier-heavier. I did not ship it. I can still explain it as an alternative for byte-accurate socket internals.

**Q. CO-RE?**  
A. Bytecode relocates using host BTF. No BTF → attach fails. Container image is not a kernel. We mount `/sys/kernel/btf`.

**Q. Verifier story?**  
A. I keep stack tiny, null-check `ringbuf_reserve`, bound copies, no HTTP parse in BPF, 8 B `SockMeta`. I will **not** quote a fake rejection log — the file is empty. I *did* burn time on **userspace** OTLP JSON 400.

**Q. What happens when userspace is slow?**  
A. Kernel drops; `obsagent_events_dropped_total` goes up. Completeness is not promised. Export is async so the drain isn’t coupled to HTTP POST.

**Q. Why 256 KiB RingBuf / 256 B prefix?**  
A. Bounded ABI in `common`. Prefix is enough to httparse a typical request line + some headers; truncation is graceful. RingBuf is a power of two. Bigger buffer is option (b), not the policy.

**Q. RingBuf vs PerfEventArray?**  
A. RingBuf: single consumer, variable-sized records, `reserve`/`submit`. Perf: per-CPU pages, historically fiddly for mixed event sizes. I chose RingBuf for kind-tagged 48 B / 288 B events.

**Q. Helper vs direct pointer?**  
A. User buffers are not kernel-trusted. Enter stashes the pointer; exit uses `bpf_probe_read_user` (conceptually) into the reserved slot. Never “just dereference” userspace.

**Q. 512 B stack?**  
A. Don’t put the 256 B prefix as a BPF stack local if you can write into the RingBuf slot. That’s the design intent — I describe it as a class even without a saved verifier log.

**Q. bpf-linker in Docker?**  
A. Prebuilt musl tarball. `cargo install bpf-linker` in the image was too slow/fragile.

**Q. Why nightly `rust-src`?**  
A. Aya BPF target needs it. Userspace agent can be stable; the **ebpf crate** is the special one.

**Q. Can eBPF deadlock the kernel?**  
A. Helpers are limited; programs must be verifier-safe; we never wait on userspace. That’s why we drop.

**Q. LSM / sleepable BPF?**  
A. Not used. Don’t pretend.

**Q. Map types you know?**  
A. HashMap (pending, sock meta, SSL*→fd), RingBuf, counters. I won’t invent `LPM_TRIE`.

**Q. Per-CPU maps?**  
A. Drop counters are often per-CPU then summed — if asked “I’d open the eBPF crate.” Don’t guess the exact Aya API from memory if you’re shaky.

---

## 6. Q&A — correlation / HTTP

**Q. Where is the request ID?**  
A. There isn’t one. Structure: client **write then read** on the same fd. Server is the inverse; TLS path only implements **client** pairing to avoid double-count.

**Q. Why exit timestamps?**  
A. Enter is “syscall started.” Exit is “kernel finished the copy.” Latency is app-visible completion of the response half minus completion of the request half — not NIC RTT.

**Q. Formula?**  
A. `latency_ns = t_end_ns.saturating_sub(t_start_ns)` on `Exchange`. Both times are BPF `ts_ns` on the **I/O event exits**.

**Q. HTTP/2?**  
A. Multiplexed streams on one fd. Fd-only FSM **will mis-pair**. Stretch S1: parse frames / stream IDs. Not done. Say that in the first minute if they do gRPC.

**Q. How would HTTP/2 work?**  
A. Userspace (or a much smarter BPF) parses frames, keys `(tgid, fd, stream_id)`, maps HEADERS `:path`. gRPC still needs protobuf or at least `:path`. I did not implement it. Don’t whiteboard a fake HPACK decoder.

**Q. Pipelining?**  
A. Documented mis-pair. I don’t pretend HTTP/1.1 is always stop-and-wait. New request can replace pending.

**Q. `writev` / `sendmsg`?**  
A. Not in the Phase 2 attach set. Possible miss — many servers write that way.

**Q. `recvfrom` vs `read`?**  
A. Both in the attach set for Phase 2 (read/write/recvfrom/sendto). Don’t add `readv` unless you’ve checked the crate.

**Q. How do you know it’s HTTP?**  
A. `looks_like_request` (GET/POST/…) and `looks_like_response` (`HTTP/`) then `httparse`. Binary fds don’t become routes.

**Q. Chunked encoding / bodies?**  
A. We pair **prefixes**, not full bodies. Status line in the first 256 B is the common case. Huge headers can truncate — graceful, maybe miss status.

**Q. Keep-alive?**  
A. Same fd, sequential exchanges. FSM returns to idle after emit. That’s the happy path.

**Q. Why 60s timeout?**  
A. Hung halves; aligned with TUI window (Q11 in the phase notes). Not a magic RTO.

**Q. Thread pools?**  
A. Key is tgid+fd, not tid. Worker threads sharing the process fd table still pair.

**Q. Fork?**  
A. Fd table copied; tgid changes. I did not ship special fork handling — say “possible miss / leftover SOCK_META.”

**Q. Unix sockets?**  
A. IPv4 TCP-oriented (`AF_INET`). Don’t claim HTTP over UDS.

---

## 7. Q&A — TLS / security

**Q. Are you decrypting TLS?**  
A. No. I hook **OpenSSL** after encrypt/before decrypt in the library. rustls, Go `crypto/tls`, BoringSSL-first, custom BIO without `SSL_set_fd` → miss.

**Q. Is that legal / safe?**  
A. It’s a **privileged** node agent — same class as many eBPF security/obs tools. We **never log prefixes**, redact `Authorization`/`Cookie`/`Set-Cookie`, metrics-only UI. Secrets can still exist in agent RAM. Cluster must treat the DaemonSet as a trust boundary. See `docs/security.md`.

**Q. Dual-plane (syscall timing + TLS body)?**  
A. Explicitly **not** Milestone 3. TLS-marked fds skip sock I/O.

**Q. EINPROGRESS connect vs handshake?**  
A. Non-blocking connect completing later is **not** the TLS handshake. Phase 1 latency is syscall enter→exit only.

**Q. Why `SSL_set_fd`?**  
A. Need `SSL*` → fd to reuse the same correlator key. rfd/wfd variants exist for split BIOs. Custom BIO that never sets fd → drop.

**Q. `SSL_read_ex`?**  
A. Separate pending map / attach so the `_ex` size_t out-params don’t alias the old stash. Phase 3 hygiene; mention if they ask “did you only hook SSL_read?”

**Q. Kernel TLS (kTLS)?**  
A. Not handled. Don’t pretend uprobes see kTLS payload.

**Q. mTLS / client certs?**  
A. Irrelevant to pairing. We don’t parse certs.

**Q. Can an attacker use this agent?**  
A. If they can load BPF or compromise the DaemonSet, they already have node-level power. RBAC and image pinning are the controls I would add next — not shipped as a full policy product.

**Q. In-kernel redaction?**  
A. Rejected for M3: hot, fragile, verifier-hostile. Redact in userspace before export.

---

## 8. Q&A — Kubernetes / identity / kind

**Q. How do you get pod names?**  
A. cgroup v2 from `/proc/<tgid>/cgroup` on a **real node** with `hostPID`. Plus a **pod-IP index** from the API. Kind-in-Docker on WSL: BPF tgid ≠ node `/proc` → `src=proc:unknown`. Dst `demo/frontend-…` is IP index. I say that before they think Grafana is lying.

**Q. Why `hostPID`?**  
A. BPF sees host PIDs. Without it, the agent’s container PID ns doesn’t match. Cost: you also see kubelet, Docker API, CoreDNS — high cardinality. Filter in userspace is future work, not a fake “we only trace apps.”

**Q. RBAC?**  
A. ServiceAccount can list pods. Index **soft-fails** if the API is down — agent still emits IP:port.

**Q. Why not Downward API only?**  
A. That identifies **the agent pod**, not the **target** process.

**Q. ClusterIP vs pod IP?**  
A. Clients connect to Service IP (`10.96.28.98:8080` in the scrape). Indexing **pod** IPs still helped name some destinations; don’t claim full Endpoints/kube-proxy translation.

**Q. CNI / kube-proxy iptables?**  
A. We see the syscall’s `daddr`. That’s the connect() argument, which may be ClusterIP. We did not parse conntrack.

**Q. Apply order?**  
A. Namespace from smoke4 first; ConfigMap after. Image `kind load` after rebuild or you debug yesterday’s binary.

**Q. Privileged vs caps?**  
A. Prefer caps; `privileged: true` is the documented escape hatch. Kind rolled with the cap set.

**Q. resource limits 1 CPU / 512Mi?**  
A. Agent budget, not a proof of &lt;2% host.

**Q. Why DaemonSet not Deployment?**  
A. Need **every node’s** kernel. One replica is the wrong topology.

**Q. Windows nodes?**  
A. Out of scope. This is Linux BPF.

---

## 9. Q&A — metrics / OTLP

**Q. Why not the OTel SDK?**  
A. Keep the agent small and the ABI under our control. Cost: I implemented JSON wrong once.

**Q. Cumulative vs delta?**  
A. `aggregationTemporality = 2` — histogram **over the process lifetime**. Each OTLP `bucketCounts` array is **still per-bucket**; Prometheus `le` is what the collector accumulates for scrape.

**Q. Worked example?**  
A. Two requests, 3 ms and 12 ms, bounds include 5 and 10 and 25. Per-bucket might be `[0,0,1,0,1,…]` with `count=2`. Prometheus `le=5` would be 1, `le=25` would be 2 after conversion. If I sent `[0,0,1,1,2]` as OTLP buckets, the collector is right to 400.

**Q. The 400?**  
A. Empty payload `{}` was 200. Real payload: unclosed root + cumulative bucket array whose sum ≠ `count`. Fix in `to_otlp_json`; export warn now includes body snippet. Then `:8889` showed the histogram.

**Q. Cardinality?**  
A. Series key `(src,dst,method,route,status_class)`, cap 2048 → `_other`. Edges cap 4096. Kind still floods `/readyz` and Docker `/v1.54` if you don’t mentally filter the demo `GET /`.

**Q. status_class?**  
A. 2xx/3xx/4xx/5xx-style grouping for the key (don’t invent a label you haven’t seen — if unsure: “status and class derived in registry/http parse”).

**Q. Gauge-per-sample?**  
A. Phase 4 defect. Phase 5 replaced it. Don’t describe the old exporter as current.

**Q. Flush interval?**  
A. ~10 s async. Drain is not waiting on POST.

**Q. What if collector is down?**  
A. `otlp_dropped` should move; events can still correlate. I did not implement a disk WAL.

**Q. Exemplars / traces on buckets?**  
A. No.

**Q. Units?**  
A. Histogram is milliseconds (`http_client_duration_milliseconds`). Internal record is ns then `/ 1e6`.

---

## 10. Q&A — overhead / ops / tests

**Q. Overhead?**  
A. Target &lt;2% under a **documented** load. WSL `ps` bursts: Phase 1 ~1.6%, Phase 2 ~10.7%, Phase 3 ~3.5%. I will not quote &lt;2% as a result.

**Q. How do you run it?**  
A. Checkout on Windows; **build BPF in WSL2 Ubuntu** (`scripts/wsl-run.sh`). Never compile `obsagent-ebpf` on Windows. Root for attach smokes.

**Q. CARGO_TARGET_DIR?**  
A. `/home/sahil/.cache/obsagent-target` so the Windows filesystem doesn’t host the BPF build.

**Q. Dockerfile / bpf-linker?**  
A. Image uses a **prebuilt musl bpf-linker tarball**.

**Q. How many tests?**  
A. `test-agent` 38 passed (2026-08-13). Kernel tests are smoke/correctness scripts, not `cargo test` in `bpfel`.

**Q. correctness band?**  
A. p50 within max(±10 ms, ±10%) of injected delay.

**Q. Why count=5 not 10?**  
A. Repeat loop used to double-count; gate asserts 5.

**Q. CI?**  
A. Don’t overclaim GitHub Actions if you didn’t show it. Speak WSL gates you actually ran.

---

## 11. Q&A — “what would you do next?”

Pick **two**, not a laundry list:

1. **HTTP/2 stream IDs** (Stretch S1) — otherwise gRPC interviews kill you.  
2. **Userspace allowlist** — drop kube/docker cardinality.  
3. **Real-node identity** — not kind-in-Docker.  
4. **Verifier log** — one real rejection with bytecode SHA if you hit one.  
5. **Overhead protocol** — host-normalized, not `ps` burst.  
6. **Sampling when drop_rate high** — still philosophy (a).  
7. **writev/sendmsg** attach — completeness for real servers.

Do not promise DistServe-style theater, Istio, or a Grafana SaaS.

**What I would not do next:** parse HTTP in BPF; dual-plane merge “because it’s cool”; fake pod names.

---

## 11b. Q&A — Rust (short; full answers in RUST_GRILL)

If they stay on language for more than 10 minutes, switch your brain to [RUST_GRILL.md](RUST_GRILL.md). Below is the **spoken** subset.

**Q. Why Rust?**  
A. Aya CO-RE, one language for ABI + BPF + agent, `repr(C)` shared structs, unit-test the FSM without a kernel.

**Q. What’s `no_std`?**  
A. `common` and `ebpf` don’t use libstd — no OS allocator in BPF. Agent is normal std/Tokio. Feature `user` on common adds `aya::Pod` only for the agent.

**Q. Ownership in the correlator?**  
A. `Correlator` owns the HashMap. `observe(&mut self, ev: &SockIoEvent)`. Prefix kept as owned `Vec<u8>`. Completed `Exchange` is moved into `handle_exchange`.

**Q. `Arc<Mutex<T>>`?**  
A. Drain task, TUI, and export share state. `Arc::clone` is a refcount, not a deep copy. Mutex = exclusive mutation of the FSM/aggs.

**Q. Why not await OTLP on the drain?**  
A. `record` is sync under a short lock. A background task snapshots JSON then `reqwest` `.await`. Kernel RingBuf cannot wait.

**Q. Where is unsafe?**  
A. `libc::setrlimit`; `ptr::read_unaligned` in `decode.rs` after `len` check; `unsafe impl Pod` on ABI structs; BPF helpers. Not “the whole agent is unsafe.”

**Q. `Option` vs `Result`?**  
A. Bad event → `Option` skip. Failed attach → `anyhow::Result` from `main`. Failed POST → warn + `otlp_dropped`, drain lives.

**Q. Mutex poison?**  
A. `lock_mut` uses `into_inner()` so one panic doesn’t freeze the agent.

**Q. Rate yourself in Rust?**  
A. I can maintain this workspace. I’m not a language lawyer. See RUST_GRILL §22 — don’t say 9/10.

**Q. Did you use AI to write it?**  
A. I owned the architecture, ABI locks, and gates. I can open any module and explain it. I won’t pretend I invented Aya or that I typed every line in a vacuum. Then **go to a type** — if you can’t, you aren’t ready.

---

## 12. If they mix Atlas into this interview

“That’s a separate repo: an LLM **serving** gateway with measured routing. This one is kernel HTTP reconstruction. I can do either whiteboard; I won’t blend the numbers.”

If they insist on comparing: Atlas is **userspace load/routing measurement**. This is **kernel observation**. Both have an honesty theme (WEAKENED vs kind PID). That’s the only metaphor I’d allow — then stop.

---

## 13. Mock rounds (practice with a timer)

### Round A — 20 min recruiter-plus (SRE)

They ask: 30s pitch → why not SDK → how latency is defined → kind result → is it production → overhead → HTTP/2.  
Pass if: you said unknown PID, no &lt;2%, no Pixie, p50 number with band.

### Round B — 45 min systems

They ask: draw maps → verifier classes → RingBuf options a/b/c → TLS double-count → OTLP bucket example → writev miss.  
Pass if: you refuse a fake verifier SHA; bucket example has sum=count.

### Round C — 45 min k8s

They ask: DaemonSet yaml from memory (caps, hostPID, mounts) → why not Deployment → RBAC soft-fail → cardinality → apply order.  
Pass if: you explain nested PID without blaming “a Kubernetes bug.”

### Round D — behavioral 15 min

Mistake = OTLP 400. Tradeoff = TLS client-only. Conflict = (use a real job story, not this repo). Strength = writing limits down.

### Round E — 45 min Rust (use RUST_GRILL)

They ask: workspace crates → why common is `no_std` → draw Arc/Mutex tasks → `observe` signature → `Option` skip vs `Result` attach → show `decode.rs` unsafe ritual → export lock vs await → `Pod` / `repr(C)` → rate yourself.  
Pass if: you never say “Rust has no unsafe”; you distinguish Arc clone vs T clone; you don’t claim senior lifetimes.

---

## 14. Interrupt recoveries

**“That’s just strace.”**  
“strace `-p` is ptrace-stop heavy and per-process. eBPF is in-kernel, bounded, node-wide. I still don’t parse HTTP in the kernel.”

**“So you decrypt TLS.”**  
“No. OpenSSL uprobe. Wire stays ciphertext. Privileged process sees library plaintext.”

**“This would never pass our security review.”**  
“Agree it’s a trust-boundary agent. Caps not privileged-first, redact, no prefix logs. Your review might still say no — that’s a product decision.”

**“p50 50.8 vs 50 is noise.”**  
“It’s inside the gate band. The point is the FSM clock tracks injected delay, not that we have atomic clocks.”

**“99 requests is tiny.”**  
“It’s a demo scrape, not a load test. I will not pretend otherwise.”

**“Show me production.”**  
“I don’t have it. I have WSL gates and kind.”

**“You’re reading a script.”**  
Slow down. Draw. Use one number. Stop.

---

## 15. Live-coding / take-home hints (if they go there)

They might ask you to sketch `observe_inner` or OTLP buckets on a laptop.

- Key: `(tgid, fd)`; pending one half; opposite dir; timeout 60s.  
- Do not add HTTP/2.  
- Buckets: increment **one** index; serialize as per-bucket; `count` = sum.  
- Don’t spend the hour on Aya boilerplate if they wanted the FSM.

If they ask you to write an **exploit** or “bypass the verifier”: refuse. This pack is defensive/observability.

---

## 16. Closing question you can ask them

- “Do you run node agents with hostPID today, and how do you bound cardinality?”  
- “Is L7 eBPF a goal, or do you want SDK traces only?”  
- “What’s the TLS library mix in the fleet — OpenSSL vs Go vs rustls?”

Asking that last one shows you know **coverage** is the real product problem.

---

## 17. Cheat card (night before — one page)

```text
Pitch: no SDK → tracepoints + OpenSSL uprobes → (tgid,fd) FSM → httparse → OTLP
Latency: resp_exit - req_exit   NOT wire, NOT connect(), NOT Instant
RingBuf: 256KiB drop+counter    never block kernel
TLS: library, client-only, no dual-plane, no rustls
Numbers: p50≈50.8ms / 50ms     GET / count=99     drops 0 that scrape
Scar: OTLP 400 = unclosed JSON + cumulative bucketCounts
Kind: src=proc:unknown nested PID; dst names from IP index
Never: Pixie, <2% CPU, HTTP/2 shipped, fake verifier log
Next: HTTP/2 OR allowlist OR real node
```

---

## 18. Word-for-word 2-minute (practice, then throw away)

Do not recite this in the room. Use it to hear cadence.

“The problem is SDK coverage. I wanted HTTP latency on a node without asking every team to instrument.

In the kernel I attach tracepoints on connect, accept, read, and write. On enter I stash the userspace buffer pointer. On exit I copy at most 256 bytes into a RingBuf record. The first byte is an event kind so userspace can demux 48-byte connect events from 288-byte I/O events. If reserve fails I increment a drop counter. The kernel never waits for me.

Userspace runs a state machine keyed by process id and file descriptor. A client is write then read. Latency is response-exit timestamp minus request-exit timestamp — not wire RTT, not connect time. I parse with httparse, normalize numeric path segments, and redact cookies before anything leaves the box.

HTTPS encrypts before the syscall, so I uprobe OpenSSL SSL_read and SSL_write and reuse the same state machine. I only pair client write-to-read so a local client and server both using OpenSSL don’t double-count. I do not merge syscall timing with TLS content.

I export cumulative OTLP histograms. In kind the collector returned 400 until I closed the JSON and sent per-bucket counts whose sum equals count — not Prometheus running sums. After that, scrape showed GET slash count 99 and zero drops.

Limits: not HTTP/2, not rustls, kind-in-Docker shows unknown sources because of nested PIDs, and I will not claim under two percent CPU.”

**Timer fail modes:** if you hit 2:30, you added Pixie. If you finish at 0:40, you skipped the 400 or the PID lie — add one.

---

## 19. Good vs bad answers (same question)

**Q. How do you correlate requests?**

Bad: “We use trace IDs in eBPF.”  
Good: “There is no ID. HTTP/1.1 on one fd is write then read. `(tgid, fd)` pending half, 60s timeout.”

**Q. Overhead?**

Bad: “It’s less than two percent, production-grade.”  
Good: “Target under a documented load. I have WSL `ps` bursts, Phase 2 about 10.7%. Not proven.”

**Q. Kubernetes?**

Bad: “It identifies all pods in kind.”  
Good: “Designed for hostPID + cgroup + IP index. Kind-in-Docker nested PIDs → unknown src. Dst names from IPs.”

**Q. TLS?**

Bad: “We decrypt HTTPS.”  
Good: “OpenSSL library boundary. Wire is ciphertext. Privileged agent.”

**Q. Verifier?**

Bad: “It rejected my program because of line 47 in foo.c.” (untrue)  
Good: “I keep BPF dumb — bounded copies, null-check reserve, no parser. I don’t have a saved rejection log. I do have an OTLP JSON 400.”

**Q. Why 99?**

Bad: “We handled 99 QPS in production.”  
Good: “Cumulative histogram count on that scrape for the demo GET / series.”

---

## 20. Extra Q&A — maps, ABI, Aya

**Q. How is `(tgid, fd)` packed?**  
A. `u64` map key in BPF; userspace `SockKey` struct. I’d open `ebpf` if they want the shift.

**Q. Can two processes have the same fd number?**  
A. Yes. That’s why tgid is in the key.

**Q. Can one process reuse fd 3 after close?**  
A. Yes. SOCK_META must be deleted on close or you join the wrong peer. If they go deep: “cleanup on close/SSL_free is the bug farm; I treated it as Phase 3/4 hygiene.”

**Q. Why not key on 4-tuple?**  
A. Not on every I/O event; NAT; we join 4-tuple from SOCK_META. FSM is fd-shaped because HTTP is fd-shaped.

**Q. `repr(C)` vs `repr(Rust)`?**  
A. Kernel and userspace must agree on layout. `repr(C)` + padding fields. Sizeof tests belong in `common`.

**Q. Endianness of IPs?**  
A. `daddr_be` / `dport_be` — network order. Formatting in userspace must ntohl/ntohs. Getting this wrong makes service maps look like nonsense IPs.

**Q. Aya vs libbpf-rs?**  
A. I used Aya. libbpf is the C ecosystem. I won’t pretend I shipped libbpf.

**Q. How do you load programs?**  
A. Agent includes the ELF (aya include_bytes style — “compiled bpf crate linked into the agent”). Load, attach tracepoints/uprobes, then drain. Unload on exit.

**Q. Multiple programs, one RingBuf?**  
A. Yes — kind byte demux. That’s why kind is first byte.

**Q. Could userspace block in a probe?**  
A. No. Probes are in kernel context. That’s the whole backpressure talk.

---

## 21. Extra Q&A — HTTP edge cases

**Q. HEAD requests?**  
A. In the looks_like_request list. Response may have no body; we still pair prefixes.

**Q. 100 Continue?**  
A. Extra `HTTP/` half can confuse the FSM. I did not special-case. Honest miss.

**Q. WebSocket upgrade?**  
A. After 101, bytes are not HTTP. Later chunks fail looks_like_*. Connection may stall pending until 60s.

**Q. HTTP/1.0 vs 1.1?**  
A. `HTTP/` prefix matches both. Fine.

**Q. TLS inside HTTP CONNECT?**  
A. Nightmare. Not supported.

**Q. gRPC-web / trailers?**  
A. HTTP/2 or weird HTTP/1.1. Don’t claim.

**Q. Gzip bodies?**  
A. We don’t need bodies for status/method if they’re in the first 256 B of each half.

**Q. Very long URLs?**  
A. Truncation; parse may fail; no series. Better than kernel parse.

**Q. Unicode paths?**  
A. httparse bytes; normalize is ASCII-segment heuristics. Don’t claim NFC.

**Q. Query params in cardinality?**  
A. Dropped from route key (design note).

---

## 22. Extra Q&A — Prometheus / Grafana

**Q. Did you run Grafana in kind?**  
A. Dashboard **JSON** exists. Proven path is collector `:8889`. Don’t say “we deployed Grafana” unless you did in that session.

**Q. histogram_quantile?**  
A. Needs cumulative `le` buckets from Prometheus. Collector converts OTLP. If buckets are wrong, quantiles are fiction.

**Q. rate() on histograms?**  
A. Cumulative count/sum over process life; `rate()` needs the counter semantics. Gauge-per-sample would make `rate()` garbage — that’s why P5 exists.

**Q. Recording rules?**  
A. Not shipped.

**Q. Metric names?**  
A. `http_client_duration_milliseconds`, `obsagent_events_dropped_total`, `obsagent_otlp_dropped_total`. If you forget, say “client duration histogram + self-metrics.”

---

## 23. Extra Q&A — process / TDD

**Q. How did you work?**  
A. Locked design questions per phase (Q1–Qn), then implement, then `wsl-run.sh` gates. Handoff markdown per session.

**Q. What is a locked Q?**  
A. A decision I refuse to reopen mid-phase (RingBuf policy, dual-plane, etc.) so the agent doesn’t thrash.

**Q. TDD?**  
A. Userspace FSM and registry tests first-ish; kernel proven by smoke/correctness. Not orthodox TDD on BPF.

**Q. Why Windows checkout?**  
A. That’s my desktop. BPF is WSL2. The split is a tax I document so I don’t accidentally compile on MSVC.

---

## 24. Extra Q&A — Stretch S1/S2 (speak as future, not present)

**S1 HTTP/2:** parse frames in userspace from the same 256 B? **Probably not enough** — need stream state across chunks, maybe larger prefix or reassembly. Key becomes `(tgid, fd, stream_id)`. HPACK is the boss fight. gRPC `:path`. I have not started.

**S2 CPU stacks:** `perf_event` + blazesym symbolize + join onto **slow** HTTP exchanges. Cardinality and overhead explode. I have not started.

If they want you to design S1 on the board: state the key change, state prefix insufficiency, stop before writing a HPACK decoder.

---

## 25. Pair-interview / “explain to a junior”

Junior: “So it’s Wireshark?”  
You: “Wireshark is packets. After TLS, packets are useless for HTTP. I hook the library or the syscall. Also I aggregate, I don’t keep pcaps.”

Junior: “So it’s Jaeger?”  
You: “No traces. Histograms. No context propagation.”

Junior: “Why Rust?”  
You: “Aya + one repo. The BPF is still tiny C-shaped structs.”

---

## 26. Phone screen 10-minute agenda (you drive)

0:00–0:30 30s pitch  
0:30–2:00 FSM + latency definition  
2:00–3:30 TLS one minute  
3:30–5:00 OTLP 400  
5:00–6:30 kind PID  
6:30–8:00 their stack (OpenSSL vs Go?)  
8:00–10:00 their questions / your closing Q

If they steal the agenda, let them. The cheat card still fits in the gaps.

---

## 27. Onsite 45-minute agenda (they drive; you have pockets)

Pocket A: draw architecture (5 min)  
Pocket B: FSM (10 min)  
Pocket C: RingBuf options table (5 min)  
Pocket D: OTLP numeric example (5 min)  
Pocket E: k8s yaml from memory (5 min)  
Pocket F: limits + next (5 min)  
Rest: their deep dive

If they spend 40 minutes on verifier theory: stay at **classes**. Do not invent.

---

## 28. After they say “any questions for us?”

Pick **one**:

1. How do you handle TLS library mix in the fleet?  
2. What’s your policy on hostPID node agents?  
3. Do you need HTTP/2 L7 from eBPF or is SDK traces enough?

Avoid: “What’s the tech stack?” if the JD already said. Avoid: salary in this slot if there’s a recruiter.

---

## 29. Red flags in **your** answers (self-check)

You said “always,” “all pods,” “all TLS,” “zero overhead,” “like Pixie,” “decrypt,” “production,” “I have a verifier log,” “HTTP/2 works,” “<2%,” “Grafana in cluster,” “traces,” “SDK,” “we never drop.”

Cross them out. Replace with the cheat card.

---

## 30. 50 rapid-fire (answer in ≤8 words)

1. SDK? — No, node probes.  
2. HTTP key? — tgid and fd.  
3. Latency? — Response exit minus request exit.  
4. Prefix? — 256 bytes.  
5. RingBuf? — 256 KiB, drop plus counter.  
6. Parse where? — Userspace httparse.  
7. TLS? — OpenSSL uprobes.  
8. Dual-plane? — Not in milestone 3.  
9. HTTP/2? — Not shipped.  
10. rustls? — Unsupported.  
11. Traces? — Deferred.  
12. IPv6? — Not first-class.  
13. writev? — Not attached.  
14. Clock? — Monotonic bpf ktime.  
15. FSM timeout? — 60 seconds.  
16. TUI window? — 60 seconds.  
17. OTLP temporality? — Cumulative, 2.  
18. bucketCounts? — Per-bucket, sum equals count.  
19. Empty JSON? — Collector 200, false green.  
20. Real 400? — Unclosed JSON and cumulative buckets.  
21. kind src? — proc unknown.  
22. Why? — Nested PID namespaces.  
23. Dst names? — Pod IP index.  
24. hostPID? — Match BPF tgid to proc.  
25. Caps? — BPF PERFMON PTRACE RESOURCE.  
26. BTF? — Host vmlinux, mounted read-only.  
27. Image kernel? — No.  
28. Windows BPF? — Never.  
29. p50 number? — About 50.8 vs 50 ms.  
30. count 99? — That scrape, GET slash.  
31. Drops that scrape? — Zero.  
32. Overhead claimed? — No.  
33. Phase 2 ps? — About 10.7 percent burst.  
34. Series cap? — 2048.  
35. Edge cap? — 4096.  
36. Client-only TLS? — Avoid double count.  
37. looks like request? — Method prefix.  
38. looks like response? — HTTP slash.  
39. Peer join? — After FSM, SOCK_META.  
40. SockMeta size? — 8 bytes.  
41. Event kinds? — 1–4.  
42. Connect size? — 48 bytes.  
43. Tests? — 38 userspace.  
44. Grafana proven? — Scrape 8889.  
45. Privileged? — Fallback, kind used caps.  
46. Soft-fail k8s? — Yes, emit IP.  
47. Secrets? — Redact, still in RAM.  
48. Next? — HTTP/2 or allowlist or real node.  
49. Pixie? — Different product.  
50. Lie? — Don’t.

Drill until 50 is boring. Then you are ready.
