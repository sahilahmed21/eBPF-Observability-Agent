# Vision-95 overhead load (Phase 11)

**Do not run this as a 2% proof until Milestone 10** (real node) and Phases 7–8 probes exist.
Profiles off (`OBSAGENT_PROFILE` unset).

Pinned in [phase-11-implementation-plan.md](../docs/phases/phase-11-implementation-plan.md) P11-Q1 / P11-Q9.

| Stream | Rate | Duration | Notes |
|---|---|---|---|
| HTTP/1.1 | **500 RPS** | 60 s | vegeta `-http2=false`; demo `api` or `HTTP_URL` |
| h2 / gRPC unary | **200 RPS** | 60 s | ghz vs `hello.HelloService/SayHello` (grpcbin) |
| Runs | **3** | | mean agent CPU = headline |

**Loadgen (pinned):**

| Tool | Version | Role |
|---|---|---|
| [vegeta](https://github.com/tsenart/vegeta/releases/tag/v12.13.0) | **v12.13.0** | HTTP/1.1 constant RPS |
| [ghz](https://github.com/bojand/ghz/releases/tag/v0.121.0) | **v0.121.0** | gRPC unary |

Install from those GitHub release assets (or `go install` at that tag). Scripts fail if the binary is missing.

**Connection pool:** vegeta `-workers=64 -max-workers=256`. ghz `--connections=50 -c 50`. Sticky 1/N is per fd — a 2-connection keep-alive pool makes sampling a no-op.

**Measure:** `perf stat -p <agent_pid>` on the node. Headline = mean of 3 of `(agent_cpu_seconds / wall_seconds) * 100` as % of **one core** (`CPUs utilized × 100` from `task-clock`).

**Footnote (not a second headline):** eBPF probe time is billed to the **syscall’s CPU** (the app / kubelet), not the agent task. `perf -p` does not include it.

**Target:** mean &lt; 2.0. If not: sampling / BPF deny / prefix trial — **do not lower RPS**.

**Scripts:**

- `scripts/overhead-vision95.sh` — 500+200; `DURATION_SECS` (default 60) for vegeta, ghz, and `perf stat -p`; set `AGENT_PID`
- `scripts/overload-vision95.sh` — 5k RPS HTTP/1.1, 15 s

Env: `HTTP_URL` (default `http://127.0.0.1:8080/`), `GRPC_TARGET` (default `127.0.0.1:9000`), `GRPC_CALL`.

Baseline: same load **without** the agent, then with agent attached. Baseline must actually hit 500/200.
