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
//! # HTTPS latency (Phase 3 Q2 / Phase 8)
//! Content plane: [`TlsIoEvent`] (layout twin of [`SockIoEvent`]): TLS half exit→exit.
//! Timing plane: [`SockIoTimesEvent`] on TLS fds (no prefix). Handshake: [`TlsHandshakeEvent`].

#![no_std]

/// RingBuf byte size (Phase 1 Q8). Must be a power of two.
pub const EVENTS_RINGBUF_BYTES: u32 = 256 * 1024;

/// Stack sample RingBuf (Phase 12). Separate from HTTP `EVENTS`.
pub const STACKS_RINGBUF_BYTES: u32 = 128 * 1024;

/// Max user stack IPs per perf sample (Phase 12 P12-Q12).
pub const STACK_SAMPLE_MAX_FRAMES: usize = 8;

/// Userspace sample ring cap / TTL (Phase 12).
pub const PROFILE_STORE_CAP: usize = 8192;
pub const PROFILE_STORE_TTL_SECS: u64 = 2;
pub const PROFILE_MIN_LATENCY_NS_DEFAULT: u64 = 20_000_000;
pub const PROFILE_TOP_FRAMES: usize = 5;

/// Pending enter HashMap max entries (Phase 1 Q8).
pub const PENDING_MAP_ENTRIES: u32 = 8192;

/// BPF `DENIED_TGID` / `ALLOWED_TGID` HashMap max entries (Phase 11).
pub const DENIED_TGID_ENTRIES: u32 = PENDING_MAP_ENTRIES;
pub const ALLOWED_TGID_ENTRIES: u32 = PENDING_MAP_ENTRIES;

/// IPv4 (`AF_INET`). IPv6 peers use [`AF_INET6`] in [`SockMeta`] (Phase 6).
pub const AF_INET: u16 = 2;
pub const AF_INET6: u16 = 10;

/// Bounded HTTP/socket prefix (Phase 2 Q1).
pub const SOCK_IO_PREFIX_LEN: usize = 256;

/// Scalar write/sendto: copy bound is `ret`, not a known iovec length.
pub const IO_BUF_LEN_UNBOUNDED: u32 = u32::MAX;

/// Bytes of `buf_ptr` that may be copied into `SockIoEvent::prefix`.
///
/// Vectored: `buf_len` is `iov[0].len`. Scalar: [`IO_BUF_LEN_UNBOUNDED`] (bound is `ret`).
#[inline]
pub const fn io_copy_len(ret: i64, buf_len: u32) -> u16 {
    if ret <= 0 || buf_len == 0 {
        return 0;
    }
    let mut n = ret as u64;
    if n > buf_len as u64 {
        n = buf_len as u64;
    }
    if n > SOCK_IO_PREFIX_LEN as u64 {
        n = SOCK_IO_PREFIX_LEN as u64;
    }
    n as u16
}

/// HTTP/1.1 request or response start (method + space, or `HTTP/`).
pub fn http_magic(prefix: &[u8], len: u16) -> bool {
    let n = (len as usize).min(prefix.len());
    let s = match prefix.get(..n) {
        Some(s) => s,
        None => return false,
    };
    s.starts_with(b"GET ")
        || s.starts_with(b"POST ")
        || s.starts_with(b"PUT ")
        || s.starts_with(b"HEAD ")
        || s.starts_with(b"DELETE ")
        || s.starts_with(b"OPTIONS ")
        || s.starts_with(b"PATCH ")
        || s.starts_with(b"HTTP/")
        || s.starts_with(b"PRI ")
}

/// Direction / event kind.
///
/// RingBuf demux (Phase 2 Q5 / Phase 3 Q3): first byte of each reserved record is `EventKind`.
/// `Connect`/`Accept` → [`SockLatencyEvent`] (48 B).
/// `SockIo` / `TlsIo` → [`SockIoEvent`] / [`TlsIoEvent`] (296 B twins).
/// `SockIoTimes` → [`SockIoTimesEvent`] (32 B). `TlsHandshake` → [`TlsHandshakeEvent`] (48 B).
/// `StackSample` → [`StackSampleEvent`] on the `STACKS` RingBuf only (Phase 12).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    Connect = 1,
    Accept = 2,
    SockIo = 3,
    /// OpenSSL plaintext prefix (Phase 3 Q3).
    TlsIo = 4,
    /// TLS-fd syscall timing only — no prefix (Phase 8).
    SockIoTimes = 5,
    /// `SSL_do_handshake` enter→success (Phase 8).
    TlsHandshake = 6,
    /// perf_event user stack sample (Phase 12; `STACKS` RingBuf).
    StackSample = 7,
}

impl EventKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Connect),
            2 => Some(Self::Accept),
            3 => Some(Self::SockIo),
            4 => Some(Self::TlsIo),
            5 => Some(Self::SockIoTimes),
            6 => Some(Self::TlsHandshake),
            7 => Some(Self::StackSample),
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
/// `prefix_len` is [`io_copy_len`] (`min(ret, first-buffer-len, 256)`); tail is zero.
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
    /// `bpf_get_current_cgroup_id()` (cgroupfs inode). 0 = unset.
    /// Used for k8s src identity when BPF tgid ∉ host `/proc` (WSL).
    pub cgroup_id: u64,
    pub prefix: [u8; SOCK_IO_PREFIX_LEN],
}

pub const SOCK_IO_EVENT_SIZE: usize = core::mem::size_of::<SockIoEvent>();

/// Phase 3 Q3: layout twin of [`SockIoEvent`]; `kind` must be [`EventKind::TlsIo`].
pub type TlsIoEvent = SockIoEvent;

pub const TLS_IO_EVENT_SIZE: usize = SOCK_IO_EVENT_SIZE;

/// Timing-only sock I/O on a TLS fd (Phase 8). No user-buffer copy.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SockIoTimesEvent {
    pub kind: u8,
    pub dir: u8,
    pub _pad: u16,
    pub fd: i32,
    pub pid: u32,
    pub tgid: u32,
    pub ret: i64,
    pub ts_ns: u64,
}

pub const SOCK_IO_TIMES_EVENT_SIZE: usize = core::mem::size_of::<SockIoTimesEvent>();

/// `SSL_do_handshake` success (Phase 8). `latency_ns` is first-enter → this exit.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TlsHandshakeEvent {
    pub kind: u8,
    pub _pad0: [u8; 7],
    pub pid: u32,
    pub tgid: u32,
    pub fd: i32,
    pub _pad1: u32,
    pub ret: i64,
    pub latency_ns: u64,
    pub ts_ns: u64,
}

pub const TLS_HANDSHAKE_EVENT_SIZE: usize = core::mem::size_of::<TlsHandshakeEvent>();

/// perf_event stack sample on the `STACKS` RingBuf (Phase 12).
///
/// `ips[0..frame_count]` are user-space return addresses (raw, not normalized).
#[repr(C, align(8))]
#[derive(Clone, Copy, Debug)]
pub struct StackSampleEvent {
    pub kind: u8,
    pub _pad0: [u8; 7],
    pub tgid: u32,
    pub pid: u32,
    pub ts_ns: u64,
    pub frame_count: u8,
    pub _pad1: [u8; 3],
    pub ips: [u64; STACK_SAMPLE_MAX_FRAMES],
}

pub const STACK_SAMPLE_EVENT_SIZE: usize = core::mem::size_of::<StackSampleEvent>();

/// Per-fd peer metadata. BPF map value for `SOCK_META`.
///
/// Key remains `(tgid, fd)` as `u64`.
/// `flags` bit0 = peer address is valid.
/// `flags` bit1 = I/O sample decision taken (Phase 11).
/// `flags` bit2 = keep SockIo/TlsIo/SockIoTimes for this fd.
/// v4: `daddr[0..4]` = `sin_addr.s_addr` native bytes, rest 0.
/// v6: `daddr` = `sin6_addr.s6_addr`.
#[repr(C, align(8))]
#[derive(Clone, Copy, Debug)]
pub struct SockMeta {
    pub family: u8,
    pub flags: u8,
    pub dport_be: u16,
    pub daddr: [u8; 16],
    pub _pad: [u8; 4],
}

/// Lookup of `SOCK_META` for I/O sampling. [`SockMetaKeep::Unmarked`] means the
/// fd was never connect/accept-marked — the I/O path must not insert a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SockMetaKeep {
    Unmarked,
    Decided(bool),
    Undecided,
}

impl SockMeta {
    pub const FLAG_HAS_ADDR: u8 = 1 << 0;
    pub const FLAG_SAMPLE_DECIDED: u8 = 1 << 1;
    pub const FLAG_KEEP_IO: u8 = 1 << 2;

    /// Keep I/O for this fd, once a sample bit has been drawn.
    ///
    /// `None` = not decided yet (caller should draw). Sticky afterwards.
    pub fn io_sampled_keep(self) -> Option<bool> {
        if self.flags & Self::FLAG_SAMPLE_DECIDED == 0 {
            None
        } else {
            Some(self.flags & Self::FLAG_KEEP_IO != 0)
        }
    }

    /// Set the sticky I/O keep bits. Does not clear [`Self::FLAG_HAS_ADDR`].
    pub fn set_io_keep(&mut self, keep: bool) {
        self.flags &= !(Self::FLAG_SAMPLE_DECIDED | Self::FLAG_KEEP_IO);
        self.flags |= Self::FLAG_SAMPLE_DECIDED;
        if keep {
            self.flags |= Self::FLAG_KEEP_IO;
        }
    }

    pub fn with_peer_v4(daddr_be: u32, dport_be: u16) -> Self {
        let mut daddr = [0u8; 16];
        daddr[..4].copy_from_slice(&daddr_be.to_ne_bytes());
        Self {
            family: AF_INET as u8,
            flags: Self::FLAG_HAS_ADDR,
            dport_be,
            daddr,
            _pad: [0; 4],
        }
    }

    pub fn with_peer_v6(daddr: [u8; 16], dport_be: u16) -> Self {
        Self {
            family: AF_INET6 as u8,
            flags: Self::FLAG_HAS_ADDR,
            dport_be,
            daddr,
            _pad: [0; 4],
        }
    }

    pub fn has_addr(self) -> bool {
        self.flags & Self::FLAG_HAS_ADDR != 0
    }

    /// I/O keep for an **existing** `SOCK_META` lookup. Missing meta is unmarked:
    /// do not capture I/O and do not insert a new key.
    pub fn keep_from_lookup(meta: Option<Self>) -> SockMetaKeep {
        match meta {
            None => SockMetaKeep::Unmarked,
            Some(m) => match m.io_sampled_keep() {
                Some(keep) => SockMetaKeep::Decided(keep),
                None => SockMetaKeep::Undecided,
            }
        }
    }

    /// Copy peer from `self` (typically [`Self::with_peer_v4`]) and keep sample
    /// bits from `old` when already decided. Does not draw a new keep bit.
    pub fn merge_peer_preserve_sample(self, old: Option<Self>) -> Self {
        let mut out = self;
        if let Some(old) = old {
            if old.io_sampled_keep().is_some() {
                out.flags = (out.flags & Self::FLAG_HAS_ADDR)
                    | (old.flags & (Self::FLAG_SAMPLE_DECIDED | Self::FLAG_KEEP_IO));
            }
        }
        out
    }

    /// IPv4 `sin_addr.s_addr` bytes, or `None` if this is not AF_INET.
    pub fn v4_addr(self) -> Option<u32> {
        if self.family == AF_INET as u8 {
            Some(u32::from_ne_bytes([
                self.daddr[0],
                self.daddr[1],
                self.daddr[2],
                self.daddr[3],
            ]))
        } else {
            None
        }
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
    /// First iovec length, or [`IO_BUF_LEN_UNBOUNDED`] for scalar write/sendto.
    pub buf_len: u32,
    pub fd: i32,
    pub dir: u8,
    /// 1 = TLS fd: emit [`SockIoTimesEvent`], do not copy prefix.
    pub timing_only: u8,
    pub _pad: [u8; 2],
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
unsafe impl aya::Pod for SockIoTimesEvent {}

#[cfg(feature = "user")]
unsafe impl aya::Pod for TlsHandshakeEvent {}

#[cfg(feature = "user")]
unsafe impl aya::Pod for StackSampleEvent {}

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
        assert_eq!(EventKind::from_u8(5), Some(EventKind::SockIoTimes));
        assert_eq!(EventKind::from_u8(6), Some(EventKind::TlsHandshake));
        assert_eq!(EventKind::from_u8(7), Some(EventKind::StackSample));
        assert_eq!(EventKind::from_u8(0), None);
        assert_eq!(EventKind::from_u8(8), None);
        assert_eq!(AF_INET6, 10);
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
        assert_eq!(SOCK_IO_EVENT_SIZE, 296);
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
                cgroup_id: 0,
                prefix: [0; SOCK_IO_PREFIX_LEN],
            }),
            296
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
        assert_eq!(core::mem::size_of::<PendingIo>(), 24);
        assert_eq!(core::mem::align_of::<PendingIo>(), 8);
    }

    #[test]
    fn io_copy_len_vectored_uses_iov_not_ret() {
        // writev: iov[0] is 20 B, syscall wrote 8000 B into later iovecs.
        assert_eq!(io_copy_len(8000, 20), 20);
        assert_eq!(io_copy_len(20, 20), 20);
        assert_eq!(io_copy_len(20, u32::MAX), 20);
        assert_eq!(io_copy_len(512, u32::MAX), SOCK_IO_PREFIX_LEN as u16);
        assert_eq!(io_copy_len(512, 10), 10);
        assert_eq!(io_copy_len(0, 100), 0);
        assert_eq!(io_copy_len(-1, 100), 0);
        assert_eq!(io_copy_len(100, 0), 0);
    }

    #[test]
    fn http_magic_requires_method_space() {
        assert!(http_magic(b"GET /", 5));
        assert!(http_magic(b"POST /", 6));
        assert!(http_magic(b"HTTP/1.1", 8));
        assert!(!http_magic(b"POST", 4));
        assert!(!http_magic(b"POSTGRES", 8));
        assert!(!http_magic(b"GET", 3));
        assert!(http_magic(b"PRI * HTTP/2.0", 14));
        assert!(!http_magic(b"PRINTER", 7));
    }

    #[test]
    fn event_kind_byte_discriminates_ringbuf_payload() {
        // Q5: demux by first byte; sizes differ.
        assert_ne!(SOCK_LATENCY_EVENT_SIZE, SOCK_IO_EVENT_SIZE);
        assert_eq!(EventKind::Connect as u8, 1);
        assert_eq!(EventKind::SockIo as u8, 3);
        assert_eq!(EventKind::TlsIo as u8, 4);
        assert_eq!(EventKind::SockIoTimes as u8, 5);
        assert_eq!(EventKind::TlsHandshake as u8, 6);
        assert_eq!(EventKind::StackSample as u8, 7);
    }

    #[test]
    fn stack_sample_event_layout() {
        assert_eq!(STACK_SAMPLE_EVENT_SIZE, 96);
        assert_eq!(core::mem::align_of::<StackSampleEvent>(), 8);
        assert!(STACKS_RINGBUF_BYTES.is_power_of_two());
    }

    #[test]
    fn sock_io_times_is_32_bytes() {
        assert_eq!(SOCK_IO_TIMES_EVENT_SIZE, 32);
        assert_eq!(core::mem::align_of::<SockIoTimesEvent>(), 8);
        assert!(SOCK_IO_TIMES_EVENT_SIZE <= 48);
    }

    #[test]
    fn tls_handshake_is_48_bytes() {
        assert_eq!(TLS_HANDSHAKE_EVENT_SIZE, 48);
        assert_eq!(core::mem::align_of::<TlsHandshakeEvent>(), 8);
    }

    #[test]
    fn sock_io_296_with_cgroup_id() {
        assert_eq!(SOCK_IO_EVENT_SIZE, 296);
        assert_eq!(SOCK_LATENCY_EVENT_SIZE, 48);
    }

    // --- Phase 3 TlsIo ABI (Q3 twin, Q4=256) ---

    #[test]
    fn tls_io_is_sock_io_twin() {
        assert_eq!(TLS_IO_EVENT_SIZE, SOCK_IO_EVENT_SIZE);
        assert_eq!(TLS_IO_EVENT_SIZE, 296);
        assert_eq!(core::mem::size_of::<TlsIoEvent>(), core::mem::size_of::<SockIoEvent>());
        assert_eq!(core::mem::align_of::<TlsIoEvent>(), 8);
    }

    #[test]
    fn pending_tls_layout() {
        assert_eq!(core::mem::size_of::<PendingTls>(), 32);
        assert_eq!(core::mem::align_of::<PendingTls>(), 8);
    }

    // --- Phase 6 SockMeta dual-stack ---

    #[test]
    fn sock_meta_layout() {
        assert_eq!(SOCK_META_SIZE, 24);
        assert_eq!(core::mem::align_of::<SockMeta>(), 8);
        let m = SockMeta::with_peer_v4(0x0100007f, 0x5000);
        assert!(m.has_addr());
        assert_eq!(m.family, AF_INET as u8);
        assert_eq!(m.dport_be, 0x5000);
        assert_eq!(m.v4_addr(), Some(0x0100007f));
    }

    #[test]
    fn sock_meta_v6_loopback() {
        let mut addr = [0u8; 16];
        addr[15] = 1;
        let m = SockMeta::with_peer_v6(addr, 0x5000);
        assert!(m.has_addr());
        assert_eq!(m.family, AF_INET6 as u8);
        assert_eq!(m.daddr, addr);
        assert_eq!(m.v4_addr(), None);
    }

    #[test]
    fn sock_meta_sample_flags_do_not_clear_addr() {
        let mut m = SockMeta::with_peer_v4(0x0100007f, 0x5000);
        assert_eq!(m.io_sampled_keep(), None);
        m.set_io_keep(true);
        assert!(m.has_addr());
        assert_eq!(m.io_sampled_keep(), Some(true));
        m.set_io_keep(false);
        assert!(m.has_addr());
        assert_eq!(m.io_sampled_keep(), Some(false));
        assert_eq!(
            SockMeta::FLAG_HAS_ADDR
                | SockMeta::FLAG_SAMPLE_DECIDED
                | SockMeta::FLAG_KEEP_IO,
            0b111
        );
        assert_eq!(DENIED_TGID_ENTRIES, PENDING_MAP_ENTRIES);
        assert_eq!(ALLOWED_TGID_ENTRIES, PENDING_MAP_ENTRIES);
        assert_eq!(SockMeta::keep_from_lookup(None), SockMetaKeep::Unmarked);
        let unmarked = SockMeta::with_peer_v4(1, 2);
        assert_eq!(
            SockMeta::keep_from_lookup(Some(unmarked)),
            SockMetaKeep::Undecided
        );
        let mut kept = unmarked;
        kept.set_io_keep(true);
        assert_eq!(
            SockMeta::keep_from_lookup(Some(kept)),
            SockMetaKeep::Decided(true)
        );
        let peer = SockMeta::with_peer_v4(9, 10);
        let merged = peer.merge_peer_preserve_sample(Some(kept));
        assert!(merged.has_addr());
        assert_eq!(merged.io_sampled_keep(), Some(true));
        assert_eq!(merged.v4_addr(), Some(9));
    }
}
