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

**Phase 11 (locked):** still (a). Add in-kernel **sticky 1/N of connections**, not per-event modulo.

- `SAMPLE_N` Array: auto `{1,2,4,8,16}` (double when `DROPS` increased in the last 1 s; halve after 30 s quiet) or pin `OBSAGENT_SAMPLE_N`.
- Draw `KEEP_IO` once per `(tgid,fd)` at `SOCK_META` insert (connect/accept). I/O paths must not create a new `SOCK_META` key. All `SockIo` / `TlsIo` / `SockIoTimes` on that fd follow the bit. Skipping frames independently would unpair HTTP and desync HPACK.
- Sample skip does **not** increment `DROPS`. `DROPS` remains reserve-fail only.
- Deny-list: `DENIED_TGID` skips probe work for the agent tgid + deny-list comms **before** prefix copy. Allow-only (`OBSAGENT_COMM_ALLOW`): `ALLOW_ONLY` + `ALLOWED_TGID` (default deny). Userspace `CommFilter` still runs.
- `/proc` sweep is a separate task (not on the RingBuf drain tick). Numeric `/proc` names that are tids (status `Tgid:` ≠ pid) are not map keys.
- `n ≤ 1` keeps all I/O. Never `% 0`.

Export `obsagent.sample_n` (gauge) next to `obsagent.events_dropped`.

## Tradeoff to say in interviews

Silent data loss vs kernel memory blowup. We refuse silent loss: **drops are first-class telemetry**. Completeness under overload is not promised; overhead and host stability are.
