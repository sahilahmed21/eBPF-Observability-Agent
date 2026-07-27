# Verifier rejection log

Append-only log of **real** BPF verifier rejections hit while building this agent.
Hypothetical entries do not belong here — interview prep needs scar tissue.

| Date | Program / function | Symptom (verifier message summary) | Root cause | Fix / restructure |
|------|--------------------|--------------------------------------|------------|-------------------|
| — | — | *(none yet — Phase 0+)* | — | — |

## How to write a good entry

1. Paste the truncated verifier log (or the decisive lines).
2. Name the probe and the helper/map access that failed.
3. Explain the restructuring in one sentence (e.g. “moved 512-byte buffer from stack to per-CPU array”).
4. Link the commit SHA once code exists.

## Common rejection classes (reference — not substitutes for real entries)

- Unbounded / verifier-unprovable loops
- Stack > 512 bytes
- Invalid pointer arithmetic / unchecked map value pointers
- Reading kernel/user memory without `bpf_probe_read_*`
- Incomplete `bpf_ringbuf_reserve` null checks before write
- Type mismatches on CO-RE field access without proper BTF
