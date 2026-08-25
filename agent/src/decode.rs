//! RingBuf demux (Phase 2 Q5 / Phase 3 Q3): kind byte selects payload layout.

use obsagent_common::{
    EventKind, SOCK_IO_EVENT_SIZE, SOCK_IO_PREFIX_LEN, SOCK_IO_TIMES_EVENT_SIZE,
    SOCK_LATENCY_EVENT_SIZE, SockIoEvent, SockIoTimesEvent, SockLatencyEvent,
    TLS_HANDSHAKE_EVENT_SIZE, TLS_IO_EVENT_SIZE, TlsHandshakeEvent,
};

#[derive(Clone, Copy, Debug)]
pub enum DecodedEvent {
    Latency(SockLatencyEvent),
    Io(SockIoEvent),
    /// Same layout as [`SockIoEvent`]; `kind == TlsIo`.
    TlsIo(SockIoEvent),
    SockIoTimes(SockIoTimesEvent),
    TlsHandshake(TlsHandshakeEvent),
}

impl DecodedEvent {
    pub fn tgid(&self) -> u32 {
        match self {
            Self::Latency(ev) => ev.tgid,
            Self::Io(ev) | Self::TlsIo(ev) => ev.tgid,
            Self::SockIoTimes(ev) => ev.tgid,
            Self::TlsHandshake(ev) => ev.tgid,
        }
    }

    pub fn pid(&self) -> u32 {
        match self {
            Self::Latency(ev) => ev.pid,
            Self::Io(ev) | Self::TlsIo(ev) => ev.pid,
            Self::SockIoTimes(ev) => ev.pid,
            Self::TlsHandshake(ev) => ev.pid,
        }
    }
}

/// Decode a RingBuf record. Returns `None` if too short or unknown kind.
pub fn decode_event(bytes: &[u8]) -> Option<DecodedEvent> {
    let kind = EventKind::from_u8(*bytes.first()?)?;
    match kind {
        EventKind::Connect | EventKind::Accept => {
            if bytes.len() < SOCK_LATENCY_EVENT_SIZE {
                return None;
            }
            // SAFETY: kernel wrote SockLatencyEvent; length checked.
            let ev = unsafe {
                core::ptr::read_unaligned(bytes.as_ptr().cast::<SockLatencyEvent>())
            };
            if ev.kind != kind as u8 {
                return None;
            }
            Some(DecodedEvent::Latency(ev))
        }
        EventKind::SockIo => Some(DecodedEvent::Io(decode_io(bytes, EventKind::SockIo)?)),
        EventKind::TlsIo => Some(DecodedEvent::TlsIo(decode_io(bytes, EventKind::TlsIo)?)),
        EventKind::SockIoTimes => {
            if bytes.len() < SOCK_IO_TIMES_EVENT_SIZE {
                return None;
            }
            let ev = unsafe {
                core::ptr::read_unaligned(bytes.as_ptr().cast::<SockIoTimesEvent>())
            };
            if ev.kind != kind as u8 {
                return None;
            }
            Some(DecodedEvent::SockIoTimes(ev))
        }
        EventKind::TlsHandshake => {
            if bytes.len() < TLS_HANDSHAKE_EVENT_SIZE {
                return None;
            }
            let ev = unsafe {
                core::ptr::read_unaligned(bytes.as_ptr().cast::<TlsHandshakeEvent>())
            };
            if ev.kind != kind as u8 {
                return None;
            }
            Some(DecodedEvent::TlsHandshake(ev))
        }
        EventKind::StackSample => None,
    }
}

fn decode_io(bytes: &[u8], expect: EventKind) -> Option<SockIoEvent> {
    if bytes.len() < SOCK_IO_EVENT_SIZE {
        return None;
    }
    debug_assert_eq!(SOCK_IO_EVENT_SIZE, TLS_IO_EVENT_SIZE);
    let mut ev = unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<SockIoEvent>()) };
    if ev.kind != expect as u8 {
        return None;
    }
    if ev.prefix_len as usize > SOCK_IO_PREFIX_LEN {
        ev.prefix_len = SOCK_IO_PREFIX_LEN as u16;
    }
    let n = ev.prefix_len as usize;
    ev.prefix[n..].fill(0);
    Some(ev)
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsagent_common::{
        EventKind, IoDir, SOCK_IO_EVENT_SIZE, SOCK_IO_PREFIX_LEN, SOCK_IO_TIMES_EVENT_SIZE,
        TLS_HANDSHAKE_EVENT_SIZE, TLS_IO_EVENT_SIZE, SockIoTimesEvent, TlsHandshakeEvent,
    };

    #[test]
    fn decodes_latency_connect() {
        let ev = SockLatencyEvent {
            kind: EventKind::Connect as u8,
            _pad0: [0; 7],
            pid: 1,
            tgid: 2,
            ret: 0,
            latency_ns: 100,
            ts_ns: 200,
            daddr_be: 0,
            dport_be: 0,
            _pad1: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&ev as *const SockLatencyEvent).cast::<u8>(),
                SOCK_LATENCY_EVENT_SIZE,
            )
        };
        match decode_event(bytes) {
            Some(DecodedEvent::Latency(out)) => {
                assert_eq!(out.latency_ns, 100);
                assert_eq!(out.kind, EventKind::Connect as u8);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn decodes_sock_io() {
        let ev = SockIoEvent {
            kind: EventKind::SockIo as u8,
            dir: IoDir::Write as u8,
            prefix_len: 4,
            fd: 5,
            pid: 1,
            tgid: 2,
            ret: 4,
            ts_ns: 99,
            cgroup_id: 0,
            prefix: {
                let mut p = [0u8; SOCK_IO_PREFIX_LEN];
                p[..4].copy_from_slice(b"GET ");
                p
            },
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&ev as *const SockIoEvent).cast::<u8>(),
                SOCK_IO_EVENT_SIZE,
            )
        };
        match decode_event(bytes) {
            Some(DecodedEvent::Io(out)) => {
                assert_eq!(out.fd, 5);
                assert_eq!(out.prefix_len, 4);
                assert_eq!(&out.prefix[..4], b"GET ");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn zeros_bytes_past_prefix_len() {
        let ev = SockIoEvent {
            kind: EventKind::SockIo as u8,
            dir: IoDir::Write as u8,
            prefix_len: 4,
            fd: 1,
            pid: 1,
            tgid: 1,
            ret: 4,
            ts_ns: 0,
            cgroup_id: 0,
            prefix: [0xAA; SOCK_IO_PREFIX_LEN],
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&ev as *const SockIoEvent).cast::<u8>(),
                SOCK_IO_EVENT_SIZE,
            )
        };
        match decode_event(bytes) {
            Some(DecodedEvent::Io(out)) => {
                assert_eq!(&out.prefix[..4], &[0xAA; 4]);
                assert!(out.prefix[4..].iter().all(|&b| b == 0));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn caps_oversize_prefix_len() {
        let ev = SockIoEvent {
            kind: EventKind::SockIo as u8,
            dir: IoDir::Read as u8,
            prefix_len: 9999,
            fd: 1,
            pid: 1,
            tgid: 1,
            ret: 10,
            ts_ns: 0,
            cgroup_id: 0,
            prefix: [0; SOCK_IO_PREFIX_LEN],
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&ev as *const SockIoEvent).cast::<u8>(),
                SOCK_IO_EVENT_SIZE,
            )
        };
        match decode_event(bytes) {
            Some(DecodedEvent::Io(out)) => {
                assert_eq!(out.prefix_len as usize, SOCK_IO_PREFIX_LEN);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn decodes_tls_io() {
        let ev = SockIoEvent {
            kind: EventKind::TlsIo as u8,
            dir: IoDir::Write as u8,
            prefix_len: 4,
            fd: 7,
            pid: 1,
            tgid: 2,
            ret: 4,
            ts_ns: 42,
            cgroup_id: 0,
            prefix: {
                let mut p = [0u8; SOCK_IO_PREFIX_LEN];
                p[..4].copy_from_slice(b"GET ");
                p
            },
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&ev as *const SockIoEvent).cast::<u8>(),
                TLS_IO_EVENT_SIZE,
            )
        };
        match decode_event(bytes) {
            Some(DecodedEvent::TlsIo(out)) => {
                assert_eq!(out.fd, 7);
                assert_eq!(out.kind, EventKind::TlsIo as u8);
                assert_eq!(&out.prefix[..4], b"GET ");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn decodes_sock_io_times() {
        let ev = SockIoTimesEvent {
            kind: EventKind::SockIoTimes as u8,
            dir: IoDir::Write as u8,
            _pad: 0,
            fd: 8,
            pid: 1,
            tgid: 2,
            ret: 16,
            ts_ns: 77,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&ev as *const SockIoTimesEvent).cast::<u8>(),
                SOCK_IO_TIMES_EVENT_SIZE,
            )
        };
        match decode_event(bytes) {
            Some(DecodedEvent::SockIoTimes(out)) => {
                assert_eq!(out.fd, 8);
                assert_eq!(out.ts_ns, 77);
                assert_eq!(out.kind, EventKind::SockIoTimes as u8);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn decodes_tls_handshake() {
        let ev = TlsHandshakeEvent {
            kind: EventKind::TlsHandshake as u8,
            _pad0: [0; 7],
            pid: 1,
            tgid: 2,
            fd: 9,
            _pad1: 0,
            ret: 1,
            latency_ns: 4_000_000,
            ts_ns: 88,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&ev as *const TlsHandshakeEvent).cast::<u8>(),
                TLS_HANDSHAKE_EVENT_SIZE,
            )
        };
        match decode_event(bytes) {
            Some(DecodedEvent::TlsHandshake(out)) => {
                assert_eq!(out.fd, 9);
                assert_eq!(out.latency_ns, 4_000_000);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_short_and_unknown() {
        assert!(decode_event(&[]).is_none());
        assert!(decode_event(&[1, 2, 3]).is_none());
        assert!(decode_event(&[99; SOCK_LATENCY_EVENT_SIZE]).is_none());
    }
}
