# NEXT_PHASES.md — eBPF Observability Agent

Written 2026-11-03. Read-only review: no source, config, or git state was changed.

Tags used throughout:
- **[OBS]** observed in this repo (file/line given)
- **[RES]** found in web research (URL + date given; "unverified" where I could not confirm)
- **[INF]** my inference. Treat as hypothesis until you test it.

Time assumption: today is 2026-11-03, deadline is 2026-11-30, so 27 days. At 4–6 usable h/day that is ~110–160 h gross. Interview prep, applications, and other projects (Chronos) will eat a large share. I plan for **~85 h net on this repo** and give a ~50 h minimum path.

---

## 1. Project summary + honest assessment

**What it is [OBS].** A Rust + Aya eBPF agent (~11.8k LOC: `agent/` 7.6k, `ebpf/src/main.rs` 1.3k, `common/` 0.7k). It attaches syscall tracepoints (connect/accept/read/write/readv/writev/sendmsg/recvmsg) and OpenSSL uprobes (`SSL_read/SSL_write/SSL_do_handshake`). It sends fixed-size events (`SockIoEvent`, 288 B, 256 B payload prefix) over a RingBuf. Userspace reassembles split writes, pairs request/response per `(tgid, fd)` (HTTP/1.1) or `(tgid, fd, stream_id)` (hand-rolled HTTP/2 + HPACK), aggregates into HdrHistograms, builds a service map, and exports OTLP metrics and sampled traces to a collector → Prometheus → Grafana. A DaemonSet manifest and a k3s e2e exist. Docs claim "complete, claim-lock 10/10" (`docs/GOALS-AND-SUCCESS.md`).

**Genuinely strong [OBS]:**
- Real kernel↔userspace systems work, not a wrapper. Drop-and-count backpressure, adaptive 1/N connection sampling (`agent/src/sample.rs`), self-process exclusion, CO-RE socket reads.
- Honest measurement. `docs/overhead.md` records 87% of one core at 500 HTTP + ~200 gRPC rps and refuses the original "<2%" claim. This is rare and worth keeping.
- Ground-truth oracle exists: `testdata/` servers with injectable latency (`/slow` 50 ms → observed p50 50.7 ms). That is the seed of an eval.
- Evidence artifacts are checked in (`docs/handoff/artifacts/`).

**Weak / fragile [OBS]:**
1. **No CI.** `.github/` is absent. About 182 `#[test]` attributes exist (grep count; I did not run them). The "correctness gates" are shell scripts needing root + WSL2 + k3s (`scripts/correctness-*.sh`, `e2e-k3s.sh`). No one can verify the claims without your machine.
2. **No replay, no deterministic event log, no property/fuzz tests.** Hand-written HPACK/H2 parsers (`agent/src/h2/*.rs`, 1.5k lines) run inside a privileged process on untrusted bytes with unit tests only.
3. **The 87% is unexplained.** Hot path at `agent/src/main.rs:389-390` clones the `self_ids` HashSet and extends it with `bpf_ids` for **every ring-buffer event**, then ~20 `Arc<Mutex<…>>` are locked per event (`main.rs:194-252`). I have not profiled it. [CORRECTED 2026-11-03, see INVESTIGATION.md §5: the cloned set holds only the agent's own ids (`filter.rs:234-261`), so it is NOT a plausible main contributor. The checked-in log implies ~1.37 M instructions per request; cause unknown until profiled.] The `perf -p` methodology also excludes in-kernel BPF program time (the doc's own footnote admits it), and it ran on WSL2 only.
4. **No distributed tracing.** `trace_export.rs` says "one root span per exchange"; span/trace IDs are hashed from `(tgid, fd, stream_id, ts)`. `grep traceparent|tracestate|parent_span` over `agent/src` returns nothing. So `frontend → api` appears in the service map but never as a parent/child trace.
5. **Blind spots by design:** 256 B prefix, first iovec only, no io_uring/sendfile probes per `docs/architecture/PROBE_MAP.md` (I did not exhaustively verify the attach list), OpenSSL only (no Go `crypto/tls`, no rustls).
6. **Verifier log is staged.** `docs/verifier-rejection-log.md` is titled "real verifier rejections", but its own Entry 1 says "no natural reject paste survived… force a 600 B stack sink". Both entries are bpf-linker/LLVM stack-limit errors you induced deliberately, not kernel verifier rejections. An interviewer who asks "what did the verifier reject?" will find this in minutes.
7. **Docs volume looks agent-generated.** ~104 markdown files: 13 phase plans, 14 handoff session logs, 14 TDD docs, duplicate `ring-buffer-backpressure.md` in two folders, "Q8 / Q11 locked" decision codes referring to plan docs. The self-graded "Verdict: Yes" page is not evidence. There is no comparison against Beyla/OBI/Pixie anywhere.
8. Working tree not clean: `docs/architecture/PROJECT-DEEP-DIVE.md` modified, two untracked `scripts/tmp-*.sh`. `Cargo.lock` is tracked despite being listed in `.gitignore`.

**Bottom line [INF].** A competent, partly vibe-built APM-style eBPF agent with unusually honest numbers. As shipped it is a *different product category* from your spike (agent reliability). It proves systems depth, not agent evals/observability.

---

## 2. Alignment score vs core direction

**Primary spike (agent reliability = evals + observability): LOW as-is.**
**Core backend / systems depth: MEDIUM-HIGH.**
**Sandboxing leg: LOW as-is, but one probe set away from relevant.**

Justification. [OBS] Nothing in the repo knows what an LLM call, tool call, token, trajectory, or eval is. Its "evals" are latency gates against a known-latency HTTP server. It has no record/replay, no regression suite runnable in CI, and no failure taxonomy. What does transfer [INF]: the kernel→userspace pipeline, backpressure discipline, TLS-plaintext capture, process/cgroup identity, and the honest-measurement habit are the *same machinery* that agent-observability research systems use (AgentSight, below). The distance between "HTTP latency APM" and "zero-instrumentation agent flight recorder with a measured-accuracy eval" is about two probe families, a payload parser, and a replay harness. That makes the repo **conditionally HIGH** after the phases below, and a generic APM clone that duplicates a CNCF project (OBI) if you do nothing.

---

## 3. Research findings (key insights only)

### 3.1 Agent evals + observability

- **[RES] Anthropic, "Demystifying evals for AI agents", 2026-01-09** — https://anthropic.com/engineering/demystifying-evals-for-ai-agents
  - Start with 20–50 tasks drawn from real failures.
  - Prefer deterministic (code) graders; use model graders only where needed and calibrate against humans; give judges an "Unknown" escape hatch.
  - Grade outcomes, not exact paths.
  - pass@k vs pass^k: pass^k (all k trials succeed) is the right metric for consistency; 75% per-trial → ~42% pass^3.
  - Start every trial from a clean environment; shared state inflates scores.
  - Capability evals graduate into regression evals; read transcripts.
  - This is the closest thing to a primary source for how a serious lab frames it. **Your repo has none of these structures.**
- **[RES] Failure taxonomies / step-level diagnosis.**
  - MAST: 14 failure modes in 3 categories, κ=0.88 inter-annotator agreement — arXiv 2503.13657 (date inferred from ID: Mar 2025). A blog cites "1,642 traces"; the paper release says 200+ — unverified, check the paper.
  - AgentRx (Microsoft): finds the *first unrecoverable step* in 115 annotated failed trajectories — https://www.microsoft.com/en-us/research/blog/systematic-debugging-for-ai-agents-introducing-the-agentrx-framework (arXiv 2602.02475, ~Feb 2026 from ID).
  - AgentEval (DAG step-level error propagation) arXiv 2604.23581 — I only saw the title/snippet, not the paper.
- **[RES] LLM-as-judge pitfalls.**
  - Zheng et al. (MT-Bench), arXiv 2306.05685 (2023): position bias and verbosity bias are real; mitigation = swap order.
  - Wang et al., "Judging the Judges", arXiv 2406.07791: argues several biases are partly position effects.
  - Contrary, single-author preprint: Soumik, arXiv 2604.23178 (Apr 2026) claims position bias is now negligible on current models and style bias dominates. **Conflicting / thin: do not build a claim on it.**
  - My search found no dedicated *calibration* paper. Don't cite one.
- **[RES] Record/replay for agent regression testing.** "Chronicle: Cut-Point Replay for Regression Testing of LLM Agents", arXiv 2609.20625, 2026-09-17 — https://arxiv.org/abs/2609.20625
  - Records at non-deterministic boundaries; full replay makes zero model calls and is bit-stable over 20 reps; cut-point replay serves some boundaries from record and runs the rest live with new code.
  - Reported 23 µs/boundary recording overhead.
  - Caveat from the abstract: model boundaries are *simulated* in the benchmark (6 failures), and recordings can hold secrets.
  - This is almost exactly the Chronos (deterministic simulation) mindset applied to agents. It is a strong thematic bridge for you.
- **[RES] Zero-instrumentation agent observability exists.** AgentSight (Zheng et al.), arXiv 2508.02736v2, Aug 2025 — https://arxiv.org/html/2508.02736v2
  - Same technique as your repo: uprobes on OpenSSL `SSL_read/SSL_write` for LLM traffic, plus `sched_process_exec` + syscall kprobes for actions.
  - Correlates by process lineage, a 100–500 ms time window after an LLM response, and argument matching.
  - Reports 2.9% average overhead on Claude Code 1.0.62, ~6k lines Rust/C + 3k lines TS.
  - Evaluation is three case studies (prompt-injection exfil of `/etc/passwd`, a retry loop, multi-agent contention). From the summary I retrieved, there is **no precision/recall of the correlation itself and no stated limitations section**. Check the full text before relying on this.
  - **Implication [INF]: "capture LLM traffic with eBPF" is not novel. A measured-accuracy evaluation of the observer, plus deterministic replay, is the open space.**
- **[RES] OpenTelemetry is absorbing this.** OBI (OTel eBPF Instrumentation) v1.0.0 — https://opentelemetry.io/blog/2026/obi-v1-release/ (release date not stated on the page).
  - Covers many protocols, improved HTTP/2/gRPC stream context and trace-parent relationships.
  - Lists GenAI/MCP coverage (model providers, embeddings, vector retrieval, MCP); detailed extraction needs explicit payload-capture config.
  - Coralogix docs (https://coralogix.com/docs/ebpf-auto-instrumentation) say OBI covers Java, Node.js and generic Go TLS. **So your "OpenSSL only" scope is narrower than the CNCF tool; do not pitch the repo as a Beyla/OBI competitor.**
- **[RES] OTel GenAI semantic conventions — status conflicting.** Defined agent operations include `create_agent`, `invoke_agent`, `invoke_workflow`, `plan`, `execute_tool`. Most sources (TrueFoundry, Aug 2026: https://www.truefoundry.com/blog/opentelemetry-genai-semantic-conventions; Greptime, 2026-05-09: https://greptime.com/blogs/2026-05-09-opentelemetry-genai-semantic-conventions) say still *Development*. One source (https://businesstechnavigator.com/news/opentelemetry-genai-semantic-conventions-agents-2026, 2026-06-22) claims stabilization. Another says GenAI definitions moved to a separate repo in v1.42.0. I did **not** verify against the spec. Pin a version and check before coding attribute names.
- **[RES] Tooling landscape, thin and vendor-heavy.**
  - Langfuse acquired by ClickHouse, announced 2026-01-16 (https://siliconangle.com/2026/01/16/database-maker-clickhouse-raises-400m-acquires-ai-observability-startup-langfuse/).
  - Braintrust $80M Series B, Feb 2026, ICONIQ (via https://www.pulse.bot/ai/deals/braintrust-secures-80m-series-b-funding-led-by-iconiq-capital-46cbd875-7584-4028-a2aa-a02aee14fd8d — secondary source).
  - Arize funding sources conflict (a Series D and a Dynatrace acquisition appear in single unverified listings).
  - Comparison posts say Langfuse requires extra work for production eval pipelines and multi-step causal analysis is manual. These are competitors' blogs. Directional only.

### 3.2 Sandboxing / secure execution

- **[RES] Fly.io, "Firecracker vs gVisor", 2026-09-09** — https://fly.io/learn/firecracker-vs-gvisor/
  - Firecracker = separate guest kernel in KVM microVM (<125 ms boot, <5 MiB overhead per project figures).
  - gVisor = userspace kernel "Sentry"; every syscall pays a cost; implements a subset of the Linux ABI (io_uring off by default).
  - Firecracker needs KVM; gVisor works without it.
  - Vendor content; the numbers are Firecracker project claims, not independent benchmarks.
- **[RES] Containment principles (secondary sources, weak).** Simon Willison's "lethal trifecta" (private data + untrusted content + external comms); remove a capability rather than rely on model resistance; sandboxes do not stop leakage of credentials the agent legitimately holds; use a credential broker + per-tool egress allowlist; CaMeL (DeepMind) is research-stage with no production implementation per a Sophos write-up (search snippet only; I did not read the CaMeL paper). Claude Code's sandbox reportedly = bubblewrap/Seatbelt + out-of-sandbox network proxy; I only saw third-party write-ups (e.g. https://medium.com/@Koukyosyumei/how-claude-code-and-codex-sandbox-untrusted-code-ba39b493046a) — **verify against Anthropic's own post before citing**.
- **[INF] Relevance to you.** An eBPF observer is the *audit* half of a sandbox (what actually happened vs what policy allowed). That is a coherent adjacent leg without building a microVM runtime.

### 3.3 Backend foundations

- **[RES] Durable execution for agents.** Pydantic AI officially wraps agents for Temporal, DBOS, Prefect, Restate (https://pydantic.dev/docs/ai/integrations/durable_execution/overview/). Temporal's OpenAI Agents integration runs each model call as an Activity not repeated on replay. Replay prevents re-running journaled steps but **external side effects still need idempotency keys** (LangGraph docs guidance, secondhand via https://agentwiki.org/durable_execution_for_agents — low-quality wiki). Workflow code must be deterministic; LLM/time/random live in activities.
- **[INF]** Your Chronos work (DST, Raft) plus Insights (CDC/WAL/Debezium) already covers event-sourcing/idempotency depth. The gap is *applying* it to agent traces, which is exactly record/replay.

### 3.4 What startups build / ask

- **[RES] Postings (all from one search; stage NOT confirmed for any):**
  - Glean "ML Engineer, LLM Evals & Observability" (trace enrichment, pipelines, dashboards; $200–300K) — https://job-boards.greenhouse.io/gleanwork/jobs/4669417005 — Glean is not early-stage.
  - LangChain "Engineering Manager, AI Observability & Evals Platform" ($200–250K) — ziprecruiter listing, not early-stage.
  - Town "AI Engineer, Evals & Agent Quality" (end-to-end eval ownership, regression prevention; on-site SF) — https://www.jobleads.com/us/job/ai-engineer-evals-agent-quality--san-francisco--e93e6654d0f91c3856dd34623d7e909b8 — Town is the most plausible early-stage match; funding stage not verified.
  - Scale AI "Frontier Agents Engineer" — reliability, observability, retries, graceful degradation.
- **[RES] Sandbox-infra funding (money is flowing here):** Daytona $60M Series B at $1B valuation (Fortune, 2026-10-20: https://fortune.com/2026/10/20/daytona-startup-building-the-agentic-cloud-valued-at-1-billion-in-latest-funding-round/) after a $24M Series A (PR Newswire, Feb 2026). Modal $355M Series C, May 2026 (secondary blog: https://bex.co/blog/2026/09/11/ai-sandbox-funding-modal-daytona-e2b; a ~$750M follow-up is unconfirmed). E2B funding sources conflict ($21M Series A Jul 2025 vs $35M).
- **[RES] Reported interview questions: evidence is THIN.** I found no Reddit/X/Glassdoor threads. The only direct item is a blog framing "debug an agent that is only sometimes wrong, can't reproduce, no errors" as a Senior+ interview question (https://aimlcompanion.ai/blog/ai-observability-debugging-sometimes-wrong-agent-2026, undated). Its answer pattern: trace → score → turn failures into dataset → fix → gate deploys; pin versions; redaction/retention of traces. Treat as one data point.
- **[INF]** The recurring ask across postings is "own the loop from traces to regression evals." Eval/observability *platform* companies (Braintrust, LangChain, Glean-type) want that loop; sandbox companies (Daytona/E2B/Modal) want secure-execution and infra depth.

### 3.5 Open source and unsolved gaps

- **[RES→INF]** Gaps visible from the sources above that a small project can credibly address:
  1. *Accuracy of the observer is unmeasured.* AgentSight (case studies only, per the summary I read) and OBI report capability, not correlation precision/recall against ground truth.
  2. *Record/replay from the OS boundary rather than SDK boundary.* Chronicle records at SDK-level model/tool boundaries with simulated models; nobody I found records at the syscall+TLS boundary and replays through the observer.
  3. *Concurrency breaks time-window correlation.* AgentSight's 100–500 ms window is a heuristic. How it degrades with parallel sub-agents is not quantified in what I read. [INF]
  4. *Non-OpenSSL TLS* (Go, rustls) stays a coverage hole; OBI handles Go. Don't chase this.

---

## 4. Gaps and opportunities specific to this repo

| Gap (what interviewer expects) | Evidence in repo | Opportunity |
|---|---|---|
| Reproducible correctness without root | Gates are shell scripts needing WSL2/k3s; no CI | Record/replay + golden logs run in plain CI |
| Ground truth + accuracy numbers | Only `/slow` p50 band | Correlation recovery curve vs injected loss/reorder |
| Explained overhead | 87%, no profile (per-event clone at `main.rs:389` ruled out, see INVESTIGATION.md) | Profile, cost model µs/event, before/after |
| Agent-awareness | Zero LLM/tool/token concepts | Parse LLM API calls over existing `TlsIo` path |
| Intent→effect linkage | No exec/open probes, no process tree beyond identity | Add 2 tracepoints + policy audit |
| Trace correctness | Roots only, no propagation | Parse `traceparent` from header prefix, or document why not |
| Fuzz/property testing of parsers | Unit tests only on h2/hpack | `proptest`/`cargo-fuzz` on `h2/frame.rs`, `hpack.rs`, `http.rs` |
| Credibility | Self-graded docs, staged verifier log, no baseline | Honest README with limitations + OBI comparison; relabel the log |

**Deprioritize / leave alone:**
- HPACK/CONTINUATION completeness, more HTTP/2 features.
- CPU profiling join (Phase 12), Grafana polish, TUI, `kind` variants.
- Go/rustls TLS.
- 14 handoff + 13 phase docs: do not add more. Archive or collapse.
- Do **not** claim or chase "<2% CPU".

---

## 5. Proposed next phases

Ordered by impact-per-hour. Hours are focused hours.

### Phase A — Make it checkable (4–5 h) · do first, no dependencies
- **Goal.** Remove the "only works on my laptop" objection and the cleanliness flags.
- **Deliverables.**
  - GitHub Actions: `cargo test` on `common` and userspace modules, `clippy`, `fmt`.
  - Clean the working tree: commit or discard `PROJECT-DEEP-DIVE.md` changes; delete `scripts/tmp-*.sh`.
  - Merge duplicate `ring-buffer-backpressure.md`.
  - README gains a "Limitations & non-goals" and "How this differs from OBI/Beyla/Pixie" section.
  - Retitle `verifier-rejection-log.md` to "forced stack-limit experiments" and say plainly they were induced.
- **Acceptance.** Green CI badge on `main`; CI log shows test count (expect ~180; verify).
- **Risk.** `agent/build.rs` compiles the eBPF crate (nightly + `-Z build-std` + `bpf-linker`). CI must install that or gate the BPF build; budget for it. If it eats >3 h, run tests on `common` + pure modules only and mark BPF build as manual.
- **Cut if short.** Docs consolidation.

### Phase B — Event log record/replay + loss/fuzz harness (14–16 h) · depends on A
- **Goal.** Turn the pipeline into a deterministic function of an event log. This is the single highest-leverage change: it makes every later claim reproducible in CI, and it is a direct echo of Chronos (DST) and of Chronicle.
- **Deliverables.**
  - `obsagent record --out run.evlog`: raw RingBuf records (already fixed-size `repr(C)`) plus header (kernel, `sample_n`, drop counters).
  - `obsagent replay run.evlog`: decode → reassemble → correlate → h2 → aggregate → export with **no BPF**.
  - Refactor required [OBS/INF]: `ingest_http_io` (`main.rs:1157`) takes `AyaHashMap` handles (`sock_meta`, `inflight`) directly. Put a small trait (`SockMetaSource`, `InflightSet`) in front of it with an in-memory impl for replay. This is real work, ~6–8 h of the budget.
  - 3–5 small golden logs recorded from `testdata/` (HTTP/1.1 `/slow`, writev split-header, gRPC, TLS) checked into `tests/fixtures/`.
  - Fault injector over the log: drop p%, reorder across simulated CPUs within a window (RingBuf is per-reserve order, not timestamp order [INF]), duplicate. Output curve: exchange-recovery rate and p50 error vs loss rate.
  - `proptest` or `cargo-fuzz` on frame/HPACK/HTTP parsers: no panics, bounded allocations.
- **Acceptance (measurable).**
  - Replay output byte-identical across 20 runs (borrowing Chronicle's stability measure).
  - Golden tests run in CI without root.
  - Published recovery curve (e.g. recovery % at 0/1/5/10/25% drops, with and without reorder).
  - Fuzzing: ≥10M iterations (or ≥1 h) with zero panics, or fixed crashes documented.
  - **Impressive:** the curve shows a *named* failure region you can explain (e.g. fd reuse within the 60 s TIMEOUT, `correlate.rs:22`) and a fix.
- **Risk.** Refactor sprawl in 1.5k-line `main.rs`. Cut: only ingest path + golden logs; skip reorder simulation.

### Phase C — Overhead truth (8–10 h) · depends on B (replay gives repeatable CPU profiling) but can start in parallel
- **Goal.** Convert "87% of a core" from a confession into a cost model you can defend.
- **Deliverables.**
  - Userspace flamegraph (`perf record` + inferno/samply) under the Vision-95 load, WSL2 and one real Linux host if you can get one.
  - Kernel-side cost via `bpf_stats` (`sysctl kernel.bpf_stats_enabled=1`, `bpftool prog show` → `run_time_ns`, `run_cnt`). This fixes the doc's own methodology hole.
  - Fix top hotspots. Candidates [INF, unverified until profiled; the `main.rs:389` clone was ruled out in INVESTIGATION.md]: event amplification, h2/HPACK decode, per-event state scans, per-event lock traffic. Option if profile says so: move the drain loop to a single-owner task passing batches, removing most `Arc<Mutex<>>`.
  - Table: µs CPU per event and per request, before/after, userspace + kernel.
- **Acceptance.** Same pinned load, before/after, 3 runs each; userspace CPU reduced by a stated factor with the cause named; kernel cost reported; written explanation of where each µs goes. A ≥2× drop is plausible but I don't know it. If the profile shows something else dominates, report that.
- **Risk.** WSL2 `perf` noise. Cut: do only the clone fix + `bpf_stats` + write-up (≈4 h).

### Phase D — LLM-call capture → OTel GenAI spans, with an *observer-accuracy eval* (22–26 h) · depends on B
- **Goal.** Make the repo agent-aware, and — the part that differentiates it from AgentSight/OBI — **measure how accurate the zero-instrumentation observer is** against SDK-side ground truth.
- **Day-1 gate (2 h feasibility spike, do before anything else).** Confirm `SSL_read/SSL_write` uprobes attach and fire for (a) Python `openai`/`anthropic` SDK (system libssl) and (b) Node (what Claude Code and many agent CLIs run on). AgentSight reports Claude Code works [RES]; I did not verify symbol availability. If Node fails, scope to Python only.
- **Deliverables.**
  - Host/path classifier for LLM APIs (`/v1/messages`, `/v1/chat/completions`) over existing `TlsIo` events.
  - Streaming (SSE) timing: time-to-first-byte of body (TTFT proxy), time-to-last-event, status, retries.
  - Token usage extraction from the final stream event. **Needs a capture-size change** (today: 256 B prefix, `SockIoEvent` 288 B): either tail capture of the last N bytes of a response or small chunk streaming. This touches `common/src/lib.rs` ABI, `ebpf/src/main.rs`, and RingBuf pressure. Check Anthropic/OpenAI docs for the exact usage fields before coding.
  - Emit spans with GenAI attributes (`gen_ai.operation.name`, model, usage tokens); **pin the semconv version** (status conflicting, §3.1).
  - Privacy default: store no prompt/response content, only metadata. (Chronicle notes recordings leak secrets [RES].)
  - **Deterministic mock LLM server** (injectable TTFT, inter-token delay, token counts, 429/500 faults). This is your `testdata/` pattern extended, and the oracle.
- **Acceptance (measurable).**
  - Over N≥200 calls on the mock server: eBPF-derived total latency and TTFT within ±5% of server-side truth (pick the tolerance before measuring); token counts exact.
  - Same on ≥1 real SDK path (Python) for N≥50 calls, compared to SDK-side timing.
  - Works with zero app changes; overhead reported in µs/event from Phase C.
  - **Impressive:** a table of *where the observer is wrong* (e.g. HTTP/2 multiplexed streams, keep-alive reuse, split TLS records) with counts.
- **Risk.** Biggest unknowns: Node symbol availability; tail-capture design vs verifier stack limits (you know this pain; use the per-CPU scratch pattern already in `IO_SCRATCH`). Cut: non-streaming only + Python only.

### Phase E — Intent→effect correlation + sandbox policy audit (22–26 h) · depends on D
- **Goal.** Hit the sandboxing leg and the "agent reliability" failure-detection story with a *measured* result.
- **Deliverables.**
  - Add `sched_process_exec`, `openat` tracepoints (and reuse existing `connect`). Process tree rooted at a configured PID/cgroup (leverage existing `cgroup_index.rs`).
  - Policy file: allowed path globs, egress allowlist (host from the TLS plaintext `Host` header/SNI via the existing path, since `connect` only has IPs).
  - Link each LLM response to subsequent effects by lineage + time window + argument matching, then **measure how that degrades with concurrency** (1, 2, 4, 8 parallel sub-agents).
  - Labeled benchmark of ~30 scripted scenarios against the mock LLM: benign (git clone, pip install, tests) and *harmless canary* violations (read a canary file outside allowed paths, connect to a non-allowlisted local sink, exec a banned binary). Never real exfil.
  - **Deterministic graders only** (path/host/exec checks). Deliberately skip an LLM observer; Anthropic's guidance prefers code graders where possible [RES], and it avoids judge-bias issues.
  - Detection only, no blocking.
- **Acceptance.** Report precision, recall, false-positive rate, detection latency, overhead; correlation accuracy vs concurrency curve; baseline comparison against "syscall-only, no LLM linkage".
- **Risk.** Drifting into a security product. Cut: exec + openat only, 10 scenarios, no concurrency sweep.

### Phase F — Write-up + demo (8–10 h) · depends on whatever is done
- **Deliverables.** One blog post, one 3-minute demo video, restructured README with a results table: observer accuracy, µs/event, replay determinism, recovery curve, detection precision/recall. Put limitations first.
- **Acceptance.** An outsider can reproduce replay and golden results with `git clone && cargo test && obsagent replay …` without root.
- **Cut.** Video.

### Minimum viable path (~50 h; fits comfortably before 2026-11-30)
A (4) → B (14, without reorder simulation) → C-lite (4: clone fix + `bpf_stats`) → D-lite (18: Python + mock server + non-streaming + SSE tail, accuracy table) → F-lite (8). Total ≈ 48–52 h.
Add E-lite (exec+openat, 10 scenarios, ~16 h) only if ≥20 h remain after D.

Suggested calendar:
- Nov 3–9: Phase A; D feasibility spike (day 1–2); start B.
- Nov 10–16: finish B; C-lite.
- Nov 17–23: D.
- Nov 24–30: F, plus E-lite if time. Kill rule: **if the D feasibility spike or the B refactor is not working by Nov 12, stop and freeze at A + B-lite** (see §9).

---

## 6. Interview-defensibility

**Ten hardest questions** (each cites where your current material is weak):

1. **"Why is it 87% of a core at 700 rps, and why does your `perf -p` number exclude the kernel side?"** Weak: no profile (the log implies ~1.37 M instructions per request), ~20 mutexes, WSL2-only measurement.
2. **"How do you pair requests and responses without a request ID, and what breaks it?"** Pipelining, multiple outstanding on one fd, fd reuse inside the 60 s `TIMEOUT` (`correlate.rs:22`), threads sharing an fd, `io_uring`/`sendfile` bypassing your tracepoints, `looks_like_request` heuristics on a 256 B prefix. You documented some; can you quantify the error rate? (Phase B's curve answers this.)
3. **"RingBuf is MPSC with per-reserve ordering. Do your kernel timestamps and your userspace processing order agree across CPUs?"** The state machine consumes in ring order; latency uses kernel `ts_ns`. A request and response on different CPUs can be consumed out of order. Likely a shallow spot.
4. **"What did the verifier actually reject, and how do you prove bounds on `bpf_probe_read_user`?"** Your log is induced and LLVM-level, not kernel-verifier. Know the 512 B stack limit, per-CPU scratch maps, bounded loops, null-checked `ringbuf_reserve`, and what the verifier tracks (ranges, pointer types).
5. **"Your sampling is sticky per fd with an adaptive N. Are the published percentiles unbiased?"** Connection-level sampling skews toward long-lived connections; doubling/halving with 30 s decay (`sample.rs`) can oscillate; histograms of sampled data need reweighting. Do you know that, and did you test it?
6. **"Walk through SSL uprobes: partial reads, `SSL_read_ex`, static linking, stripped symbols, TLS 1.3, what you miss."** Plus uprobe cost (trap per call) and why Go/rustls fail.
7. **"Histograms: why HdrHistogram, what about merging across nodes, coordinated omission, and bucket choice for OTLP cumulative histograms?"** You hit an OTLP 400 on `bucketCounts` (roadmap line) — the underlying bucket/percentile accuracy tradeoff is likely shallow.
8. **"Why does your trace have no parent/child relationship? How would you get context propagation with eBPF?"** Reading `traceparent` from the header prefix; why OBI's approach to this is hard; limits with TLS and HTTP/2 HPACK-compressed headers.
9. **"Why `Arc<Mutex<…>>` everywhere on a hot path instead of a single owner with channels? Lock ordering? Backpressure from the exporter?"**
10. **"Why not just use Beyla/OBI/Pixie — and how do you know your numbers are right?"** The question that decides the interview. Today the honest answer is "I don't have a baseline." After D/E the answer is a measured-accuracy table.

**Concepts, papers, books to understand (recommended from general knowledge, not fetched this session unless a URL appears above):**
- eBPF: kernel BPF docs (verifier, ring buffer); *Learning eBPF* (Liz Rice); *BPF Performance Tools* (Brendan Gregg); Aya book.
- Protocols: RFC 9113 (HTTP/2), RFC 7541 (HPACK), SSE spec.
- Tracing: Dapper paper; W3C Trace Context.
- Measurement: Gil Tene on coordinated omission; HdrHistogram docs.
- Backend: *Designing Data-Intensive Applications* (event logs, stream processing); durable-execution docs (Temporal/Restate/DBOS).
- Agent eval: Anthropic evals post (URL above), MAST (2503.13657), AgentRx, Chronicle (2609.20625), AgentSight (2508.02736), Zheng 2306.05685 for judge bias.

**Likely shallow [INF, based on repo signals]:** RingBuf cross-CPU ordering; verifier internals; sampling statistics; mutex architecture; why HTTP/2 stream state is keyed the way it is under connection reuse; anything in the 1.5k lines of hand-written HPACK/H2 you did not write line by line (the phase docs look agent-driven). Re-derive each by rewriting it on paper, or cut the claim.

---

## 7. Resume / portfolio angle

Draft bullets. **Do not use a bullet until its phase is done; replace brackets with measured numbers.**

- *True today:* "Built a Rust/Aya eBPF agent reconstructing HTTP/gRPC latency from syscall tracepoints and OpenSSL uprobes, with drop-counted RingBuf backpressure and adaptive sampling; measured ~87% of one core at 500 HTTP + ~200 gRPC rps on k3s and published it rather than claiming <2%."
- *After B:* "Made the event pipeline a deterministic function of a recorded event log: [N] golden traces replay byte-identically in CI without root; correlator recovers [Y]% of exchanges at [Z]% injected RingBuf loss; fuzzed parsers for [M] iterations with [K] crashes fixed."
- *After D:* "Zero-instrumentation LLM-call capture via TLS uprobes: TTFT and total latency within [X]% of SDK-side measurement over [N] calls, exact token counts, no application changes; [µs] CPU per event."
- *After E:* "Syscall-level policy audit for agent runs: detected [a]/[b] injected policy violations with [c] false positives over [d] benign runs; correlation accuracy vs sub-agent concurrency measured."

**Standout artifact.** A blog post titled for the finding, not the tool, e.g. "How accurate is a zero-instrumentation agent observer?" with the recovery curve, the observer-vs-SDK error table, and a failure-region analysis. It reads as an eval of an observability system, which is your spike. Open with limitations.

---

## 8. Open-source angle

I did **not** triage issue trackers this session; step one is 1 h of reading them. Realistic, tied-to-repo options:
1. **AgentSight** (eunomia-bpf; paper arXiv 2508.02736): contribute your labeled scenario benchmark and accuracy harness. Highest signal because it is a research-flavored gap [INF]. Needs maintainer acceptance; not guaranteed.
2. **OpenTelemetry eBPF Instrumentation (OBI):** check the support matrix for HTTP/2, SSE, GenAI payload gaps; a small, well-tested parser fix or fixture contribution is realistic. The release notes list detailed GenAI extraction as explicit-config, so there may be room.
3. **Chronicle** (code is public per the abstract): an adapter or example that feeds OS-boundary recordings into cut-point replay. Speculative.
4. **aya-rs:** docs or examples around uprobe/perf_event pitfalls you hit. Low risk, lower signal.

---

## 9. Recommendation: **SHARPEN, time-boxed**

Reasoning.
- The repo already has the expensive part: a working kernel→userspace pipeline and honest measurement culture. Freezing it wastes that, but polishing it as an APM clone is a dead end — OBI is a CNCF-hosted tool with broader TLS coverage [RES], and your own scope doc says "not Pixie-parity".
- It becomes interview-relevant only if it turns into an *evaluated agent flight recorder*: record/replay (B), a measured observer-accuracy table (D), and optionally a policy audit (E). Those speak directly to evals + observability + sandboxing.
- B alone has high value per hour: it fixes the reproducibility, CI, and Chronos-echo story.

**Kill / freeze rules.**
- If by **Nov 12** the D feasibility spike fails (no uprobe attach for any realistic agent runtime) *or* the B refactor has not produced a replayable golden test, **freeze at A + B-lite** and spend the remaining hours on a different artifact: an agent eval harness with deterministic graders + pass^k + trace-to-regression loop, which fits the primary spike more directly than anything in this repo.
- Do not add more phase documents. Every deliverable above is code + a number.

**Evidence limits.** I did not run the tests, profile anything, or open the issue trackers. Interview-question evidence is thin (one blog). Job-posting funding stages are unverified. Items marked "unverified" must be checked before you quote them.
