# `common/`

Shared event types and constants used by both `ebpf/` (`no_std`) and `agent/`.

Keep layouts `#[repr(C)]` and stable — this is the kernel↔user ABI.
