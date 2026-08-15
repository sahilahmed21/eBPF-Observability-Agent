# Phase 4 — Implementation Plan

Companion to [phase-4.md](phase-4.md). Architecture source of truth: session design review
(2026-08-12) + this locked Q appendix.

**Phase 4 buys:** node-local production path — peer-aware service map, cgroup→pod identity,
cumulative OTLP metrics, DaemonSet deploy, kind demo. Keeps Phases 1–3 probe→correlate→HTTP path.

---

## Locked decisions (do not silently reverse)

| ID | Choice |
|---|---|
| **Q1 Peer binding** | Replace presence-only `SOCK_FDS` with `SOCK_META[(tgid,fd)] → {daddr,dport,flags}` at connect enter / accept exit. Join peer onto `Exchange` in userspace. Do **not** bloat every SockIo/TlsIo. |
| **Q2 Remote identity** | Destination always `IP:port`; best-effort enrich to pod via in-cluster pod IP index. Never require perfect pod names. |
| **Q3 Local identity** | `/proc/<tgid>/cgroup` + `/proc/<tgid>/comm` → container_id / pod_uid when present; fallback `proc:{comm}:{tgid}`. |
| **Q4 OTLP scope** | Cumulative **metrics first**. Sampled traces **out of M4** (optional later). Soft-fail if export disabled/unreachable. |
| **Q5 Aggregation dual-view** | Keep CLI/TUI rolling **60s** `HttpAggregator` (gate compat). Service map + OTLP use separate cumulative/edge state. |
| **Q6 Persistence** | **None** in-agent. Collector/Prometheus own retention. |
| **Q7 Privileges** | Prefer `CAP_BPF` + `CAP_PERFMON` + `CAP_SYS_PTRACE`; `privileged: true` only as documented fallback. `hostPID: true`; ro BTF/proc mounts. |
| **Q8 Cardinality** | Cap service-map edges (4096); overflow → `_other` bucket. |
| **Q9 Export queue** | Bounded OTLP record buffer; drop + counter on full — never block RingBuf drain. |
| **Q10 TLS map** | Keep client-only TLS HTTP pairing (Phase 3 post-review). Service map is outbound/client-centric for HTTPS. |

---

## Component map

| Component | Path | Notes |
|---|---|---|
| ABI `SockMeta` | `common/` | 8 B map value |
| eBPF `SOCK_META` | `ebpf/` | replaces `SOCK_FDS` |
| Correlator | `agent/correlate.rs` | `Exchange.peer` optional |
| Identity | `agent/identity.rs` | cgroup parse + LRU |
| Pod IP index | `agent/k8s_index.rs` | SA token poll; soft-fail |
| Service map | `agent/service_map.rs` | in-RAM adjacency |
| OTLP export | `agent/export.rs` | cumulative metrics |
| Deploy | `deploy/` | Dockerfile, DS, RBAC, collector |

---

## Explicit non-goals (M4)

- Graph DB, dual-plane TLS, HTTP/2, always-on traces, CRI-socket primary identity, IPv6,
  redesign of working probes, ClusterIP→pod via Endpoints (optional later).
