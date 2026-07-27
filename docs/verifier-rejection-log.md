# Verifier rejection log

Every BPF verifier rejection that forced a restructure goes here. Interview ammo = real scars, not textbook hypothetics.

| Date | Program | Reject reason (short) | Fix |
|---|---|---|---|
| | | | |

## Common patterns to expect

- Unbounded / verifier-unprovable loops → bounded `for` with constant limit
- Stack > 512 B → smaller locals, per-cpu array scratch
- Invalid pointer / missing `bpf_probe_read*` → helper reads + null checks
- Unreleased ringbuf reservation → always discard/submit on all paths
- Map value too large → shrink event / use truncated prefix

Template entry:

```
### YYYY-MM-DD — <prog name>
- Reject: <paste key verifier line>
- Why unsafe from verifier POV:
- Restructure:
```
