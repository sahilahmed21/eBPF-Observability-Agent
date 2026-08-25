# Phase 6 review-fix TDD evidence

**Source:** core review (copy bound, INFLIGHT lifetime, drain `/proc`, empty ALLOW).  
**Date:** 2026-08-18  
**Status:** unit GREEN. Checkpoint git commits not created (repo rule: commits only when asked).

## User journeys

1. As an operator, I want `writev` prefixes to be the first iovec’s bytes, not adjacent process memory.
2. As an operator, I want a dropped first chunk not to leave the fd emitting forever.
3. As an operator, I want a hung split-header to stop emitting after 60 s without `close()`.
4. As an operator, I do not want every syscall to parse `/proc/.../cgroup` on the RingBuf thread.
5. As an operator, I want `OBSAGENT_COMM_ALLOW=` to mean unset, not deny-all.

## Task → test mapping

| Behavior | Test target | RED | GREEN |
|---|---|---|---|
| `io_copy_len(ret, iov_len)` | `common` `io_copy_len_vectored_uses_iov_not_ret` | compile: `io_copy_len` missing | 16 passed (`wsl-run.sh test-common`) |
| HTTP magic requires method space | `common` `http_magic_requires_method_space` | compile: `http_magic` missing | same |
| `PendingIo` carries `buf_len` | `common` `pending_io_layout` (24 B) | (blocked by compile RED) | 24 B |
| Decode zeros prefix tail | `decode::zeros_bytes_past_prefix_len` | (agent not compiled until BPF linked) | 53 passed |
| Timeout unmarks `(tgid, fd)` | `reassemble::timeout_evicts` | `take_unmark` missing | PASS `[(42, 1)]` |
| Empty allow is not exclusive | `filter::empty_allow_set_is_not_exclusive` | would deny `nginx` | PASS |
| `OBSAGENT_K8S=0` is off | `filter::k8s_zero_is_off` | `k8s_defaults_enabled` missing | PASS |
| comm() does not need cgroup | `identity::comm_reads_only_comm_file` | `comm` missing | PASS |

## Commands & outcomes

```text
# RED (2026-08-18)
wsl -d Ubuntu -- bash scripts/wsl-run.sh test-common
  error[E0425]: cannot find function `io_copy_len` in this scope
  error[E0425]: cannot find function `http_magic` in this scope
  error: could not compile `obsagent-common` (lib test) due to 14 previous errors

# GREEN
wsl -d Ubuntu -- bash scripts/wsl-run.sh test-common  → 16 passed
wsl -d Ubuntu -- bash scripts/wsl-run.sh test-agent   → 53 passed
```

BPF link RED (6-arg `emit_io_kind`, “stack arguments are not supported”) then GREEN after passing `&PendingIo` (3 args).

## Guarantees

| # | What is guaranteed | Evidence |
|---|---|---|
| 1 | Vectored copy bound is `min(ret, iov[0].len, 256)`, not `min(ret, 256)` | `io_copy_len` unit tests |
| 2 | `POST` without a space is not HTTP magic | `http_magic` unit tests |
| 3 | Decode zeros `prefix[prefix_len..]` | `zeros_bytes_past_prefix_len` |
| 4 | Reassembly timeout yields `(tgid, fd)` for INFLIGHT remove | `timeout_evicts` + `take_unmark` |
| 5 | Empty allow-set is deny-list mode, not exclusive empty | `empty_allow_set_is_not_exclusive` |
| 6 | `OBSAGENT_K8S=0`/`false` does not enable k8s defaults | `k8s_zero_is_off` |
| 7 | Filter comm lookup can succeed with only `/proc/<tgid>/comm` | `comm_reads_only_comm_file` |

BPF mark-after-submit and probe-fail-skip are not unit-tested (no BPF unit harness). They are in `ebpf/src/main.rs` `emit_io_kind`: `mark_inflight` only after `slot.submit`; failed `bpf_probe_read_user_buf` returns before reserve.

## Coverage / gaps

- Unit: common 16, agent 53. No tarpaulin in this repo; 80% line coverage not measured.
- INFLIGHT-after-submit: compile/link only.
- `writev` iov[0] << ret: unit formula covered; correctness6 fixture still has iov[0] ≈ full headers (e2e not re-run this cycle).
- SockIoEvent stays 288 B: filter uses `IdentityResolver::comm` (comm file + TTL cache), not BPF `comm` on the event. Full cgroup parse remains per completed exchange in `handle_exchange`.
- Reassembler HashMap cap (review Minor): not added.

## Merge evidence

No checkpoint commits. RED: missing `io_copy_len`/`http_magic`. GREEN: 16 + 53 unit tests. BPF 6-arg link failure fixed by `&PendingIo`.
