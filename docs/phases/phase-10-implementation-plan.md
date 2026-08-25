# Phase 10 — Implementation Plan (Real-node service map)

Companion to [phase-10.md](phase-10.md). Locked: [VISION-95.md](VISION-95.md) Q12, Q13.
Architecture review (2026-08-20) is the dest-join source of truth: **ClusterIP → Service ns/name**, not EndpointSlice.

**Phase 10 buys:** live service map with **named** src and dst. Kind-in-Docker-on-WSL is **not** this gate.

**Gate from Phase 9:** traces/metrics export works; this phase is **topology + identity**, not new protocols.

---

## 0. Out of Phase 10

Multi-cluster, Cilium/Istio identity, Hubble, multi-node mesh, EndpointSlice-as-dest, kube crate / informers, conntrack, a database, BPF/ABI changes, Phase 11 sampling, claiming kind-in-Docker works.

---

## 1. Locked decisions

| ID | Choice |
|---|---|
| **P10-Q1** | Default cluster: **k3s**. Prefer WSL2 Ubuntu if P10-Q2 passes; otherwise a cloud Ubuntu VM. Not kind-in-Docker. |
| **P10-Q2** | Prefer BPF `tgid` ∈ mounted proc. On WSL, BPF tgids may be absent from `/proc` — fall back to `bpf_get_current_cgroup_id()` → cgroupfs inode → pod UID (`cgroup_index`). Fail closed (`proc:unknown`) if both miss. |
| **P10-Q3'** | Src: cgroup → pod uid → Pod list. Dst: **pod IP → pod ns/name**; **ClusterIP → Service ns/name**. Reuse `DstId::Pod` for both labels. |
| **P10-Q4'** | `GET/list` **pods + services**, same 15s poll, no kube crate, no watch/informers in M10. |
| **P10-Q5** | RBAC: `get/list/watch` on core **pods** and **services**. No EndpointSlice RBAC. |
| **P10-Q6** | Demo ns `demo`: HTTP frontend→`api` is the **named-edge gate**. Also OpenSSL HTTPS (`api-tls`) and cleartext gRPC (`api-grpc`) in the same namespace. |
| **P10-Q7** | `scripts/e2e-k3s.sh`. `wsl-run.sh k3s-e2e` strips CRLF. Does not fail `test-agent` if k3s is missing. `K3S_E2E=1` makes a missing cluster a FAIL. |
| **P10-Q8** | Named client edge required: `frontend` → `api`. IP-only dst for that series is a **fail**. Server-side reverse edges are allowed. |
| **P10-Q9** | Document kernel, BTF, cgroup v2, k3s version in the handoff **from a green e2e node**. |
| **P10-Q10** | **k3s on WSL2 Ubuntu if PID-ns script passes**; else cloud Ubuntu VM. No extra hypervisor until Q2 fails. |
| **P10-Q11** | k3s default **containerd + systemd cgroup**. |

Rejected: mapping ClusterIP through EndpointSlice to a ready pod (invents a replica `connect()` never saw). EndpointSlices do not carry ClusterIP; the Service object does.

---

## 2. Architecture (identity)

```text
connect()/accept() sockaddr → SOCK_META
prefixes → RingBuf → correlate → ParsedExchange

tgid → /host/proc/cgroup → uid → PodIndex.by_uid → SRC ns/name
daddr → podIP then ClusterIP → ns/name; else ip:port
     → ServiceMap + OTLP
```

Drain: sync lookups only. Kube list: async 15s; on failure **keep last snapshot**. Export: existing 10s task. No agent DB. No new process.

### Partial failure

- Pods list fail → keep **entire** previous index.
- Services list fail, pods ok → update pod maps; **keep previous ClusterIP map**.
- CA/token/403/timeout: last snapshot; warn; drain continues.

### Lookup order

`lookup_ip`: **pod IP first**, then ClusterIP (collision impossible in real k8s; tests still encode pod-wins).

Pod IP is stored only for **Running**, **non-hostNetwork**, **non-deleting** pods. UID map still includes terminating/succeeded pods so src join works.

List parse: invalid JSON or missing `items` is **Err** (keep last snapshot). `items: []` is the only empty replace. Lists are paginated (`limit=500`, follow `continue`; first page `resourceVersion=0`).

---

## 3. Subphases

### 10.0 — Preconditions

M9 green. Agent image builds (`deploy/Dockerfile`).

---

### 10.1 — Topology (PID ns)

**Work:** `scripts/check-pid-ns.sh`. Run inside the agent pod on a real node **before** asserting names.

**Verify:** P10-Q2 PASS. Do not tick M10 from unit tests alone.

---

### 10.2 — Kube index (pods + services)

**Work:** Two IPv4 maps in `k8s_index.rs`; parse `status.podIP`/`podIPs[]` and `spec.clusterIP`/`clusterIPs[]`; skip `"None"` / non-v4; reuse reqwest client; `info!` refresh sizes.

**Verify:** `test-agent` fixtures (no cluster).

---

### 10.3 — RBAC + DaemonSet

**Work:** `deploy/k8s/rbac.yaml` services get/list. Keep `hostPID: true` and `/host/proc`. Privileged fallback if k3s rejects `CAP_BPF`.

**Verify:** agent Running on a node that passes P10-Q2.

---

### 10.4 — Named-edge gate

**Work:** Demo HTTP frontend→`api`. `scripts/e2e-k3s.sh` asserts scrape `src=demo/frontend…` and `dst=demo/api`. `proc:unknown` must not be that series.

**Verify:** script PASS on a real node; paste scrape in the handoff. **Do not** tick this from a SKIP.

---

### 10.5 — Interview + close

**Work:** Mark kind-in-Docker limitation as historical in `RESUME_AND_STORY.md` **only after** 10.4 is GREEN (don't claim 95% yet). Tick phase-10.md.

---

## 4. Success criteria

1. PID ns check PASS on the node that runs the agent.
2. Demo HTTP series named both ends (`frontend` src, `api` Service dst).
3. ClusterIP dest is the Service name, not a guessed replica.
4. Handoff topology diagram after a green e2e.

---

## 5. Appendix — §10

| ID | Answer | Date | Notes |
|---|---|---|---|
| P10-Q10 | k3s on WSL2 Ubuntu if Q2 passes; else cloud Ubuntu | 2026-08-20 | architecture review |
| P10-Q11 | k3s default containerd + systemd cgroup | 2026-08-20 | architecture review |
