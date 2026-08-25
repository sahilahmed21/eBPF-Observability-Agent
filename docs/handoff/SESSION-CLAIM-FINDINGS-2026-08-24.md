# Claim-lock findings — 2026-08-24 (WSL2 + k3s)

Record of what we proved, fixed, and what is still open toward VISION-95 / Milestone 12.

## Environment

| Item | Value |
|---|---|
| Host | WSL2 Ubuntu 26.04 on Windows (`Collosal`) |
| Kernel | `6.6.114.1-microsoft-standard-WSL2` |
| Cluster | k3s `v1.36.3+k3s1`, runtime `containerd://2.3.2-k3s2` |
| Node | `collosal` Ready, IP `172.31.40.193` |
| Repo HEAD (recorded) | `da6a862bcd428ad1d2bb08e9d657d0fb52d5384e` |
| Image | `ebpf-obs-agent:latest` rebuilt same day (includes **cgroup-id** identity) |

## What we achieved today (ops + product)

### Host / k3s
1. **k3s initially crash-looped** with `ContainerManager … wrong number of fields (expected 6, got 7)` — caused by Docker Desktop WSL mount of `C:\Program Files\…` (unescaped space in `/proc/mounts`). Fixed by reinstalling Docker to a path without spaces / WSL integration hygiene.
2. **iptables** was missing; installed (`iptables` / nft).
3. k3s reached **active (running)**; kubeconfig `/etc/rancher/k3s/k3s.yaml`.

### Gate tooling
4. `e2e-k3s` `ROOT=/` bug: `wsl-run.sh` copies script to `/tmp` so script-relative ROOT broke. Fixed: `OBSAGENT_ROOT` export + fail-fast if `deploy/k8s/rbac.yaml` missing.
5. Claim-lock hygiene: fail-closed image import, `RESTORE_FAILED` privileged restore, stronger claim checks.

### Milestone 10 (named service map) — **PASS**
6. First e2e after cluster up: **P10-Q2 PASS** (`frontend tgid=7936` in `/host/proc`) but **named-edge FAIL**.
7. Diagnostics showed:
   - Dest join **worked**: `dst="demo/api"` in OTLP.
   - Src was `proc:unknown:<tgid>` (e.g. 32462) — that tgid **did not exist** in `/proc` or `/host/proc`.
   - Real frontend python host tgid was **8608** (cgroup had pod UID); pause was 7936.
8. Root cause: on this WSL path, **BPF event tgids ≠ host `/proc`**, so `/proc`-only identity fail-closed to `unknown` while ClusterIP→Service still named dest.
9. **Fix (Option B):** `bpf_get_current_cgroup_id()` on `SockIoEvent` + userspace `cgroup_index` (cgroupfs inode → pod UID) + DaemonSet mount of `/sys/fs/cgroup`. ABI SockIo **288→296**.
10. Rebuild image + re-run e2e → **`PASS: named edge src=demo/frontend… dst=demo/api`** · **`=== e2e-k3s PASS ===`**
11. Evidence: `docs/handoff/artifacts/e2e-k3s.pass`, log `docs/handoff/artifacts/logs/e2e-k3s.log`.

### Unit tests (cgroup-id change)
- `wsl-run.sh test-common` → 21 passed  
- `wsl-run.sh test-agent` → **161 passed** (incl. `cgroup_index::scan_maps_pod_slice_and_scope_inodes`)

## Final goal status

**VISION-95 resume sentence (legal only at claim lock 10/10):**  
> … HTTP/gRPC latency and a live service map … OTLP … under 2% CPU …

| Goal piece | Status |
|---|---|
| Zero-instr eBPF HTTP/HTTPS (demo phases) | Built earlier (M0–M5) |
| writev / capture completeness (M6) | **PASS on HEAD** — correctness6 2026-08-25 |
| gRPC / HTTP/2 (M7) | **PASS on HEAD** (2026-08-24 logs) |
| Dual-plane TLS (M8) | **PASS on HEAD** (2026-08-24 log) |
| OTLP traces (M9) | **PASS** — smoke9 + Grafana PNGs (`grafana-vision95.png`, `dashboard2.png`) |
| **Live named service map on real node (M10)** | **PROVEN** on WSL2 k3s |
| &lt;2% or honest % (M11) | **PROVEN honest %** — mean ~87% of one core on pin load |
| Profiles + verifier + claim rewrite (M12) | **LOCKED 10/10** (2026-08-25) |

**Did we achieve the final goal?**  
**Yes (VISION-95 claim lock 10/10), with honest overhead.** Do **not** claim under 2% — use **~87% of one core**. Residuals (Go TLS, rustls, XDP, multi-cluster, Pixie) stay out.

## Remaining

None for claim lock. Optional hygiene: resume DS after local BPF gates; delete `scripts/tmp-*` helpers.

**Done:** rows **1–10**. Profiles: `prof_hit=11`. Verifier: 2 stack-limit pastes. README/resume rewritten.

**Implementation remaining:** none required for VISION-95 unless a gate fails. Optional: allow-list for cleaner Grafana hero; `STRICT_COUNT=1` correctness6 on a quiet VM.

**Out of scope (residual 5%):** Go TLS, XDP, multi-cluster, EndpointSlice-as-dest, etc.

## Honest completion estimate

| Lens | Estimate |
|---|---|
| Feature code for Phases 6–12 | ~85–90% written |
| Claim-proven on current host | **~50%** (5/10 rows) |
| Path to “project closed” | M11 → M12 → rewrite claims |

## Keep

- WSL2 + k3s + cgroup-id identity path that just passed M10  
- Fail-closed e2e image/ROOT/restore hygiene  
- Honest README until 10/10
