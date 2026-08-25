# Verifier rejection log

Append-only log of **real** BPF verifier / bpf-linker stack rejections hit while building this agent.
Hypothetical entries do not belong here — interview prep needs scar tissue.

| Date | Program / function | Symptom (verifier message summary) | Root cause | Fix / restructure |
|------|--------------------|--------------------------------------|------------|-------------------|
| 2026-08-25 | `emit_io_kind` | BPF stack limit exceeded (bpf-linker / LLVM) | Temporary 600 B `force_pad` on stack as `bpf_probe_read_user_buf` sink (VISION Q18 force) | Keep I/O event body in `IO_SCRATCH` PerCpuArray; never allocate large prefixes on the BPF stack |
| 2026-08-25 | `profile_sample` | BPF stack limit exceeded (bpf-linker / LLVM) | Temporary 600 B `bpf_get_stack` destination on stack (VISION Q18 force #2) | Keep stack sample buffer ≤ `STACK_BUF_BYTES` (8×u64); enlarge only via PerCpuArray if needed |

## How to write a good entry

1. Paste the truncated verifier log (or the decisive lines).
2. Name the probe and the helper/map access that failed.
3. Explain the restructuring in one sentence (e.g. “moved 512-byte buffer from stack to per-CPU array”).
4. Link the commit SHA once code exists.

## Entry 1 — `emit_io_kind` stack limit (2026-08-25)

**Context:** VISION-95 Q18 — no natural reject paste survived Phases 6–12; force a 600 B stack sink into the always-loaded sock I/O emit path, capture, restore. Production code already uses `IO_SCRATCH: PerCpuArray<SockIoEvent>`.

**Decisive lines** (from `docs/handoff/artifacts/logs/vrej-build-stack600.log`):

```
ERROR llvm: … in function _RNvCs57rxnwnBDlA_6probes12emit_io_kind void (i8, ptr, i64):
Looks like the BPF stack limit is exceeded. Please move large on stack variables into BPF per-cpu array map.
…
Error: LLVM issued diagnostic with error severity
error: could not compile `obsagent-ebpf` (bin "probes") due to 1 previous error
```

**Fix kept:** `IO_SCRATCH.get_ptr_mut(0)` + write `SockIoEvent` there; bounded `bpf_probe_read_user_buf` into `ev.prefix` only.

## Entry 2 — `profile_sample` stack limit (2026-08-25)

**Context:** Second Q18 force — enlarge `bpf_get_stack` destination to 600 B in `profile_sample` (loaded when `OBSAGENT_PROFILE=1`).

**Decisive lines** (from `docs/handoff/artifacts/logs/vrej-build-getstack600.log`):

```
ERROR llvm: … in function profile_sample i32 (ptr):
Looks like the BPF stack limit is exceeded. Please move large on stack variables into BPF per-cpu array map.
…
Error: LLVM issued diagnostic with error severity
error: could not compile `obsagent-ebpf` (bin "probes") due to 1 previous error
```

**Fix kept:** `let mut stack = [0u8; STACK_BUF_BYTES as usize]` with `STACK_BUF_BYTES = 8 * size_of::<u64>()`; copy into `StackSampleEvent.ips` and submit via `STACKS` RingBuf.

## Common rejection classes (reference — not substitutes for real entries)

- Unbounded / verifier-unprovable loops
- Stack > 512 bytes
- Invalid pointer arithmetic / unchecked map value pointers
- Reading kernel/user memory without `bpf_probe_read_*`
- Incomplete `bpf_ringbuf_reserve` null checks before write
- Type mismatches on CO-RE field access without proper BTF
