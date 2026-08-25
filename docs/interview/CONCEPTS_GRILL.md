# eBPF Observability Agent — concepts, code, grill

**Use when:** they go deep on eBPF, correlation, TLS, OTLP, identity, or “how does this actually work?”  
**Companion:** [RESUME_AND_STORY.md](RESUME_AND_STORY.md) · [SCRIPT_AND_QNA.md](SCRIPT_AND_QNA.md) · [RUST_GRILL.md](RUST_GRILL.md) (language / ownership / Tokio / unsafe — **this codebase**)  
**Code tour:** [../architecture/PROJECT-DEEP-DIVE.md](../architecture/PROJECT-DEEP-DIVE.md)  
**Evidence:** [../testing/phase-5.tdd.md](../testing/phase-5.tdd.md) · [../handoff/SESSION-2026-08-15-kind-e2e.md](../handoff/SESSION-2026-08-15-kind-e2e.md)

This is a **kernel observability / systems** project (Rust + Aya). It is **not** an APM SaaS, not Cilium, not Pixie. If they ask “eBPF fundamentals,” map only what this repo forces you to know. Inventing Pixie internals or a verifier SHA from an empty log is worse than saying “I don’t have that log.”

**Build rule:** Windows checkout; **compile BPF only in WSL2 Ubuntu**. Never `cargo build -p obsagent-ebpf` on Windows.

**How to drill:** pick one subsection, draw the whiteboard from memory, then open the named file and walk the types. If you cannot name the file, you do not know it yet.

**Rust language grill is a separate file** on purpose: eBPF physics ≠ borrow checker. If the round is “why `Arc<Mutex<Correlator>>`,” go to [RUST_GRILL.md](RUST_GRILL.md), not §1.5.

---

## 0. Architecture (say this on a whiteboard)

```text
Target process (no SDK)
  glibc connect/accept/read/write          OpenSSL SSL_read/SSL_write
           │                                         │
           ▼ tracepoints                             ▼ uprobes (plaintext)
┌─────────────────────────────────────────────────────────────────┐
│  eBPF (ebpf/)  — dumb, bounded, verifier-safe                   │
│  PENDING HashMap  SOCK_META[(tgid,fd)]→{daddr,dport}  (8 B)     │
│  EVENTS RingBuf (256 KiB)   DROPS counter on reserve fail       │
└──────────────────────────────┬──────────────────────────────────┘
                               │ kind-tagged records (1 | 2 | 3 | 4)
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│  agent (Tokio)  decode.rs → correlate.rs → http.rs              │
│  identity.rs + k8s_index.rs → service_map.rs                    │
│  HttpAggregator (60s TUI)  │  MetricsRegistry (cumulative OTLP) │
│  Ratatui / headless print  │  POST /v1/metrics JSON             │
└──────────────────────────────┬──────────────────────────────────┘
                               ▼
                    otel-collector → Prometheus :8889
                    Grafana obsagent.json (optional JSON, not in-cluster proof)
```

**Crates:** `common` (`#[repr(C)]` ABI, `no_std`) · `ebpf` (`bpfel-unknown-none`) · `agent` (userspace). Optional `xtask` / testdata Axum `/slow`.

**Two hard boundaries (say them if they interrupt):**

1. **BPF never parses HTTP.** Prefix copy only. Parser is `httparse` in userspace.
2. **Userspace never blocks the RingBuf drain on collector I/O.** `record` is sync; flush is async ~10s.

**What “no SDK” means:** the target binary is not relinked, not restarted for a library, not required to speak OTLP. Cost is **privilege** (caps + host mounts), **reconstruction error** (no request ID), and **library coverage** (OpenSSL vs rustls).

---

## 0b. One request, end-to-end (cleartext)

Say this as a story so they see time, not boxes.

1. App `connect()` → enter tracepoint stashes args; exit emits `EventKind::Connect` (`SockLatencyEvent`, 48 B) and writes `SOCK_META[(tgid,fd)]`.
2. App `write(fd, "GET /slow HTTP/1.1\r\n…")` → enter stashes userspace buffer pointer; exit copies `min(ret, 256)` bytes into `SockIo` (`kind=3`).
3. If `ringbuf_reserve` fails → BPF `drop_count++`, event gone.
4. Agent drain: first byte `3` → decode `SockIoEvent` → `Correlator::observe` → pending `(tgid, fd)` with dir=Write.
5. App `read` of `"HTTP/1.1 200 …"` → second `SockIo` → opposite dir + `looks_like_response` (`HTTP/`) → `Exchange` with `t_start_ns` / `t_end_ns`.
6. Join peer from `SOCK_META` / `peer_cache` (correlator itself does **not** read BPF maps).
7. `parse_exchange` → method, `normalize_path`, status. `redact_headers` before any off-node sink.
8. `HttpAggregator` 60s window (TUI p50/p95/p99) **and** `MetricsRegistry::record` (process-lifetime histogram).
9. Every ~10s `export.rs` POSTs OTLP JSON. Prometheus scrapes collector `:8889`.

**HTTPS:** steps 2–5 use `TlsIo` (`kind=4`) from uprobes, `observe_client` only (write→read). Sock I/O on that fd is skipped (no dual-plane).

**correctness3:** Axum injects **50 ms** on `/slow`; observed **p50 ≈ 50.82 ms** (band max(±10 ms, ±10%)). That is the definition of “the FSM’s clock is not nonsense.”

---

## 0c. Glossary (use their words, then ours)

| They say | You say | In this repo |
| --- | --- | --- |
| probe | program that runs on a kernel/user event | tracepoints + OpenSSL uprobes in `ebpf/` |
| map | kernel-owned typed storage | PENDING, SOCK_META, EVENTS RingBuf, drop_count |
| RingBuf | MPSC byte pipe kernel→user | 256 KiB, power of two (`EVENTS_RINGBUF_BYTES`) |
| verifier | static checker before load | 512 B stack, bounded loops, checked pointers |
| BTF | type info for the running kernel | `/sys/kernel/btf/vmlinux` |
| CO-RE | same bytecode, relocated offsets | Aya + bpf-linker; image does **not** ship a kernel |
| tgid | thread-group id ≈ userspace PID | correlation key with `fd` |
| pid (BPF) | often the **thread** id | do not key HTTP on tid alone |
| uprobe | break at a userspace function | `SSL_read` / `SSL_write` / `SSL_set_fd` |
| tracepoint | stable-ish kernel hook | `sys_enter/exit_*` |
| OTLP | OpenTelemetry wire format | we POST **JSON** `/v1/metrics`, not gRPC |
| histogram temporality | cumulative vs delta over time | `aggregationTemporality = 2` = cumulative **series** |
| bucketCounts | OTLP per-bucket array | **not** Prometheus `le` running sums |
| hostPID | pod sees host PID namespace | required so `/host/proc/<tgid>` matches BPF |
| kind-in-Docker | k8s nodes are containers on Docker/WSL | **nested PID**; `src=proc:unknown` |

---

## 0d. Probe types (they will ask “why not kprobe?”)

| Kind | Where it fires | Stability | What we use |
| --- | --- | --- | --- |
| **Tracepoint** | named kernel event (`sys_enter_connect`) | better than raw kprobe on internals | syscall enter/exit |
| **kprobe** | any kernel symbol | breaks across builds | not the primary attach |
| **uprobe** | userspace function in a mapped ELF | needs the right `.so` | OpenSSL |
| **USDT** | explicit app static probe | needs app cooperation | **not used** (that would be an SDK-shaped contract) |
| **XDP / TC** | packets | great for L3/L4, bad for HTTP in TLS | **not used** |

**Say:** “Tracepoints for syscalls because I want the POSIX contract. Uprobes for TLS because the wire is ciphertext. I did not put HTTP parse in XDP.”

**Trap:** “eBPF sees all packets so I see all HTTP.” TLS and HTTP/2 both kill that sentence.

---

## 0e. pid vs tgid vs fd (draw three columns)

- **tgid:** process. `getpid()` in the app. Fd table is per process.
- **pid in BPF:** often the thread. Two threads of one process share fds.
- **fd:** integer in **that** process’s table. Fd 3 in nginx is not fd 3 in curl.

**Why key `(tgid, fd)` not `(pid, fd)`:** HTTP on a connection is process-fd, not thread-fd. Thread-keyed state would split one TCP connection across workers (or miss pairing).

**Why not 4-tuple alone:** NAT, connection reuse, and “I don’t have the 4-tuple on every I/O event” (we store it in `SOCK_META` at connect/accept). 4-tuple is **join** data, not the FSM key.

---

## 1. Concept → physics → code → what to say → trap

### 1.1 Why eBPF (and why not an SDK)

**Physics.** An SDK needs a language runtime, a deploy, and a restart. Kernel probes see every process on the node that hits the attach points — including ones that will never get a ticket to add OpenTelemetry.

**Code.** Tracepoints on `sys_enter/exit_connect`, `accept4`, `read`/`write`/`recvfrom`/`sendto`; OpenSSL uprobes in `ebpf/`. Agent loads via Aya in `agent/src/main.rs`.

**Say:** “Zero instrumentation in the app. Cost is privilege, verifier, and reconstruction — not a magic request ID.”

**Trap:** Claiming you see *all* HTTP. HTTP/2, pipelining, rustls/Go TLS, custom OpenSSL BIO, `writev`/`sendmsg` are documented misses.

**If they push “why not just sidecars?”:** sidecars still need a mesh or a rewrite. This is **node-local, library-and-syscall**. Different completeness, different privilege.

---

### 1.2 Verifier (tiny stack, no unbounded loops)

**Physics.** Before the program runs, the kernel checker proves: no unbounded loops, stack ≤ 512 bytes, every pointer from a map or `ringbuf_reserve` is null-checked, helper calls (`bpf_probe_read_*`) for kernel/user memory, no arbitrary kernel writes.

**Code (how we stay legal):**

- Fixed-layout events in `common/src/lib.rs`: `SockLatencyEvent` 48 B; `SockIo`/`TlsIo` 288 B twins; prefix **256 B** (`SOCK_IO_PREFIX_LEN`).
- Stash pointers on enter, copy bounded prefix on exit — no HTTP parse in BPF.
- First byte of each RingBuf record is `EventKind` (`Connect=1`, `Accept=2`, `SockIo=3`, `TlsIo=4`).
- `SOCK_META` value is **8 bytes** (`SockMeta`) — map values that bloat every I/O event were rejected as a *design*, not after a failed patch.
- Pending enter map sized (`PENDING_MAP_ENTRIES = 8192`) so the HashMap is bounded.

**Say:** “I keep BPF dumb. Complexity lives in userspace where I can unit-test the FSM.”

**Trap:** Inventing verifier logs. `docs/verifier-rejection-log.md` is **empty of real entries**. Speak **classes**:

| Class | What it looks like | How this repo avoids it |
| --- | --- | --- |
| Stack overflow | too many locals / large arrays on stack | events live in RingBuf slots, not BPF stack copies of HTTP |
| Null reserve | use RingBuf pointer without check | check `reserve` → else drop_count |
| Unbounded loop | parse HTTP headers in BPF | parse in userspace |
| Invalid CO-RE | field offset without BTF | require `/sys/kernel/btf/vmlinux` |
| Helper misuse | read user memory without `bpf_probe_read_user` | enter-stash pointer, exit copy with helper |

Do **not** fabricate a bytecode SHA or a `R0 invalid mem access` log you did not save.

**Userspace scar (real, 2026-08-15):** OTLP JSON 400 — unclosed root object + Prometheus-cumulative `bucketCounts`. Collector scrape after fix. That *is* interview-safe scar tissue. Prefer it over a fake verifier war story.

---

### 1.3 CO-RE and BTF

**Physics.** Kernel structs (`sock`, `sockaddr_in`) change offsets between builds. Compile Once — Run Everywhere: the loader relocates field access using BTF from the **running** kernel, not from the compile machine’s headers alone.

**Code.** Aya + nightly `rust-src` + `bpf-linker`. Dev host and kind node both showed BTF present (`6.6.114.1-microsoft-standard-WSL2`). Image does **not** ship a kernel. Dockerfile uses a **prebuilt musl bpf-linker tarball**, not `cargo install bpf-linker` on every image build.

**Say:** “Without BTF, CO-RE sock fields don’t relocate; attach fails. That’s a **host** property. The container mounts `/sys/kernel/btf` read-only.”

**Trap:** “Docker image includes the kernel.” It does not (`deploy/Dockerfile`).  
**Trap:** “WSL2 always has BTF.” We **checked**; we did not assume every laptop.

**If they ask “clang vs Aya”:** Aya is a Rust toolchain that still emits BPF ELF + BTF relos. The verifier does not care that the source was Rust.

---

### 1.4 RingBuf backpressure

**Physics.** The kernel path that hits your probe cannot sleep waiting for a slow agent. `bpf_ringbuf_reserve` either returns a slot or it doesn’t. PerfEventArray has different loss semantics (per-CPU pages); we chose RingBuf for a single consumer and variable-sized kind-tagged records.

**Decision (locked):** drop + BPF `drop_count` map. Userspace scrapes → `obsagent.events_dropped` / Prometheus `obsagent_events_dropped_total`. Kind scrape **2026-08-15: 0**. Zero on that scrape is **not** “we never drop.”

**Options you should be able to list (from [../architecture/ring-buffer-backpressure.md](../architecture/ring-buffer-backpressure.md)):**

| Option | Pros | Cons | Ours |
| --- | --- | --- | --- |
| (a) Drop + counter | bounded kernel RAM; loss visible | incomplete under overload | **default** |
| (b) Bigger RingBuf | absorbs spikes | delays the drop; doesn’t fix sustained load | size is 256 KiB, not “infinite” |
| (c) Userspace channel | app-level policy | still need kernel bound; extra copy | drain is fast; export is async |
| Sampling 1-in-N | predictable load | biased metrics | **not shipped**; mention as future under (a) |

**Code.** `common::EVENTS_RINGBUF_BYTES = 256 * 1024` (power of two). Export never blocks drain (`agent/src/export.rs` — sync `record`, async flush every 10s).

**Say:** “Silent loss vs kernel RAM. We refuse silent loss. Completeness under overload is not promised.”

**Trap:** “We buffer unbounded in userspace so we never drop.” That stalls the drain or OOMs the node — then you drop anyway, silently, in the kernel.

**Follow-up they love:** “What if drops > 0?” Alert on `events_dropped` rate; optionally sample; do **not** grow the RingBuf without a measurement.

---

### 1.5 Correlation without a request ID

**Physics.** Kernel has no `X-Request-Id`. HTTP/1.1 on one fd is almost always write-then-read (client) or read-then-write (server). That structure **is** the ID.

**Code.** `agent/src/correlate.rs`:

- Key `SockKey { tgid, fd }`.
- `Pending { dir, ts_ns, prefix, at: Instant }`.
- `TIMEOUT = 60s` (aligned with rolling TUI window); `evict_stale` on each observe.
- Latency = response-half exit `ts_ns` − request-half exit `ts_ns` (`saturating_sub`).
- First half must `looks_like_request` (method prefix) and `ret >= 0`, `prefix_len > 0`.
- Second half must be **opposite** direction and `looks_like_response` (`HTTP/`).
- If mismatch but new chunk looks like a request, **replace** pending (don’t wedlock a bad half).
- TLS: `observe_client` → `client_only=true` → only Write→Read. Same-host OpenSSL client **and** server would otherwise double-count rates.

Whiteboard SM: [../architecture/correlation.md](../architecture/correlation.md)

**Say:** “I reconstruct exchanges from structure, then parse HTTP in userspace.”

**Trap table (memorize):**

| Case | Truth in this repo |
| --- | --- |
| HTTP/1.1 pipelining | Occasional mis-pair — documented, not solved |
| HTTP/2 | fd-only pairing **breaks** — Stretch S1, not shipped |
| No TCP reassembly | Only first HTTP-looking chunk per half |
| Mid-stream `recv` that isn’t method/`HTTP/` | Dropped; never joins |
| `sendmsg`/`writev` | Not in Phase 2 attach set |
| `ret < 0` | Not a half |
| Hung request | 60s eviction, no span |

**Counter-question to ask them:** “Would you rather mis-pair under pipelining or drop the connection?” We chose simple FSM + document.

---

### 1.6 HTTP parse, normalize, redact

**Physics.** BPF sees bytes. Dashboards need **METHOD + route + status class**, not `/users/1` vs `/users/2` as two worlds. Secrets in prefixes are a **compliance** bug if they leave the node.

**Code.** `agent/src/http.rs`:

- `httparse` on the two prefixes (`parse_exchange`).
- `normalize_path`: all-digit segment → `:id`; UUID → `:uuid`; long hex → `:hex` ([../design-notes/path-normalization.md](../design-notes/path-normalization.md)).
- Query string dropped from the route key (cardinality).
- `redact_headers`: `Authorization`, `Cookie`, `Set-Cookie` (export path, not a BPF hot path).
- Best-effort regex for tokens/card-like numbers is policy in `docs/security.md` — don’t overclaim a perfect DLP engine.
- **Metrics-only UI — never log prefixes.**

**Say:** “Parse in userspace so the verifier never sees a parser. Secrets may still sit in agent RAM — the process is the trust boundary.”

**Trap:** “We never see plaintext.” TLS uprobes **do**. That is why this is a privileged agent ([../security.md](../security.md)).  
**Trap:** “`:id` is OpenAPI.” It is a heuristic. `/v2` stays literal; a path segment `2` becomes `:id`.

**False merge example (say if they ask cardinality):** `/health/1` and `/health/2` both `/health/:id` — maybe wrong if `1` meant a version. We accepted that for MVP.

---

### 1.7 TLS without decrypting the wire

**Physics.** After `SSL_write`, TCP payload is ciphertext. Syscall capture cannot httparse HTTPS. After `SSL_read` returns, the app has plaintext. That return is the uprobe.

**Code.**

1. Uprobe `SSL_set_fd` (+ rfd/wfd) → map `SSL*` → fd.
2. `SSL_read` / `SSL_write` (+ `_ex`) enter-stash / exit-emit `TlsIo` (layout twin of `SockIo`, `kind=4`).
3. Soft-fail if `libssl.so.3` / `1.1` missing — **cleartext still works**.
4. Skip sock I/O on TLS-marked fds (no dual-plane merge).
5. `SSL_free` / fd cleanup so maps don’t leak (Phase 3 hygiene).

**Latency:** TLS half exit→exit. **Not** wire RTT, **not** `connect()` duration, **not** TLS handshake time unless those bytes happen to be your HTTP halves (they aren’t).

**Say:** “I intercept the library boundary, not the packet. OpenSSL only. rustls / Go crypto/tls / BoringSSL-first-class are out of scope.”

**Trap:** Dual-plane “TLS content + syscall timing” — explicitly **not** Milestone 3. Custom BIO (`SSL_set_fd` never called) → drop TLS event.  
**Trap:** “EINPROGRESS connect is the handshake.” Phase 1 `SockLatencyEvent::latency_ns` is **syscall enter→exit only**. Non-blocking connect returning `-EINPROGRESS` measures the syscall, not TCP or TLS completion (`common/src/lib.rs` header comment).

**Why client-only TLS pairing:** on one host, curl and nginx both use OpenSSL → two TlsIo streams for one HTTP exchange. Counting both **doubles** RPS. Cleartext `observe()` still allows server read→write because the demo needed both directions without that double-count pathology (or sock path doesn’t hit two libssl endpoints the same way). Be ready to say: “TLS path is client-only by construction.”

---

### 1.8 Peer binding (`SOCK_META`)

**Physics.** I/O events should not carry a full 4-tuple on every byte. Connect/accept already know the peer. Copying `daddr`/`dport` into every 288 B `SockIo` wastes RingBuf and verifier patience.

**Code.** BPF map `SOCK_META[(tgid,fd) as u64] → SockMeta { daddr_be, dport_be, flags, _pad }` (**8 B**). `FLAG_HAS_ADDR` bit0. Userspace joins onto `Exchange.peer` **after** emit. `peer_cache.rs` TTL so unknown dst is not forced mid-flight.

**Say:** “Metadata at connection time, join in userspace. Don’t bloat every SockIo.”

**Trap:** “The correlator looks up the BPF map.” It does not — comment in `correlate.rs` is explicit. Join is a later pipeline stage so the FSM stays testable with fake events.

**IPv4 only:** `AF_INET = 2`. IPv6 is a documented non-goal for the phases shipped.

---

### 1.9 Identity and service map

**Physics.** A PID is not a pod. cgroup v2 paths carry pod UID / container id. ClusterIP needs a pod-IP index because the **destination** of a client connect is often a Service IP, not a pod IP.

**Code.**

- `identity.rs`: `/proc/<tgid>/cgroup` + `comm`; cmdline basename if `comm` empty; cache TTL **30s**.
- `k8s_index.rs`: list pods (SA token), IP → `ns/name`; **soft-fail** (agent still emits `ip:port`).
- `service_map.rs`: in-RAM directed edges, cap **4096** → `_other`.

**Kind-in-Docker on WSL (measured 2026-08-15):**

```text
BPF tgid  ──────────►  WSL2 host PID namespace
/host/proc in DS  ──►  kind node container’s /proc
```

Those are **not the same namespace**. Result: `src=proc:unknown:<tgid>`. Names like `demo/frontend-…` on the **dst** come from **IP → k8s_index**, not from cgroup of the client.

**Say:** “On a real node, hostPID + host `/proc` match BPF PIDs. Kind-in-Docker is a nested PID lie — I labeled it instead of faking Grafana.”

**Trap:** Showing scrape full of `/readyz`, `/livez`, Docker `/_ping`, `/v1.54/…` as “the microservices demo.” Demo series: `GET /` → ClusterIP `10.96.28.98:8080` **count=99**; dst `demo/frontend-…` **count=99**.

**Why `hostPID: true`:** BPF always sees host-style PIDs for host-networked syscalls. If the agent’s `/proc` is the container ns, lookup misses. Cost: kube-proxy and Docker API traffic enter the agent. **Userspace allowlist is not shipped** — say so.

---

### 1.10 Cumulative OTLP histograms

**Physics.** Prometheus scrape is a **counter/histogram that only goes up** until process restart. Gauge-per-sample “last latency” is a lie for `rate()` and heatmaps. OTLP histograms have:

- `count`, `sum`
- `bucketCounts[]` — **count in that bucket only** (exclusive of others)
- `explicitBounds[]` — upper bounds; +Inf is the extra bucket
- `aggregationTemporality`: `2` = cumulative **over time** for that instrument

Prometheus `histogram_quantile` uses **cumulative** `le` buckets. The **collector** converts OTLP per-bucket → Prometheus cumulative. If you send Prometheus-style running sums as OTLP `bucketCounts`, `sum(bucketCounts)` ≠ `count` → collector **400**.

**Code.** `metrics_registry.rs`:

- Bounds ms: `1, 2, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000` +Inf.
- Series key `(src, dst, method, route, status_class)`; cap **2048** → `_other`.
- `record` places **one** increment in the first matching bound (per-bucket internally).
- `export.rs` POST `{OTEL_EXPORTER_OTLP_ENDPOINT}/v1/metrics` (kind: `http://otel-collector.observability.svc:4318`).
- Self-metrics: `events_dropped`, `otlp_dropped`, `otlp_flushes_ok`, `edge_count`.

**Bug we hit (kind, 2026-08-15):**

1. Empty `{"resourceMetrics":[]}` → collector **200** (so “endpoint works” was a false green).
2. Real payload: **unclosed JSON** (`EOF while parsing an object`) + **cumulative** `bucketCounts` → **400**.
3. Fix `to_otlp_json`; export warn includes response **body snippet**.
4. Then scrape: `http_client_duration_milliseconds_*`, `obsagent_otlp_dropped_total 0`.

**Say:** “I hand-rolled OTLP JSON to keep the Aya agent off the SDK. I learned the proto the hard way in kind.”

**Trap:** “Temporality 2 means bucketCounts are cumulative.” Temporality is **time**. Buckets in one payload are still per-bucket; their sum must equal `count`.

**Why not OpenTelemetry Rust SDK:** binary size, version hell next to Aya/nightly, and we only needed histograms + a few counters. Cost: we implemented JSON wrong once. That’s the trade.

---

### 1.11 DaemonSet privileges

**Code.** `deploy/k8s/daemonset.yaml`:

- `hostPID: true`
- caps: drop ALL, add `BPF`, `PERFMON`, `SYS_PTRACE`, `SYS_RESOURCE`
- `privileged: false` with comment: if cluster rejects distinct caps, `privileged: true`
- ro mounts: `/sys/kernel/btf`, `/sys/kernel/debug`, `/host/proc` ← host `/proc`
- env: `OBSAGENT_HEADLESS=1`, `OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector.observability.svc:4318`
- resources: request 50m/128Mi, limit 1 CPU / 512Mi
- SA `ebpf-obs-agent` for pod list

**Capability one-liners (SRE grill):**

| Cap | Why |
| --- | --- |
| `BPF` | load programs / maps (modern kernels) |
| `PERFMON` | perf-related attach (and some probe ops) |
| `SYS_PTRACE` | uprobes / inspect other processes |
| `SYS_RESOURCE` | locked mem / rlimit for BPF |

**Say:** “Least privilege first. Privileged is a kernel/cluster compatibility escape hatch.”

**Trap:** “We’re unprivileged sidecars.” `hostPID` + those caps is a **node agent**. Treat like Tetragon/Pixie class, not like a REST API pod.

**Apply order (ops scar):** `deploy/k8s/configmap.yaml` after smoke4 creates namespace `observability`. Premature apply fails. Kind load image **before** DS starts or you run an old binary (OTLP 400 ghost).

---

### 1.12 Overhead

**Target:** &lt;2% CPU under a *documented* load. **Not claimed as proven.**

| Phase | `ps` CPU (quick burst) | Notes |
| --- | --- | --- |
| 1 | 1.6% | connect-load; not host-normalized |
| 2 | 10.7% | HTTP burst; aspirational &lt;2% unmet |
| 3 | 3.5% | HTTPS quick; same caveat |

**Why `ps` is a weak meter:** short bursts, WSL2, no isolated cgroup of the agent vs kind, no packet rate normalization, no comparison vs uninstrumented baseline on a quiet node.

**Say:** “I measured with `ps` on WSL bursts and wrote the caveats in `docs/overhead.md`. I will not quote &lt;2% in an interview.”

**If they ask how you’d measure for real:** documented load (RPS, payload size, protocol mix); `cpuacct` or `perf stat` on the agent + a control node; drop rate vs CPU; never a single `ps` screenshot.

---

### 1.13 Clocks (they will mix these up)

| Clock | Use here |
| --- | --- |
| `bpf_ktime_get_ns` | `CLOCK_MONOTONIC` ns on events (`ts_ns`, syscall latency) |
| `Instant` | userspace eviction of pending FSM (60s) — **not** the HTTP latency |
| `SystemTime` | OTLP `startTimeUnixNano` / resource timestamps |

**HTTP latency is monotonic BPF timestamps**, not `Instant` and not wall clock. NTP cannot “explain” a 50 ms `/slow` miss; a wrong **half pairing** can.

---

### 1.14 Maps catalog (draw if they say “what’s in the kernel?”)

Speak only maps you can defend from `common` + `ebpf`:

| Map | Type (concept) | Key | Value | Why bounded |
| --- | --- | --- | --- | --- |
| PENDING | HashMap | enter id (syscall) | stashed ptr / meta | 8192 entries |
| SOCK_META | HashMap | `(tgid, fd)` u64 | 8 B peer | per live fd, cleaned |
| EVENTS | RingBuf | — | kind-tagged bytes | 256 KiB |
| drop_count | percpu/array counter | — | u64 | one number |
| SSL* → fd | HashMap | SSL pointer | fd | `SSL_free` / set_fd |

If you forget a name, say “I’d open `ebpf/src`” rather than invent a `LRU_HASH` of 4-tuples.

---

### 1.15 Decode and drain (userspace hot path)

**`decode.rs`:** inspect first byte → `EventKind::from_u8` → transmute/copy into `SockLatencyEvent` (48 B) or `SockIoEvent` (288 B). Wrong size / unknown kind → skip (don’t panic the drain).

**`main.rs` drain (conceptual):** Tokio task reads RingBuf → correlate → identity → parse → agg + registry. TLS branch `observe_client`. Cleartext `observe`.

**Headless vs TUI:** `OBSAGENT_HEADLESS=1` in the DaemonSet (Ratatui needs a tty). Local demos can use the TUI 60s table.

**Tests:** `wsl-run.sh test-agent` — **38 passed** (2026-08-13), including `records_into_correct_bucket` (parse + per-bucket).

---

### 1.16 Two aggregators (don’t confuse them)

| | `HttpAggregator` / hdrhistogram | `MetricsRegistry` |
| --- | --- | --- |
| Window | rolling **60s** for TUI | **process lifetime** cumulative |
| Percentiles | p50/p95/p99 rebuilt from hdr | Prometheus quantiles via buckets |
| Kind scrape | not what `:8889` shows | **this** is OTLP |
| Overflow | talkers table | 2048 series → `_other` |

**Say:** “The CLI percentiles and the Prometheus histogram are related but not the same instrument.”

---

### 1.17 Testing gates (what “green” means)

| Gate | What it proves | What it does not |
| --- | --- | --- |
| `test-agent` | FSM, parse, OTLP JSON shape | kernel attach |
| `smoke3` | TLS events + HTTP rows on WSL | k8s identity |
| `correctness3` | `/slow` p50 band vs 50 ms | production RPS |
| `smoke4` | local/kind wiring | without `KIND_E2E=1`, kind is SKIP |
| `KIND_E2E=1 smoke4` | DS + collector + demo | real-node cgroup identity, &lt;2% CPU |

**correctness3 extra:** `GET /slow count=5` (not 10) — repeat=5 must not double-count. `edges=1` locally.

---

### 1.18 Workspace / WSL (ops, but they ask)

```text
Windows:  C:\projects\eBPF-Observability-Agent
WSL:      /mnt/c/projects/eBPF-Observability-Agent
Target:   CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target
Runner:   wsl -d Ubuntu -u sahil -- python3 .../scripts/wsl_exec.py wsl-run.sh {build|test-agent|smoke3|...}
Root:     wsl -d Ubuntu -u root -- ... smoke3 | correctness3   # attach
```

**Never compile BPF on Windows.** Aya/`bpfel-unknown-none` is a Linux story.

---

## 2. File map (for “where is that?”)

| Piece | Path |
| --- | --- |
| ABI / kinds / sizes / comments on clocks | `common/src/lib.rs` |
| Probes + maps | `ebpf/src/` |
| Load / drain | `agent/src/main.rs` |
| Decode | `agent/src/decode.rs` |
| FSM | `agent/src/correlate.rs` |
| HTTP | `agent/src/http.rs` |
| Identity | `agent/src/identity.rs` |
| Pod IP | `agent/src/k8s_index.rs` |
| Edges | `agent/src/service_map.rs` |
| Peer TTL | `agent/src/peer_cache.rs` |
| 60s TUI agg | `agent/src/http_agg.rs`, `agg.rs` |
| OTLP | `agent/src/metrics_registry.rs`, `export.rs` |
| Deploy | `deploy/k8s/daemonset.yaml`, `deploy/Dockerfile` |
| Demo apps | `demos/microservices/k8s.yaml` |
| Gates | `scripts/wsl-run.sh`, `scripts/smoke-milestone*.sh`, `scripts/correctness-phase3.sh` |
| Security policy | `docs/security.md` |
| Correlation whiteboard | `docs/architecture/correlation.md` |

---

## 3. What is **not** in the repo (say no)

- HTTP/2 frame demux / gRPC `:path` (Stretch S1)
- `perf_event` + blazesym CPU merge onto slow requests (Stretch S2)
- Dual-plane TLS+syscall timing
- Graph DB / traces as Milestone 5 (traces **deferred**)
- IPv6 as first-class; Endpoints-based ClusterIP→pod as **primary** (IP index is pods)
- Production Grafana **in-cluster** (JSON exists; proven scrape is collector `:8889`)
- rustls / Go TLS / BoringSSL-first
- Sampling controller when drop_rate high
- Userspace allowlist for kube/docker cardinality
- Real verifier rejection log with SHAs

---

## 4. Competitive honesty (30 seconds)

| Product | What they optimize | What you are |
| --- | --- | --- |
| OpenTelemetry SDK | complete traces, context propagation | **no app change**; incomplete |
| Pixie | protocol decode + cluster UX + memory store | node-local metrics reconstruction |
| Cilium Hubble | datapath flows, L3/L4 (+ some L7 with Cilium) | syscall/uprobe HTTP/1.1 |
| Datadog NPM / APM | product + agents + backend | a **spike** with kind scrape |
| Tetragon | security policy / process | we share **privilege class**, not policy engine |

Never “we’re like X but open source.” Say “narrower completeness, honest limits.”

---

## 5. Phase map (if they ask “what did you build when?”)

| Phase | Milestone | One sentence |
| --- | --- | --- |
| 0 | load trivial program | Aya workspace, BTF, bpf-linker |
| 1 | connect/accept latency | RingBuf + pending + TUI TCP |
| 2 | HTTP/1.1 cleartext | prefix + FSM + httparse |
| 3 | OpenSSL HTTPS | uprobes, client-only TLS, redact policy |
| 4 | k8s | identity, service map, DS, OTLP (gauge-era) |
| 5 | hardening | cumulative histograms, self-metrics, kind e2e |
| S1/S2 | not started | HTTP/2; CPU stacks |

---

## 6. Failure-mode catalog (interview gold)

Walk **two** of these unprompted if they say “what breaks?”

1. **Pipelining** — two requests before a response → FSM may attach the wrong response.  
2. **HTTP/2** — N streams, one fd → guaranteed nonsense without stream IDs.  
3. **Kind PID** — `unknown` src; don’t “fix” by hardcoding demo names.  
4. **OpenSSL miss** — rustls app is invisible on TLS plane; may still show ciphertext sock I/O that **won’t parse**.  
5. **Custom BIO** — no `SSL_set_fd` → no fd → drop TLS event.  
6. **RingBuf full** — drops visible; p50 **biased low** if you only keep what fit (slow requests might be larger… actually we drop **events**, so pairing can also **break** — say “drops corrupt completeness, not just count”).  
7. **OTLP 400** — metrics vanish; `otlp_dropped` should move; empty POST looked healthy.  
8. **Cardinality** — hostPID + Docker API → series explosion; cap → `_other` hides it.  
9. **EINPROGRESS** — Phase 1 connect latency ≠ handshake.  
10. **Partial reads** — prefix isn’t `GET ` / `HTTP/` → half ignored.

---

## 7. Mini quizzes (cover the page, answer out loud)

1. Why is `SockMeta` 8 bytes?  
2. Why is `TlsIo` a type alias of `SockIo`?  
3. Why `observe_client` for TLS?  
4. Why first byte of RingBuf record?  
5. Why `aggregationTemporality=2` still has per-bucket `bucketCounts`?  
6. Why didn’t empty OTLP JSON catch the 400?  
7. Why `/host/proc` + `hostPID`?  
8. Why not parse HTTP in BPF?  
9. What is HTTP latency in one formula?  
10. Name one attach we **don’t** have (`writev`).

Answers: (1) don’t bloat I/O events (2) same correlator (3) double-count (4) demux 48 vs 288 B (5) time vs bucket (6) empty 200 (7) PID match (8) verifier (9) resp exit − req exit (10) `sendmsg`/`writev` / HTTP/2 / rustls.

---

## 8. ABI field-by-field (if they open `common/src/lib.rs`)

### 8.1 Constants you should recite

| Name | Value | Why |
| --- | --- | --- |
| `EVENTS_RINGBUF_BYTES` | 256 * 1024 | power of two; kernel RingBuf rule |
| `PENDING_MAP_ENTRIES` | 8192 | bound enter-stash; in-flight syscalls |
| `AF_INET` | 2 | IPv4 only this project |
| `SOCK_IO_PREFIX_LEN` | 256 | httparse request-line + some headers |
| `SOCK_LATENCY_EVENT_SIZE` | 48 | connect/accept record |
| `SOCK_IO_EVENT_SIZE` | 288 | 32 B header-ish + 256 prefix (layout frozen) |

288 = kind/dir/prefix_len/fd/pid/tgid/ret/ts + 256 prefix + padding as in the struct. If they quibble the exact pad, say “I’d print `size_of` — the ABI is frozen; don’t add `fd` to `SockLatencyEvent` (Phase 2 Q6).”

### 8.2 `EventKind` demux

RingBuf is a byte pipe. Userspace **must** know the size of the next record. Convention: **byte 0** is kind.

- `1 Connect` / `2 Accept` → read 48 B `SockLatencyEvent`
- `3 SockIo` / `4 TlsIo` → read 288 B `SockIoEvent` (TlsIo is a type alias)

Unknown kind: skip / increment a decode-error counter if you have one; **do not panic** the drain.

### 8.3 `SockLatencyEvent` (Phase 1)

| Field | Meaning in an interview |
| --- | --- |
| `kind` | Connect vs Accept |
| `pid` / `tgid` | thread vs process — HTTP later uses tgid |
| `ret` | syscall return; negative errno |
| `latency_ns` | **enter→exit of this syscall only** |
| `ts_ns` | exit time, monotonic |
| `daddr_be` / `dport_be` | peer; network byte order |

**Say clearly:** this latency is **not** HTTP p50. Mixing them is the #1 whiteboard fail after “we decrypt TLS.”

`EINPROGRESS`: `connect` returns immediately; `latency_ns` is tiny; TCP handshake is **later**. Documented in the crate docs.

### 8.4 `SockIoEvent` / `TlsIoEvent`

| Field | Meaning |
| --- | --- |
| `dir` | Read=1 Write=2 |
| `prefix_len` | `min(max(ret,0), 256)` on success else 0 |
| `fd` | process fd — **FSM key with tgid** |
| `ret` | byte count or `-errno` |
| `ts_ns` | **this half’s exit** — HTTP latency uses two of these |
| `prefix` | raw bytes; may contain secrets |

`TlsIo` is the same layout so `Correlator` does not care which plane produced the event. Plane is distinguished by **which function** you call: `observe` vs `observe_client`, and by skip-sock-on-TLS-fd in the drain.

### 8.5 `SockMeta`

8 bytes: `daddr_be`, `dport_be`, `flags`, pad. `FLAG_HAS_ADDR = 1`. Replaces a presence-only `SOCK_FDS` u8 map from an earlier design. Key is `(tgid, fd)` packed `u64`.

**Interview sentence:** “Connection-time metadata, join after the FSM, keep I/O events skinny.”

---

## 9. Correlator walk (`observe_inner`) — speak as if at the file

You do not need to memorize line numbers. You need the **cases**.

**Every call:** `evict_stale(now)` — drop pending older than 60s (`Instant`, not BPF time).

**Parse dir** from `ev.dir`. Copy `prefix[..prefix_len]`.

**Case A — no pending for `(tgid,fd)`:**

- Reject if `ret < 0` or empty prefix or not `looks_like_request` (method + space).
- If `client_only` and dir is not Write → reject (TLS server read ignored).
- Insert `Pending { dir, ts_ns, prefix, at: now }`. Return `None`.

**Case B — pending exists:**

- Opposite? Cleartext: Write↔Read either way. TLS client_only: **only** pending Write + current Read.
- Also need `ret >= 0` and `looks_like_response` (`HTTP/`).
- If yes → `Exchange` with `t_start_ns = pending.ts_ns`, `t_end_ns = ev.ts_ns`, `peer: None`.
- If no, but current chunk looks like a **new request** (and TLS still Write) → **replace** pending. Return `None`.
- Else drop this event, keep or lose pending per the replace rules (see code: failed match may replace).

**`looks_like_request`:** GET/POST/PUT/HEAD/DELETE/OPTIONS/PATCH + space. Not CONNECT, not TRACE — if they ask, say “MVP method list.”

**`looks_like_response`:** prefix starts with `HTTP/`. Not a status parser yet — `httparse` does that later.

**Tests in the same file:** client_only ignores a leading Read; Write then Read emits; timeout evicts. You can say “the FSM is unit-tested without a kernel.”

---

## 10. Drain pipeline (userspace, one HTTP exchange)

Number these on the board if they say “after RingBuf, then what?”

1. **Read** record bytes from RingBuf (Aya async).  
2. **Decode** kind + struct.  
3. **Connect/Accept:** update peer maps / Phase 1 TCP agg (optional TUI). Fill mental model of `SOCK_META`.  
4. **SockIo:** if fd is TLS-marked → **skip** (no dual-plane). Else `correlator.observe`.  
5. **TlsIo:** `correlator.observe_client`.  
6. **On Exchange:** join `peer` from `SOCK_META` / `peer_cache`.  
7. **Identity:** `identity.rs` for src label (`proc:…` or pod).  
8. **k8s_index:** maybe rewrite dst IP → `ns/name`.  
9. **parse_exchange:** method, path, status.  
10. **normalize_path** for route key.  
11. **redact_headers** if anything might leave the box (OTLP attributes — prefer not to send raw headers at all).  
12. **http_agg** 60s TUI.  
13. **service_map** edge++.  
14. **metrics_registry.record** (ns → ms, one bucket++).  
15. **export** later: snapshot JSON POST.

If they ask where you’d log the prefix: **you wouldn’t.** Step 11 is defense in depth; the UI is metrics-only.

---

## 11. HTTP parse details (`http.rs`)

**`parse_exchange`:** httparse request on `req_prefix`, response on `resp_prefix`. Need method, path, status. Incomplete headers → `None` (no metric), not a panic.

**`normalize_path` (segment-wise):**

| Segment | Becomes |
| --- | --- |
| all digits | `:id` |
| UUID 8-4-4-4-12 | `:uuid` |
| long hex ≥16 | `:hex` |
| else | literal |

Examples: `/user/123` → `/user/:id`. `/orders/<uuid>/items` → `/orders/:uuid/items`. Query string **dropped** from the key.

**False merges:** `/health/1` vs versioned `/health/1` meaning v1. `/v2` stays `v2` because of the letter. A lone `2` becomes `:id`.

**`redact_headers`:** in-place overwrite of Authorization, Cookie, Set-Cookie values. Export-only, not the correlator hot path (Phase 3 Q11). In-kernel redaction was rejected as too hot/fragile.

---

## 12. Metrics registry details

**`LATENCY_BOUNDS_MS`:** 1, 2, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000, then +Inf.

**`record`:** convert ns→ms; `count++`; `sum_ms +=`; 5xx → `errors++`; find **first** bound `ms <= bound` and increment **that** bucket only; else +Inf.

**Overflow:** if `series.len() == 2048` and key is new → fold into `_other` (`SeriesKey::other()`), `overflow_count++`.

**Self-metrics** (scraped as gauges/counters in OTLP): `events_dropped` (from BPF), `otlp_dropped`, `otlp_flushes_ok`, `edge_count`.

**JSON scar:** close the root object; emit per-bucket arrays; `sum(bucketCounts) == count`. Collector example they can remember: `"count":"2","bucketCounts":["1","1"]`.

**Empty POST:** `{"resourceMetrics":[]}` was **200** — so a probe that sends empty is a **false health check**.

---

## 13. Identity algorithms (enough to talk, not to recite regex)

**cgroup v2** paths often look like `0::/kubepods.slice/kubepods-burstable.slice/kubepods-burstable-pod<UID>.slice/cri-containerd-<id>.scope` (shape varies by runtime: containerd vs docker vs crio). You extract pod UID / container id **best-effort**. Cache 30s because `/proc` reads on every event would dominate.

**kind-in-Docker:** this algorithm never gets a chance if `/host/proc/<tgid>/cgroup` is the **wrong tgid**. That’s the measured `unknown`.

**k8s_index:** list Pods, every `pod.status.podIP` → `namespace/name`. ClusterIP is **not** a pod IP. Frontend name on dst can still appear if the **peer** of the connect was a pod IP or if something in the path used pod IP. Scrape also showed ClusterIP `10.96.28.98:8080` as dst — **that is the honest client connect() target**.

**comm vs cmdline:** `comm` is 16 bytes truncated. Empty → basename of cmdline. Still not a pod name.

---

## 14. Attach set (say “this is what we hook”)

**Syscalls (tracepoints):** connect enter/exit; accept4 enter/exit; read/write/recvfrom/sendto enter (stash) / exit (emit).

**OpenSSL (uprobes):** SSL_set_fd (+ rfd/wfd); SSL_read/SSL_write; SSL_read_ex/SSL_write_ex; SSL_free (cleanup).

**Not hooked:** writev, readv, sendmsg, recvmsg, connectat, io_uring, kTLS, rustls, Go TLS, Windows.

If they say “nginx uses writev”: “Then we can miss the response half. Documented. Stretch attach.”

---

## 15. Explain to a backend engineer (no kernel words)

“Imagine every HTTP client as a socket integer in a process. I watch the OS copy bytes in and out. I assume a client writes a request then reads a response. I parse those bytes like you’d parse a test fixture. I histogram the time between those two copies. For HTTPS the OS copies ciphertext, so I watch OpenSSL instead. I never ask you to add a library. I need root-ish on the node, and I will be wrong on HTTP/2.”

Then if they still want kernel: open §1.2.

---

## 16. Explain to a kernel engineer (no product words)

“CO-RE Aya, tracepoints not kprobe-on-tcp_sendmsg, RingBuf with reserve-fail counter, 8 B sock meta map, uprobes on libssl, userspace FSM, no parser in BPF, BTF required, hostPID DaemonSet, IPv4.”

They will hunt verifier logs. Offer classes + OTLP scar.

---

## 17. Data volumes (back-of-envelope, for “is 256 KiB enough?”)

Worst case all records are 288 B I/O events: 256 KiB / 288 ≈ **910 events** in the buffer. At 100k events/s that’s ~9 ms of slack. Sustained above drain rate → drops. Prefix 256 B is the tax; shrinking prefix increases slack and decreases httparse success. That’s the product knob — we froze 256.

Connect events 48 B are cheaper. HTTP load is I/O-dominated (Phase 2 10.7% `ps` burst — not a proof, but directionally “I/O path is hotter than connect-only”).

---

## 18. Locked product decisions (don’t reopen in an interview)

These were phase “Q-locks.” Changing them in the conversation makes you look unfocused.

1. Drop + counter, not unbounded userspace queue.  
2. Parse HTTP in userspace.  
3. Metrics-only UI; never log prefixes.  
4. No dual-plane TLS+syscall merge in M3.  
5. TLS correlator client-only.  
6. OTLP cumulative histograms (P5), not gauge-per-sample.  
7. Caps first, privileged fallback.  
8. Traces deferred.

You **may** reopen them as “what I’d change with evidence.”

---

## 19. Full quiz (write answers on paper)

11. Phase 1 vs Phase 2 latency.  
12. Why TlsIo layout twin.  
13. Why skip sock I/O on TLS fd.  
14. Why `Instant` vs `ts_ns`.  
15. Why series cap 2048.  
16. Why edge cap 4096.  
17. Why identity cache 30s.  
18. Why FSM timeout 60s.  
19. What FLAG_HAS_ADDR does.  
20. Why bpf-linker tarball.  
21. Why OBSAGENT_HEADLESS.  
22. Why hostPath `/proc` → `/host/proc`.  
23. Why empty OTLP 200 is a trap.  
24. Name two OpenSSL attach points besides read/write.  
25. What Stretch S2 is.

Answers: (11) syscall vs HTTP halves (12) one FSM (13) dual-plane forbidden (14) eviction vs latency (15) cardinality (16) same (17) /proc cost (18) hung + TUI align (19) peer valid (20) Docker build (21) no TTY (22) container vs host proc (23) false green (24) set_fd, free, _ex (25) CPU stacks + blazesym.

---

## 20. Reading order the night before an onsite

1. This file §0, §0b, §1.5, §1.7, §1.10, §1.9 (kind PID).  
2. `docs/architecture/correlation.md` SM diagram.  
3. SCRIPT §1–§2 and cheat card.  
4. RESUME never-claim list.  
5. Skim `correlate.rs` cases A/B.  
6. Skim `metrics_registry.rs` `record` + `to_otlp_json` if you have time.

Do **not** cram PROJECT-DEEP-DIVE the morning of — it is for implementation, not speaking.

---

## 21. Interview question index (lookup, don’t read linearly)

Use this when a friend quizzes you. Jump to the section, then say the two-line answer.

| They ask | Section | Two-line answer |
| --- | --- | --- |
| What did you build? | §0, SCRIPT §1 | Aya agent, no SDK, HTTP/1.1 (+ OpenSSL HTTPS) → OTLP histograms. |
| Draw it | §0, §10 | Probes → maps/RingBuf → decode → FSM → httparse → registry → collector. |
| Why eBPF? | §1.1 | Node-wide, no deploy in apps. Cost: privilege + reconstruction. |
| Why not SDK? | §1.1 | Adoption. Completeness is worse than a good SDK. |
| Verifier? | §1.2 | Dumb BPF; classes not a fake log. Scar = OTLP 400. |
| CO-RE / BTF? | §1.3 | Relocate off host BTF. Image is not a kernel. |
| RingBuf full? | §1.4 | Drop + counter. Completeness not promised. |
| No request ID? | §1.5, §9 | `(tgid,fd)` write/read pairing. |
| Latency definition? | §1.5, §8.3–8.4 | HTTP: resp I/O exit − req I/O exit. Connect syscall latency is different. |
| HTTP/2? | §1.5, §3 | Breaks fd FSM. Stretch S1. Not shipped. |
| Pipelining? | §6 | Mis-pair documented. |
| Parse where? | §1.6, §11 | Userspace httparse; normalize; redact. |
| Secrets? | §1.6, security.md | Metrics-only; redact; still in RAM. |
| Decrypt TLS? | §1.7 | No. OpenSSL uprobe. |
| rustls / Go? | §1.7 | Out of scope. |
| Dual-plane? | §1.7 | Not M3. Skip sock I/O on TLS fds. |
| Client-only TLS? | §1.7, §9 | Same-host double-count. |
| EINPROGRESS? | §8.3 | Syscall time ≠ handshake. |
| SOCK_META? | §1.8, §8.5 | 8 B peer at connect; join after FSM. |
| Pod names? | §1.9, §13 | cgroup + IP index; kind unknown src. |
| hostPID? | §1.9, §1.11 | BPF tgid vs container `/proc`. |
| OTLP 400? | §1.10, §12 | Unclosed JSON + cumulative bucketCounts. |
| Temporality 2? | §1.10 | Time-cumulative series; buckets still per-bucket. |
| Empty JSON 200? | §1.10 | False health check. |
| DaemonSet? | §1.11 | Caps, hostPID, BTF, `/host/proc`. |
| Overhead? | §1.12 | Target &lt;2%; not proven; P2 ~10.7% `ps`. |
| Clocks? | §1.13 | BPF monotonic for HTTP; Instant for eviction. |
| Maps? | §1.14 | PENDING, SOCK_META, RingBuf, drops, SSL*→fd. |
| Two aggregators? | §1.16 | 60s hdr TUI vs lifetime OTLP histogram. |
| Gates? | §1.17 | test-agent 38; correctness3 p50; KIND_E2E smoke4. |
| WSL? | §1.18 | Compile BPF only in Ubuntu WSL2. |
| Pixie? | §4 | Different completeness. Don’t analogize. |
| Traces? | §3 | Deferred. |
| Next? | SCRIPT §11 | HTTP/2 or allowlist or real node (pick two). |
| Numbers? | RESUME §7 | 50.8 ms; GET / 99; drops 0 that scrape. |
| Never say? | RESUME §6 | Pixie, &lt;2%, all pods, all TLS, fake verifier. |

---

## 22. Concept → file → function (grill cheat)

| Concept | File | Symbol / idea |
| --- | --- | --- |
| ABI | `common/src/lib.rs` | `EventKind`, sizes, `SockMeta` |
| Drop policy | `ebpf/` + export | `ringbuf_reserve` fail → drop_count |
| FSM | `agent/src/correlate.rs` | `observe`, `observe_client`, `TIMEOUT` |
| HTTP | `agent/src/http.rs` | `parse_exchange`, `normalize_path`, `redact_headers` |
| Histograms | `agent/src/metrics_registry.rs` | `record`, `to_otlp_json`, `LATENCY_BOUNDS_MS` |
| POST | `agent/src/export.rs` | async flush, body snippet on 400 |
| Identity | `agent/src/identity.rs` | cgroup, comm, 30s cache |
| K8s names | `agent/src/k8s_index.rs` | IP → ns/name, soft-fail |
| Edges | `agent/src/service_map.rs` | cap 4096 |
| Peer TTL | `agent/src/peer_cache.rs` | join dst |
| DS | `deploy/k8s/daemonset.yaml` | caps, hostPID, mounts |
| Correctness | `scripts/correctness-phase3.sh` | `/slow` band |

If you cannot fill this table from memory, drill it before more Q&A.

---

## 23. Spoken “physics” one-liners (carve these)

- Kernel cannot sleep on the agent.  
- Verifier cannot check an HTTP parser.  
- Wire cannot httparse ciphertext.  
- Kind-in-Docker cannot match WSL PIDs to node `/proc`.  
- Prometheus cannot `rate()` a gauge-per-sample honestly.  
- OTLP `bucketCounts` are not `le`.  
- Fd is not a stream id.  
- tgid is not a pod.  
- `connect()` latency is not HTTP latency.  
- Empty collector 200 is not a working exporter.

Each one-liner is a whole interview if they pull the thread. You already have the thread in §1.
