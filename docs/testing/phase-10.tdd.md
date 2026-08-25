# Phase 10 TDD evidence

**Source plan:** [phase-10-implementation-plan.md](../phases/phase-10-implementation-plan.md)  
**Status:** unit GREEN (`test-agent` **161 passed** incl. cgroup-id). **e2e PASS** 2026-08-24 on WSL2 k3s (containerd). Milestone 10 evidence recorded in `docs/handoff/artifacts/e2e-k3s.pass`.

## Task → test mapping

| Plan | Test target | GREEN |
|---|---|---|
| 10.2 pods | `parses_pod_list` | Running podIP `10.0.0.5` → `demo/frontend` |
| 10.2 podIPs | `parses_pod_ips_array_v4_skips_v6` | extra v4 inserted; v6 skipped |
| 10.2 services | `parse_service_list_cluster_ip_is_service_name` | ClusterIP → **Service** `demo/api`; `None` skipped |
| 10.2 join | `lookup_pod_ip_then_cluster_ip_pod_wins_collision` | pod wins if the same IP is in both maps |
| 10.2 partial | `merge_keeps_previous_cluster_ips_when_services_list_fails` | pods update; ClusterIP map kept |
| 10.2 fail-closed | `merge_pods_fail_returns_none` / `invalid_json_is_err_not_empty_ok` | missing list / garbage JSON is `Err`, not empty Ok |
| 10.2 services parse | `invalid_services_json_keeps_cluster_ips` | garbage services body keeps previous ClusterIPs |
| 10.2 IP eligibility | `host_network_ip_skipped_uid_kept` / `succeeded_and_deleting_ips_skipped` | UID kept; dest IP skipped |
| 10.2 pages | `continue_token_on_empty_page` / `absorb_pages_joins_continue_chunks` | continue + merge pages |
| 10.2 identity | `parses_guaranteed_slice` / `parses_k3s_cgroupfs_pod_uid` / existing burstable | k3s cgroup paths |
| 10.2 cgroup-id | `cgroup_index::scan_maps_pod_slice_and_scope_inodes` | inode → pod UID for WSL ghost tgids |
| 10.1 topology | `scripts/check-pid-ns.sh` + e2e frontend tgid | BPF/workload tgid ∈ mounted proc (**node**) |
| 10.4 e2e | `scripts/e2e-k3s.sh` | same scrape line `src=demo/frontend` `dst=demo/api` (**node**) |

## Results

| Command | Result | Date | Kernel / k3s |
|---|---|---|---|
| `wsl-run.sh test-agent` | **161 passed** | 2026-08-24 | WSL2 + cgroup-id |
| `wsl-run.sh k3s-e2e` | **PASS** named edge + P10-Q2 | 2026-08-24 | 6.6.114.1-microsoft-standard-WSL2 / k3s v1.36.3+k3s1 containerd |

## Notes

Dest for a client `connect()` to `api.demo.svc` is the **ClusterIP**. The index maps that IP to the Service name `demo/api`, not a backend pod from EndpointSlice.

WSL finding: BPF `tgid` may be absent from `/host/proc` while traffic is real; src naming uses `cgroup_id` → cgroupfs inode → pod UID when `/proc` miss.
