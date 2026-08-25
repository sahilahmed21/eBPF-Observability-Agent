# Handoff — Phase 10 real-node dest join (2026-08-20)

**Paste into a new chat:** *Read `docs/handoff/SESSION-2026-08-20-phase-10.md` and continue from §Next. Do not re-litigate locked Qs (ClusterIP → Service name, not EndpointSlice). Do not claim VISION-95 resume sentence. Do not commit unless asked. Do not lift Q8 (first iovec, 256 B). Do not start Phase 11. Do not mark kind-in-Docker PID as historical until e2e-k3s is GREEN.*

**Checklist:** [`docs/phases/phase-10.md`](../phases/phase-10.md)  
**Plan + locked Qs:** [`docs/phases/phase-10-implementation-plan.md`](../phases/phase-10-implementation-plan.md)  
**TDD:** [`docs/testing/phase-10.tdd.md`](../testing/phase-10.tdd.md)  
**Prior:** Phase 9 traces + Grafana (M9 green).

**Repo:** `c:\projects\eBPF-Observability-Agent` (Windows checkout; **build/run only on WSL2 Ubuntu**)  
**Working tree:** **UNCOMMITTED.** Do **not** commit unless the user asks.

---

## 0. Environment

```bash
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh test-agent
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh k3s-e2e
# require a cluster:
K3S_E2E=1 wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh k3s-e2e
```

Always `wsl -d Ubuntu` (not `docker-desktop`). Root gates: `wsl -d Ubuntu -u root`.

---

## 1. What shipped (code)

Kube index lists **pods and services** on the existing 15s poll (no kube crate, no EndpointSlice). `lookup_ip` is pod IP then ClusterIP. ClusterIP dest label is the **Service** `ns/name` (`demo/api`), reused `DstId::Pod`. Partial failure keeps the last snapshot (pods fail → keep all; services fail → keep ClusterIP map). Identity parses guaranteed slices and k3s cgroupfs `pod<uid>`. RBAC adds `services` get/list. Demo yaml: HTTP named-edge gate + OpenSSL HTTPS + cleartext gRPC. Scripts: `check-pid-ns.sh`, `e2e-k3s.sh`.

**Not shipped:** a green k3s e2e on a node that passes P10-Q2. Milestone 10 is **not** ticked.

---

## 2. Topology (fill from a green node)

```text
[host PID ns]  BPF tgid  ==  /host/proc/<tgid>  (DaemonSet hostPID: true)
       │
       ├─ cgroup → pod UID → Pod list → src = demo/frontend-<pod>
       └─ SOCK_META.daddr
              ├─ podIP        → dst = demo/<pod>
              └─ Service VIP  → dst = demo/api   (not a replica)
```

| Field | Value |
|---|---|
| Kernel | *(fill)* |
| BTF | *(fill `/sys/kernel/btf/vmlinux`)* |
| cgroup | *(fill v2 / systemd)* |
| k3s | *(fill)* |
| P10-Q2 | **not run in agent pod** |
| scrape excerpt | **not captured** |

---

## 3. Gates

| Gate | Result | Date |
|---|---|---|
| `test-agent` | **139 passed** (review fixes) | 2026-08-20 |
| `e2e-k3s.sh` | SKIP (no reachable cluster) | 2026-08-20 |

---

## 4. Next

1. Install k3s on WSL2 Ubuntu **or** a cloud Ubuntu VM.
2. Confirm P10-Q2 inside the agent pod (`check-pid-ns.sh` / e2e exec).
3. Build/load `ebpf-obs-agent:latest` (`deploy/Dockerfile`, `k3s ctr images import`).
4. `K3S_E2E=1` `wsl-run.sh k3s-e2e` until PASS.
5. Paste scrape + kernel/k3s versions here. Then — and only then — tick Milestone 10 and mark kind PID as historical in the interview pack.

Do **not** start Phase 11 until Milestone 10 is green.
