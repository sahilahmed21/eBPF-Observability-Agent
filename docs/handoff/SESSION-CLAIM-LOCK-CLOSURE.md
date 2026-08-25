# Claim Lock Closure — session 2026-08-24

**Architecture source:** Claim Lock Closure (M10 → M11 → M12). No Phase 13 product work.  
**Status:** foundations + evidence scaffolding only. **Gates blocked** — no reachable k3s/containerd node in this environment.

## Environment probe (C0)

| Check | Result |
|---|---|
| Repo root `/mnt/c/projects/eBPF-Observability-Agent` | OK |
| BTF `/sys/kernel/btf/vmlinux` | present |
| `kubectl` | present (`/usr/local/bin/kubectl`) |
| Cluster API | **refused** — kubeconfig points at `127.0.0.1:40719` (stale kind) |
| `k3s` binary | **not** on PATH |
| `wsl-run.sh k3s-e2e` | **SKIP: no reachable cluster** (2026-08-24) |

**Host decision (locked default):** try WSL2 Ubuntu + k3s if P10-Q2 passes; else cloud Ubuntu 22.04/24.04. This session did **not** install k3s (requires interactive sudo / host change).

## What this phase implements (ops, not new BPF features)

```text
C0 host ready → C1 P10-Q2 → C2 e2e-k3s (M10)
  → C3 overhead-vision95 ×3 PROFILE=0 (M11)
  → C4 overload-vision95 (row 7)
  → C5 correctness12 (row 8)
  → C6 verifier log ≥2 (row 9)
  → C7 SESSION-VISION-95 + README/resume rewrite (M12)
```

Runtime architecture **unchanged**: DaemonSet `hostPID` + `/host/proc`, PodIndex ClusterIP→Service, sticky sampling, profiles off for M11.

## Exact commands (run on closure host)

```bash
# After k3s+containerd is up and kubeconfig points at it:
export K3S_E2E=1
# Build/import image first (see deploy/Dockerfile), then:
bash scripts/wsl-run.sh k3s-e2e

# M11 — on the same node; OBSAGENT_PROFILE must be unset/0:
AGENT_PID=<agent> HTTP_URL=... GRPC_TARGET=... bash scripts/overhead-vision95.sh
# Repeat 3×; fill docs/overhead.md and docs/testing/phase-11.tdd.md

# M12 profile gate (root/BPF):
sudo bash scripts/wsl-run.sh correctness12

# Verifier: paste ≥2 real rejects into docs/verifier-rejection-log.md (Q18 force only if still empty)
# Then fill docs/handoff/SESSION-VISION-95.md and rewrite README / interview resume.
```

## Claim lock progress

| Row | Status after this session |
|---|---|
| 1–4 | **PASS** — linked to prior phase TDD evidence (no re-run this session) |
| 5–10 | **open** — blocked on k3s node / root gates / verifier harvest / rewrite |

## Out of scope (still)

Go TLS, rustls, XDP, EndpointSlice dest join, always-on fleet profiling, inventing verifier pastes, claiming `&lt;2%` without `perf` row.
