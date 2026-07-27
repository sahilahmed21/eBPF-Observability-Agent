# Ring buffer backpressure

Interview: *RingBuf fills faster than userspace can drain — options? What did you pick?*

## Options

| Option | Pros | Cons |
|---|---|---|
| **(a) Sample / drop + drop counter** | Bounds kernel mem; loss is visible | Incomplete traces under load |
| **(b) Larger RingBuf** | Absorbs short spikes | Delays OOM/drop; doesn’t fix sustained overload |
| **(c) Userspace bounded channel + drop policy** | Clear app-level policy | Still need kernel-side bound; extra copy |

## Decision (locked for this project)

**Default: (a)** — on `bpf_ringbuf_reserve` failure, increment a BPF map counter (`drop_count`). Userspace scrapes it and exports as a metric (`ebpf_events_dropped_total`).

Use (b) only as a **small** sized buffer tuned for normal load, not as the primary strategy.

Optional later: kernel-side **sampling rate** map (e.g. 1-in-N) when drop rate exceeds a threshold — still under (a)’s philosophy.

## Tradeoff to say in interviews

Silent data loss vs kernel memory blowup. We refuse silent loss: **drops are first-class telemetry**. Completeness under overload is not promised; overhead and host stability are.
