# eBPF Observability Agent — resume and interview story

**Use when:** résumé, LinkedIn, “tell me about a systems project,” SRE / platform / kernel-adjacent screens.  
**Companion:** [CONCEPTS_GRILL.md](CONCEPTS_GRILL.md) · [SCRIPT_AND_QNA.md](SCRIPT_AND_QNA.md) · [RUST_GRILL.md](RUST_GRILL.md)  
**Evidence:** [../testing/phase-5.tdd.md](../testing/phase-5.tdd.md) · [../handoff/SESSION-2026-08-15-kind-e2e.md](../handoff/SESSION-2026-08-15-kind-e2e.md)

This is a **spike + working demo**, not a sold observability product. Atlas (LLM gateway) is a **different repo** — do not mix numbers, clocks, or dashboards.

**Rule of three claims:** in any screen, pick at most three numbers. Recommended default: **(1)** `/slow` p50 ≈ 50.7 ms vs 50 ms **(2)** claim lock **10/10** on k3s **(3)** pin-load overhead **~87% of one core** (never say &lt;2%). Drop counters / OTLP 400 scar are optional color.

---

## 1. Positioning matrix (read before you paste bullets)

| They’re hiring | Lead with | This repo’s role | Risk if you lead eBPF |
| --- | --- | --- | --- |
| SRE / observability / platform | **This agent** | #1 | Must not claim Cilium/Pixie/Datadog parity |
| Systems / Rust / kernel-adjacent | **This agent** | #1 | Verifier stories must be *classes*, not fiction |
| Kubernetes platform | DaemonSet + identity **limits** | #1 if they care about node agents | Overclaiming kind pod names |
| Backend (generic) | Correlation + HTTP parse + OTLP | Supporting | Thin if they only want CRUD |
| Security / detection | TLS uprobe *tradeoff* | Supporting | You are not a malware sandbox |
| Networking (Cilium/CNI) | Honest “not Hubble” | Weak #2 | They will grill XDP; you didn’t ship it |
| ML / LLM | Don’t lead this | Mention only if they ask “other projects” | Wrong stack |
| Frontend | Don’t lead this | Skip | |

**Honest scores (this repo as a *product*):**

| Lens | Score | Why |
| --- | --- | --- |
| Production multi-cluster eBPF APM | ~3/10 | one k3s node, OpenSSL-only TLS, no multi-cluster |
| Honest node-local reconstruction + k3s scrape | ~8.5/10 | VISION-95 claim lock **10/10**; overhead honest |
| Interview spike “built something with Aya” | ~8.5/10 | if you can whiteboard the FSM + overhead |
| “I am an eBPF engineer like Isovalent” | ~5/10 | syscall/uprobe plane, not XDP/Hubble; verifier log now has 2 real pastes |
| “I can talk OTLP/Prometheus without lying” | ~8/10 | you ate a real 400 |
| Rust language (after drilling [RUST_GRILL.md](RUST_GRILL.md)) | ~4–5/10 language, ~7/10 *this repo* | ownership, no_std, Tokio split, small unsafe — not a language lawyer |

**Rule:** cite claim-lock evidence and **~87% of one core**. Do **not** cite &lt;2% CPU. Do **not** claim Go/rustls TLS or multi-cluster.

**Leveling (internal, don’t say the number):** this reads as a **strong intern / junior–mid systems project** if you own the limits; it reads as **junior theater** if you say Pixie. Senior signal is the **OTLP 400 + kind PID honesty**, not the word eBPF.

---

## 2. Recruiter one-liners (pick **one** per conversation)

**Safe / systems:**  
Built a Rust + Aya eBPF agent that reconstructs HTTP/gRPC latency and a named k3s service map from syscalls + OpenSSL uprobes — no app SDK — OTLP export; pin-load overhead **~87% of one core**.

**Safe / SRE:**  
Shipped a privileged DaemonSet that correlates `(tgid, fd)` into HTTP/gRPC exchanges, samples under overload, joins CPU stacks to slow spans, and exports OTLP — measured **~87% of one core** under Vision-95 pin load (not &lt;2%).

**Safe / Kubernetes:**  
Ran an eBPF observability DaemonSet on WSL2 k3s: hostPID, BPF/PERFMON/ptrace caps, cgroup-id → pod identity, named `frontend → api` edge proven.

**Safe / Rust:**  
Three-crate Aya workspace: `no_std` ABI, `bpfel` programs, Tokio agent. Bounded RingBuf with drop counters; HTTP/h2 parse and OTLP JSON in userspace; PerCpuArray after real stack-limit rejects.

**Safe / security-adjacent (careful):**  
Built a node agent that intercepts OpenSSL at the library boundary (not the wire), with metrics-only UI and header redaction. Treat it as a trust-boundary process, not a toy sidecar.

**Unsafe (never):**  
Production-grade, zero-overhead / &lt;2% CPU, language-agnostic APM like Pixie/Cilium Hubble that identifies every Kubernetes pod and decrypts all TLS.

---

## 2b. Email / LinkedIn / GitHub blurb

**LinkedIn project line (≤300 chars):**  
Rust/Aya eBPF agent: HTTP/gRPC latency + named k3s service map, no SDK. OTLP traces/metrics; CPU profile join on slow spans. Pin-load overhead ~87% of one core (not &lt;2%). Claim lock 10/10.

**GitHub README subtitle:**  
Node-local HTTP/gRPC metrics from syscalls + OpenSSL uprobes. Not Pixie. Overhead: ~87% of one core under pin load.

**Cold email P.S. (2 sentences):**  
I built an eBPF observability agent in Rust (Aya) that reconstructs HTTP/gRPC latency and a live service map on k3s, with OTLP export. Happy to whiteboard why there’s no request ID, and why we claim ~87% of one core instead of under 2%.

---

## 3. Resume — this project (copy-paste variants)

### Title (pick one)

- eBPF HTTP/gRPC observability agent — Rust/Aya, service map, OTLP, ~87% of one core (measured)
- Node-local HTTP/gRPC metrics from eBPF (Aya) — no application SDK
- Privileged Kubernetes DaemonSet: reconstruct latency, named edges, export OTLP

### Full bullets (6) — default / SRE

- Built a **CO-RE Aya** agent (`common` ABI + `ebpf` + Tokio userspace) that captures bounded prefixes on TCP I/O and OpenSSL `SSL_read`/`SSL_write` — **no application SDK**.
- Reconstructed **HTTP/1.1 + HTTP/2/gRPC** exchanges with a `(tgid, fd)` FSM; dual-plane TLS content×wire join; handshake via `SSL_do_handshake`.
- Correctness gates: writev/split-header, gRPC/h2-TLS, dual-plane, profile join (`prof_hit=11/20`); `/slow` p50 ≈ **50.7 ms** vs 50 ms inject.
- Exported OTLP **metrics + sampled traces**; Grafana dashboards; overload path bumps sticky `sample_n` when RingBuf drops.
- Deployed as a Kubernetes **DaemonSet** on **WSL2 k3s**; **cgroup-id → pod** identity; proven named edge **`frontend → api`**.
- Pin-load `perf stat` (HTTP 500/s + gRPC ~200/s ×3): mean **~87% of one core** — claim that number, **not** &lt;2%. Verifier scar: 2× stack-limit pastes → PerCpuArray / small stack buffers.

### Tight (3 bullets)

- Rust/Aya eBPF: HTTP/gRPC + named k3s service map from syscalls/OpenSSL; no SDK.
- Claim lock 10/10: OTLP traces, overload sampling, CPU profile join on slow spans.
- Measured overhead **~87% of one core** under Vision-95 pin load (do not say &lt;2%).

### One line

Kernel-level HTTP/gRPC latency and a live service map without an SDK: Aya → correlate → OTLP, claim-locked on k3s at **~87% of one core** pin-load overhead.

### Variant — Rust / systems JD (6)

- Designed a frozen `#[repr(C)]` ABI (`EventKind` demux, 48 B latency vs 288 B I/O events, 8 B `SockMeta`) shared by `no_std` BPF and a Tokio agent.
- Kept BPF verifier-safe: stash pointers on syscall enter, bounded `bpf_probe_read` on exit, RingBuf 256 KiB, drop on `reserve` fail with a scraped counter.
- Implemented a testable HTTP correlator (`observe` / `observe_client`) with 60s eviction; TLS path is client-only to avoid double-counting same-host OpenSSL.
- Hand-rolled OTLP JSON histograms (`aggregationTemporality=2`, per-bucket `bucketCounts`) after a collector 400; unit test asserts parse + bucket placement.
- CO-RE via host BTF (`/sys/kernel/btf/vmlinux`); container image does not ship a kernel; bpf-linker is a prebuilt musl tarball in Docker.
- WSL2-only BPF builds (`CARGO_TARGET_DIR` cache); never compile `bpfel-unknown-none` on Windows.

### Variant — Kubernetes / platform JD (5)

- DaemonSet: `hostPID: true`, drop ALL caps then add BPF/PERFMON/SYS_PTRACE/SYS_RESOURCE; ro BTF, debugfs, `/host/proc`.
- Soft-fail pod IP index (ServiceAccount list pods); agent still emits `ip:port` if the API is down.
- In-RAM service map, 4096 edge cap → `_other`; series cap 2048.
- kind e2e (`KIND_E2E=1 smoke4`) rolled DS + otel-collector + demo `api`/`frontend`; scrape `:8889`.
- Wrote the nested-PID limitation: BPF tgid is WSL host; `/host/proc` is the kind node → `proc:unknown` sources.

### Variant — security-adjacent (4, do not lead with “we decrypt”)

- Privileged node agent: OpenSSL uprobes see plaintext at the library; metrics-only UI; **never log prefixes**.
- `redact_headers` strips Authorization / Cookie / Set-Cookie before OTLP.
- Documented trust boundary (`docs/security.md`): secrets can still exist in agent RAM.
- Explicit non-goals: rustls/Go TLS, in-kernel redaction, dual-plane timing merge.

### Variant — intern / new grad (3, more process)

- Phased delivery (0–5) with locked design questions, TDD gates (`smoke3`, `correctness3`, `smoke4`), and architecture notes for correlation and RingBuf backpressure.
- Correctness: injected 50 ms delay vs observed p50 ≈ 50.8 ms.
- Production-shaped demo: kind cluster, collector, Prometheus histogram — plus a postmortem of OTLP JSON 400.

### What **not** to put on a resume

- “&lt;2% overhead”
- “Production APM”
- “All TLS / all languages”
- “Identifies all pods”
- “HTTP/2 and gRPC”
- Verifier war stories with fake logs
- Atlas 5.5× HF cliff in the same bullet

---

## 4. Resume — job bullets (templates; fill YOUR numbers)

Do not invent. If you lack a number, qualitative + system fact only.

**If you also have Atlas / internal LLM work:** keep them **separate bullets and separate STAR stories**. Mixing “eBPF + vLLM 5.5×” in one line looks like two internships glued together.

**Template — “why two projects?”**  
Job/Atlas: product + measurement on GPUs. This repo: kernel/userspace boundary and k8s node agents. Same person, different physics.

---

## 5. Interview stories (STAR)

### 5.1 Default (90 seconds) — reconstruction

**Situation.** I wanted language-agnostic HTTP latency on a node without asking every team to ship an OpenTelemetry SDK.

**Task.** See connect/accept and request/response timing from the kernel; handle HTTPS without claiming to decrypt the wire; get something a Prometheus scraper can eat.

**Action.** Aya tracepoints + OpenSSL uprobes; RingBuf with drop counters; userspace correlator and parser; OTLP JSON histograms; DaemonSet + kind demo. When the collector returned 400, I fixed unclosed JSON and per-bucket `bucketCounts` (not Prometheus running sums).

**Result.** Local `/slow` p50 tracks 50 ms. Kind scrape showed the demo `GET /` histogram and zero drop counters. I can explain what kind **cannot** prove (pod identity, &lt;2% CPU).

### 5.2 Conflict / debugging (OTLP 400) — 60 seconds

**S.** Kind collector accepted empty metrics (200) but agent POSTs returned 400.  
**T.** Get `http_client_duration_milliseconds` on `:8889` without lying about gauges.  
**A.** Logged response body; found unclosed JSON and `bucketCounts` that were running sums (`sum ≠ count`). OTLP wants per-bucket; temporality 2 is **time**. Wrote a unit test; rebuilt image; `kind load`; rolled DS.  
**R.** POST 2xx; scrape histogram; `otlp_dropped` 0. Empty payload 200 taught me not to health-check with `{}`.

### 5.3 Honesty / kind PID — 45 seconds

**S.** Grafana-ish scrape showed `src=proc:unknown` while dst had `demo/frontend-…`.  
**T.** Don’t ship a fake identity story.  
**A.** Traced namespaces: BPF PIDs = WSL host; `/host/proc` = kind node. Dst names from **pod IP index**, not client cgroup.  
**R.** Documented in the handoff. Interview: I say the lie **before** they find it.

### 5.4 Design choice (RingBuf) — 45 seconds

**S.** Kernel cannot wait for userspace.  
**T.** Bound memory; don’t fail silent.  
**A.** 256 KiB RingBuf; `reserve` fail → `drop_count`; scrape as Prometheus counter; export async so drain isn’t coupled to HTTP.  
**R.** Kind scrape 0 drops that day; completeness still not an SLA.

### 5.5 Design choice (no request ID) — 45 seconds

**S.** Kernel has no W3C traceparent unless the app wrote it (SDK).  
**T.** Still emit per-route latency for HTTP/1.1.  
**A.** FSM on `(tgid, fd)`, write/read pairing, 60s timeout, httparse in userspace.  
**R.** Works for stop-and-wait HTTP/1.1; pipelining/HTTP/2 documented as breaks.

### 5.6 “Tell me about a mistake”

OTLP JSON. Don’t use a fake verifier failure. Secondary: applying ConfigMap before namespace existed; loading a stale image into kind.

### 5.7 “Tell me about a tradeoff”

TLS client-only pairing vs complete server-side HTTPS metrics on the same host. We chose **no double-count**. Server-side OpenSSL HTTPS on the TLS plane is a known hole unless we add attribution later.

### 5.8 “What would you do with 4 more weeks?”

Pick two: HTTP/2 stream IDs; userspace allowlist; real-node identity; overhead protocol. Not “rewrite in Go.” Not “multi-region.”

---

## 6. Never-claim list (print this)

| Phrase | Why it dies | Say instead |
| --- | --- | --- |
| “Like Pixie / Cilium Hubble / Datadog NPM” | Different product | “Node-local HTTP/1.1 reconstruction” |
| “&lt;2% CPU in production” | Burst `ps`; P2 ~10.7% | “Target, not proven; see overhead.md” |
| “We identify every pod on kind” | Nested PID ns | “`unknown` src; dst via IP index” |
| “We support all TLS” | OpenSSL `libssl` only | “OpenSSL uprobes; rustls/Go out of scope” |
| “HTTP/2 and gRPC” | Stretch S1 | “Not shipped; fd FSM would mis-pair” |
| “The verifier rejected X on commit Y” | log file empty | “Rejection classes; BPF kept dumb” |
| “Zero packet loss / complete traces” | RingBuf drops; no traces M5 | “Drops are a metric; traces deferred” |
| “I decrypted HTTPS on the wire” | uprobe at OpenSSL | “Library boundary, privileged agent” |
| “Production-grade identity” | kind demo | “Designed for hostPID nodes; kind lies” |
| “We parse HTTP in the kernel” | we don’t | “256 B prefix; httparse userspace” |
| “OTLP SDK” | hand-rolled JSON | “JSON POST /v1/metrics” |
| “IPv6 / all syscalls” | AF_INET; no writev | “IPv4; listed attach set” |
| “Senior Rust / I wrote a compiler” | this is a junior–mid *project* in Rust | “I can maintain this Aya workspace” |
| “Rust is fully memory-safe here” | decode/Pod/setrlimit/BPF helpers | “unsafe islands, length-checked decode” |
| “I don’t know Rust” (as the opener) | they stop the round | Pitch crates first; drill RUST_GRILL before the interview |

---

## 7. Numbers you may cite (dated)

| Claim | Number | Source | Caveat to attach |
| --- | --- | --- | --- |
| Injected delay vs p50 | 50 ms vs ≈50.82 ms | phase-5.tdd correctness3 | band max(±10 ms, ±10%); N=5 |
| `/slow` count | 5 (not 10) | same | double-count bug was fixed |
| Kind demo series | `GET /` count=99 | 2026-08-15 scrape | cumulative histogram count, not SLA |
| Dst named series | `demo/frontend-…` count=99 | same | IP index, not cgroup of src |
| Drops | events + OTLP = 0 | same scrape | not “never drops” |
| Histogram name | `http_client_duration_milliseconds` | collector `:8889` | |
| Tests | 38 passed | test-agent 2026-08-13 | userspace |
| Cluster | kind `obsagent`, k8s v1.33.1, kind v0.29 | handoff | |
| Kernel | `6.6.114.1-microsoft-standard-WSL2` + BTF | same | |
| RingBuf | 256 KiB | `common` | |
| Prefix | 256 B | `SOCK_IO_PREFIX_LEN` | |
| Series cap | 2048 | metrics_registry | |
| Edge cap | 4096 | service_map | |
| FSM timeout | 60 s | correlate.rs | |
| P1/P2/P3 `ps` CPU | 1.6% / 10.7% / 3.5% | overhead docs | **bursts; don’t lead** |

If they ask “is 99 the true request count?” — it is **that scrape’s cumulative histogram count** for that series, not a lifetime cluster SLA.

---

## 8. Company-shaped framing (same facts, different first sentence)

**Datadog / New Relic / Grafana / Elastic:**  
“I wanted to feel the kernel half of APM. I did not rebuild your backend. I reconstructed HTTP/1.1 into histograms and hit a real OTLP 400.”

**Cilium / Isovalent / networking:**  
“This is not XDP. Syscall + uprobe. I can still talk maps, verifier classes, and why Hubble is a different plane.”

**K8s platform (in-house):**  
“DaemonSet, caps vs privileged, BTF mounts, soft-fail kube index, cardinality under hostPID.”

**FAANG SRE:**  
“Failure modes and SLIs: `events_dropped`, `otlp_dropped`, p50 vs injected delay. Completeness is not promised.”

**Startup backend:**  
“No SDK adoption problem. Privileged agent as the cost. I’d filter kube noise next.”

**Security team:**  
“Uprobe plaintext is the point and the risk. Redact + metrics-only. I’m not claiming Tetragon.”

---

## 9. Recruiter screen Qs (30–45 min)

**What is eBPF in one sentence?**  
Programs the kernel safely (verifier) to run tiny hooks; I use it to observe syscalls and OpenSSL without changing apps.

**Is it production?**  
It’s a demo on kind plus WSL correctness gates. I treat it as a portfolio spike with honest gaps.

**Why Rust?**  
Aya’s CO-RE story; one language for ABI + agent; I still built BPF as dumb C-shaped layouts.

**Biggest risk if we hired you to productize this?**  
HTTP/2, TLS library coverage, identity on real nodes, overhead protocol, cardinality filters.

**Salary / 30 LPA talk:** this repo alone is not a 30 LPA proof. Combine with job impact + DSA if that’s their bar. Don’t argue with a recruiter using RingBuf sizes.

---

## 10. Portfolio hygiene

- README: pitch + limits + how to run in WSL.  
- Don’t screenshot Grafana full of Docker `/v1.54`. Crop `GET /` and the drop gauges.  
- Don’t commit `.env` / cluster creds.  
- Link `docs/interview/` in README only if you want interviewers to see the pack — optional; some people prefer not to show the script.

---

## 11. Weakness / gap script (they will ask)

**Technical gap I admit:** no real verifier rejection diary; Stretch HTTP/2 not started; overhead not host-normalized.

**Process gap:** kind-in-Docker was the wrong topology to prove cgroup identity — I should have planned a real VM node sooner.

**How I’m not faking seniority:** I write the failure in the handoff instead of renaming `unknown` to `frontend` in the exporter.

---

## 12. Cover letter paragraph (paste, then customize)

I built a Rust (Aya) eBPF agent that reconstructs HTTP/1.1 latency from kernel syscalls and OpenSSL uprobes without an application SDK. A correctness gate tracks an injected 50 ms delay at p50 ≈ 50.8 ms. I exported cumulative OTLP histograms and scraped them from a kind cluster after debugging a collector 400 caused by invalid JSON and Prometheus-style histogram buckets. I am explicit about limits: no HTTP/2, OpenSSL-only TLS, kind-in-Docker cannot prove pod identity, and sub-2% CPU is a target rather than a result. I am looking for SRE / platform / systems roles where those tradeoffs are the job.

---

## 13. Bullet workshop (same fact, three seniority reads)

**Junior (process):** “Implemented eBPF HTTP tracing in Rust with tests and a Kubernetes demo.”  
**Mid (mechanism):** “Reconstructed HTTP exchanges on `(pid, fd)` from syscall/uprobe events; exported Prometheus histograms via OTLP.”  
**Senior (limits):** “Shipped node-local HTTP metrics without SDKs; designed drop-visible backpressure and labeled kind PID mismatch instead of faking identity.”

Use **mid + one senior limit** on the résumé. All junior or all senior-theater both fail.

**Rewrite drill:** take any bullet and add **(a)** mechanism **(b)** evidence **(c)** limit. Example: “`(tgid,fd)` FSM (a); correctness3 p50 (b); HTTP/2 not claimed (c).”

---

## 14. What to put on GitHub vs résumé vs interview

| Surface | Include | Exclude |
| --- | --- | --- |
| GitHub README | how to run WSL, architecture diagram, limits | salary, “30 LPA”, interview scripts |
| Résumé | 3–6 bullets, 2 numbers max | RingBuf size, verifier classes |
| Screen | 30s script + one scar (OTLP 400) | entire CONCEPTS file |
| Onsite | whiteboard FSM + kind PID | Atlas GPU numbers |

---

## 15. Week-of-interview plan (this project)

**T-7:** Speak 30s and 2 min to a phone; record; cut filler.  
**T-5:** Whiteboard FSM from memory; check against correlation.md.  
**T-4:** OTLP bucket numeric example on paper.  
**T-3:** DaemonSet fields from memory (caps, hostPID, mounts, endpoint).  
**T-2:** Never-claim list out loud.  
**T-1:** Cheat card only. Sleep.  
**Day:** One number (p50 or 99), one scar (400), one limit (HTTP/2 or kind PID).

---

## 16. Behavioral bank (this repo only)

**Ownership:** Phases 0–5 with locked Qs; I didn’t skip the OTLP proto when gauges “looked fine.”  
**Curiosity:** Why empty JSON was 200; why dst had names and src didn’t.  
**Rigor:** correctness band max(±10 ms, ±10%); count=5 not 10.  
**Communication:** handoff doc for kind e2e — future me / interviewer can replay.  
**Disagreement:** (if you argued for dual-plane internally) you **lost** on purpose: M3 lock. Use that.  
**Failure to estimate:** kind-in-Docker as identity proof — wrong topology; I documented rather than hidden.

**Do not** use this repo for “led a team of 8” or “saved $2M.”

---

## 17. Recruiter objections — short replies

**“This isn’t on your résumé as a job.”**  
Correct. It’s a systems spike with measurable gates. My job is [X]. This shows kernel/userspace work.

**“We need production eBPF.”**  
I don’t claim production. I can ramp on your agent because I already hit OTLP, DaemonSet caps, and correlation failure modes.

**“We use Go / cilium/ebpf.”**  
Maps, verifier, RingBuf, CO-RE are the same physics. I used Aya; I can read C BPF.

**“We don’t allow privileged DaemonSets.”**  
Then this product shape is a hard sell; SDK APM is the path. I still understand the constraint.

**“Can you start on HTTP/2 next week?”**  
I can sketch stream-id keys; I have not shipped it. I won’t give a fake date.

---

## 18. One-page résumé block (ASCII, copy into LaTeX/GDocs)

```text
eBPF observability agent (Rust / Aya)                          2026
Personal systems project — node-local HTTP(S) metrics, no app SDK

• Syscall tracepoints + OpenSSL uprobes; 256 B prefixes; RingBuf drop counters
• (tgid, fd) HTTP/1.1 correlator; httparse; path normalize; header redact
• correctness3: /slow 50 ms delay → p50 ≈ 50.8 ms (gate band ±10 ms / 10%)
• OTLP cumulative histograms; kind scrape GET / count=99; event/OTLP drops 0
  (fixed collector 400: JSON + per-bucket counts)
• DaemonSet: BPF/PERFMON/ptrace, hostPID, BTF + /host/proc; kind-in-Docker
  PID mismatch labeled (src=proc:unknown) — not faked
```

---

## 19. LinkedIn “About” two sentences

I like systems where the kernel and userspace ABI is the product. Recently I built an eBPF HTTP metrics agent in Rust (Aya): no SDK, OpenSSL uprobes for HTTPS, OTLP histograms on kind, with documented limits.

---

## 20. Anti-portfolio (do not screenshot)

- TUI full of `unknown` without a caption.  
- Prometheus full of Docker API paths as “microservices.”  
- `ps` 1.6% as “<2% overhead.”  
- Grafana JSON that was never deployed.  
- Verifier empty log.  
- Mixing Atlas dashboard with this agent.

**Do screenshot:** correctness3 PASS line; `:8889` histogram for `GET /`; `otlp_dropped 0`; a note “kind nested PID.”
