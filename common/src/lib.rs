//! Kernel ↔ userspace ABI for sock latency (Phase 1), sock I/O (Phase 2),
//! and TLS I/O (Phase 3).
//!
//! # Latency semantics (Q7 / Phase 1)
//! `SockLatencyEvent::latency_ns` is syscall enter→exit only (`bpf_ktime_get_ns` delta).
//! Non-blocking `connect` that returns `-EINPROGRESS` measures the syscall,
//! not TCP handshake completion.
//!
//! # HTTP latency (Q8 / Phase 2)
//! Application HTTP latency is reconstructed in userspace from `SockIoEvent`
//! pairs: request-half exit `ts_ns` → response-half exit `ts_ns`.
//!
//! # HTTPS latency (Phase 3 Q2)
//! Same SM over [`TlsIoEvent`] (layout twin of [`SockIoEvent`]): TLS half exit→exit.
//! Dual-plane wire timing is out of Milestone 3.

#![no_std]

/// RingBuf byte size (Phase 1 Q8). Must be a power of two.
pub const EVENTS_RINGBUF_BYTES: u32 = 256 * 1024;

/// Pending enter HashMap max entries (Phase 1 Q8).
pub const PENDING_MAP_ENTRIES: u32 = 8192;

/// IPv4 only for Phase 1 (Q4).
pub const AF_INET: u16 = 2;

/// Bounded HTTP/socket prefix (Phase 2 Q1).
pub const SOCK_IO_PREFIX_LEN: usize = 256;

/// Direction / event kind.
///
/// RingBuf demux (Phase 2 Q5 / Phase 3 Q3): first byte of each reserved record is `EventKind`.
/// `Connect`/`Accept` → [`SockLatencyEvent`] (48 B).
/// `SockIo` / `TlsIo` → [`SockIoEvent`] / [`TlsIoEvent`] (288 B twins).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    Connect = 1,
    Accept = 2,
    SockIo = 3,
    /// OpenSSL plaintext prefix (Phase 3 Q3).
    TlsIo = 4,
}

impl EventKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Connect),
            2 => Some(Self::Accept),
            3 => Some(Self::SockIo),
            4 => Some(Self::TlsIo),
            _ => None,
        }
    }
}

/// Sock I/O direction (Phase 2).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IoDir {
    Read = 1,
    Write = 2,
}

impl IoDir {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Read),
            2 => Some(Self::Write),
            _ => None,
        }
    }
}

/// Emitted on connect/accept syscall exit when an enter record was found.
///
/// Layout is frozen for Phase 1 Milestone 1 (Phase 2 Q6: do not add `fd` here).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SockLatencyEvent {
    pub kind: u8,
    pub _pad0: [u8; 7],
    pub pid: u32,
    pub tgid: u32,
    /// Syscall return value (negative errno on failure).
    pub ret: i64,
    pub latency_ns: u64,
    /// Exit timestamp, `CLOCK_MONOTONIC` ns (`bpf_ktime_get_ns`).
    pub ts_ns: u64,
    /// Remote IPv4 address as raw `sin_addr.s_addr` (network-order bytes in memory).
    pub daddr_be: u32,
    /// Remote port, network byte order.
    pub dport_be: u16,
    pub _pad1: u16,
}

pub const SOCK_LATENCY_EVENT_SIZE: usize = core::mem::size_of::<SockLatencyEvent>();

/// Bounded socket read/write prefix (Phase 2).
///
/// Emitted on syscall exit after enter stashed the userspace buffer pointer (Q3).
/// `prefix_len` is `min(max(ret, 0), SOCK_IO_PREFIX_LEN)` when `ret >= 0`, else 0.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SockIoEvent {
    pub kind: u8,
    pub dir: u8,
    /// Valid bytes in `prefix` (≤ `SOCK_IO_PREFIX_LEN`).
    pub prefix_len: u16,
    pub fd: i32,
    pub pid: u32,
    pub tgid: u32,
    /// Syscall return value (byte count or negative errno).
    pub ret: i64,
    /// Exit timestamp, `CLOCK_MONOTONIC` ns.
    pub ts_ns: u64,
    pub prefix: [u8; SOCK_IO_PREFIX_LEN],
}

pub const SOCK_IO_EVENT_SIZE: usize = core::mem::size_of::<SockIoEvent>();

/// Phase 3 Q3: layout twin of [`SockIoEvent`]; `kind` must be [`EventKind::TlsIo`].
pub type TlsIoEvent = SockIoEvent;

pub const TLS_IO_EVENT_SIZE: usize = SOCK_IO_EVENT_SIZE;

/// Per-fd peer metadata (Phase 4 Q1). BPF map value for `SOCK_META`.
///
/// Replaces presence-only `SOCK_FDS` (`u8`). Key remains `(tgid, fd)` as `u64`.
/// `flags` bit0 = peer address is valid.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SockMeta {
    pub daddr_be: u32,
    pub dport_be: u16,
    pub flags: u8,
    pub _pad: u8,
}

impl SockMeta {
    pub const FLAG_HAS_ADDR: u8 = 1;

    pub fn with_peer(daddr_be: u32, dport_be: u16) -> Self {
        Self {
            daddr_be,
            dport_be,
            flags: Self::FLAG_HAS_ADDR,
            _pad: 0,
        }
    }

    pub fn has_addr(self) -> bool {
        self.flags & Self::FLAG_HAS_ADDR != 0
    }
}

pub const SOCK_META_SIZE: usize = core::mem::size_of::<SockMeta>();

/// Enter-side pending record (HashMap value). Not sent on the RingBuf.
///
/// Connect: `daddr_be`/`dport_be` filled on enter (`has_addr=1`).
/// Accept: `sockaddr_ptr` stored on enter; peer addr read on exit.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PendingEnter {
    pub ts_ns: u64,
    pub daddr_be: u32,
    pub dport_be: u16,
    /// 1 if `daddr_be`/`dport_be` already valid.
    pub has_addr: u8,
    pub _pad: u8,
    pub sockaddr_ptr: u64,
}

/// Enter-side pending for sock I/O (Phase 2 Q3). Not sent on the RingBuf.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PendingIo {
    pub buf_ptr: u64,
    pub fd: i32,
    pub dir: u8,
    pub _pad: [u8; 3],
}

/// Enter-side pending for TLS I/O (Phase 3 Q7). Not sent on the RingBuf.
///
/// `ssl_ptr` is looked up in `SSL_FD` on exit to recover `fd` (Q1).
/// `outlen_ptr` is non-zero for `SSL_{read,write}_ex` (points at `size_t` result).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PendingTls {
    pub buf_ptr: u64,
    pub ssl_ptr: u64,
    pub outlen_ptr: u64,
    pub dir: u8,
    pub _pad: [u8; 7],
}

#[cfg(feature = "user")]
unsafe impl aya::Pod for SockLatencyEvent {}

#[cfg(feature = "user")]
unsafe impl aya::Pod for SockIoEvent {}

#[cfg(feature = "user")]
unsafe impl aya::Pod for SockMeta {}

#[cfg(feature = "user")]
unsafe impl aya::Pod for PendingEnter {}

#[cfg(feature = "user")]
unsafe impl aya::Pod for PendingIo {}

#[cfg(feature = "user")]
unsafe impl aya::Pod for PendingTls {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sock_latency_event_is_48_bytes() {
        assert_eq!(SOCK_LATENCY_EVENT_SIZE, 48);
        assert_eq!(core::mem::align_of::<SockLatencyEvent>(), 8);
    }

    #[test]
    fn event_kind_roundtrip() {
        assert_eq!(EventKind::from_u8(1), Some(EventKind::Connect));
        assert_eq!(EventKind::from_u8(2), Some(EventKind::Accept));
        assert_eq!(EventKind::from_u8(3), Some(EventKind::SockIo));
        assert_eq!(EventKind::from_u8(4), Some(EventKind::TlsIo));
        assert_eq!(EventKind::from_u8(0), None);
        assert_eq!(EventKind::from_u8(5), None);
    }

    #[test]
    fn constants_match_q8() {
        assert_eq!(EVENTS_RINGBUF_BYTES, 256 * 1024);
        assert_eq!(PENDING_MAP_ENTRIES, 8192);
        assert!(EVENTS_RINGBUF_BYTES.is_power_of_two());
    }

    #[test]
    fn pending_enter_layout() {
        assert_eq!(core::mem::size_of::<PendingEnter>(), 24);
    }

    // --- Phase 2 SockIO ABI (Q1=256, Q5=kind-tagged, Q6=P1 unchanged) ---

    #[test]
    fn sock_latency_event_unchanged_at_48_bytes() {
        assert_eq!(SOCK_LATENCY_EVENT_SIZE, 48);
    }

    #[test]
    fn sock_io_prefix_is_256() {
        assert_eq!(SOCK_IO_PREFIX_LEN, 256);
    }

    #[test]
    fn sock_io_event_layout() {
        assert_eq!(SOCK_IO_EVENT_SIZE, 288);
        assert_eq!(core::mem::align_of::<SockIoEvent>(), 8);
        assert_eq!(
            core::mem::size_of_val(&SockIoEvent {
                kind: EventKind::SockIo as u8,
                dir: IoDir::Read as u8,
                prefix_len: 0,
                fd: 0,
                pid: 0,
                tgid: 0,
                ret: 0,
                ts_ns: 0,
                prefix: [0; SOCK_IO_PREFIX_LEN],
            }),
            288
        );
    }

    #[test]
    fn io_dir_roundtrip() {
        assert_eq!(IoDir::from_u8(1), Some(IoDir::Read));
        assert_eq!(IoDir::from_u8(2), Some(IoDir::Write));
        assert_eq!(IoDir::from_u8(0), None);
    }

    #[test]
    fn pending_io_layout() {
        assert_eq!(core::mem::size_of::<PendingIo>(), 16);
        assert_eq!(core::mem::align_of::<PendingIo>(), 8);
    }

    #[test]
    fn event_kind_byte_discriminates_ringbuf_payload() {
        // Q5: demux by first byte; sizes differ.
        assert_ne!(SOCK_LATENCY_EVENT_SIZE, SOCK_IO_EVENT_SIZE);
        assert_eq!(EventKind::Connect as u8, 1);
        assert_eq!(EventKind::SockIo as u8, 3);
        assert_eq!(EventKind::TlsIo as u8, 4);
    }

    // --- Phase 3 TlsIo ABI (Q3 twin, Q4=256) ---

    #[test]
    fn tls_io_is_sock_io_twin() {
        assert_eq!(TLS_IO_EVENT_SIZE, SOCK_IO_EVENT_SIZE);
        assert_eq!(TLS_IO_EVENT_SIZE, 288);
        assert_eq!(core::mem::size_of::<TlsIoEvent>(), core::mem::size_of::<SockIoEvent>());
        assert_eq!(core::mem::align_of::<TlsIoEvent>(), 8);
    }

    #[test]
    fn pending_tls_layout() {
        assert_eq!(core::mem::size_of::<PendingTls>(), 32);
        assert_eq!(core::mem::align_of::<PendingTls>(), 8);
    }

    // --- Phase 4 SockMeta ---

    #[test]
    fn sock_meta_layout() {
        assert_eq!(SOCK_META_SIZE, 8);
        assert_eq!(core::mem::align_of::<SockMeta>(), 4);
        let m = SockMeta::with_peer(0x0100007f, 0x5000);
        assert!(m.has_addr());
        assert_eq!(m.daddr_be, 0x0100007f);
        assert_eq!(m.dport_be, 0x5000);
    }
}
