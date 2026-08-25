# Phase 10 — Real-node service map

Plan: [phase-10-implementation-plan.md](phase-10-implementation-plan.md).  
North star: [VISION-95.md](VISION-95.md) Q12, Q13.  
Evidence: [../testing/phase-10.tdd.md](../testing/phase-10.tdd.md).

**Buys:** live service map with **named** src and dst. Kind-in-Docker-on-WSL is **not** this gate.

**Blocked on:** a node where P10-Q2 (BPF tgid ∈ mounted proc) is PASS, then `scripts/e2e-k3s.sh`.

## Checklist

- [ ] Real Linux node (WSL2 Ubuntu k3s if P10-Q2 passes, else cloud Ubuntu 22.04/24.04, BTF, kernel 5.15+, cgroup v2)
- [ ] k3s (default containerd + systemd cgroup) **without** nested PID mismatch
- [ ] Proof: BPF `tgid` exists in the `/proc` the agent mounts (`scripts/check-pid-ns.sh` in the agent pod)
- [x] cgroup → pod uid → kube name for src; dest pod IP → pod, ClusterIP → **Service** ns/name (unit tests; not EndpointSlice)
- [x] Demo yaml: HTTP/1.1 named-edge gate + gRPC + OpenSSL HTTPS in `demo`
- [x] `scripts/e2e-k3s.sh` + `wsl-run.sh k3s-e2e` (`K3S_E2E=1` requires a cluster)
- [ ] `K3S_E2E=1` / e2e script **green on a real node**
- [ ] Deny-list on; demo-namespace named edge ≥ 1; host noise not dominating
- [ ] Handoff topology diagram filled from a green node (PID ns, `/proc`, cgroup)
- [ ] Interview pack: kind PID lie marked **historical** — **only after** e2e GREEN

## Milestone 10

Scrape/labels show `src=demo/frontend-…` and `dst=demo/api` (Service name). `proc:unknown` is **not** the demo series.

**Status:** contracts + unit tests implemented; **e2e not run** — Milestone 10 **not** ticked.
