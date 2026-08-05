//! Kernel ↔ userspace ABI for Phase 1 sock latency events.
//!
//! # Latency semantics (Q7)
//! `latency_ns` is syscall enter→exit only (`bpf_ktime_get_ns` delta).
//! Non-blocking `connect` that returns `-EINPROGRESS` measures the syscall,
//! not TCP handshake completion.

#![no_std]

/// RingBuf byte size (Q8). Must be a power of two.
pub const EVENTS_RINGBUF_BYTES: u32 = 256 * 1024;

/// Pending enter HashMap max entries (Q8).
pub const PENDING_MAP_ENTRIES: u32 = 8192;

/// IPv4 only for Phase 1 (Q4).
pub const AF_INET: u16 = 2;

/// Direction / event kind (Q5).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    Connect = 1,
    Accept = 2,
}

impl EventKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Connect),
            2 => Some(Self::Accept),
            _ => None,
        }
    }
}

/// Emitted on syscall exit when an enter record was found.
///
/// Layout is frozen for Phase 1 Milestone 1. Do not reorder fields without
/// bumping a version strategy (none yet — regenerating both sides together).
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

#[cfg(feature = "user")]
unsafe impl aya::Pod for SockLatencyEvent {}

#[cfg(feature = "user")]
unsafe impl aya::Pod for PendingEnter {}

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
        assert_eq!(EventKind::from_u8(0), None);
        assert_eq!(EventKind::from_u8(3), None);
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
}
