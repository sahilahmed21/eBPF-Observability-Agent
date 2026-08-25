# Session 2026-08-20 — Phase 11 (sampling + capture maps)

Implemented sticky `KEEP_IO` + capture maps + load scripts. **Did not** run `perf stat` on a real node. Milestone 11 is **not** ticked. Do not claim &lt;2%.

## Landed

- BPF: `SAMPLE_N`, `DENIED_TGID`, `ALLOWED_TGID`, `ALLOW_ONLY`, `SOCK_META` flags bit1/2
- I/O path does **not** insert `SOCK_META` for unmarked fds (Phase 2 connect/accept gate preserved)
- `KEEP_IO` is sticky: connect/accept merge preserves an already-drawn sample bit
- Userspace: auto `{1,2,4,8,16}`, pin `OBSAGENT_SAMPLE_N`, `/proc` leader sweep on a **separate** task, `obsagent.sample_n` only after a successful map write
- Allow-only (`OBSAGENT_COMM_ALLOW`): BPF default-deny + `ALLOWED_TGID` (not “deny the world”)
- Scripts: `overhead-vision95.sh` / `overload-vision95.sh` share `DURATION_SECS` (vegeta, ghz, `perf`)

## Not done

- 11.4 three-run headline on the M10 node
- 11.5 prefix 128 trial
- Phase 12

## Env

- `OBSAGENT_SAMPLE_N` — BPF pin (disables auto)
- `OBSAGENT_TRACE_SAMPLE` — spans (unchanged)
- `OBSAGENT_COMM_ALLOW` — exclusive allow-list (BPF `ALLOW_ONLY`)
