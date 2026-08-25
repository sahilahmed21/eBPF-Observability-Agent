//! Per-socket HTTP correlation state machine (Phase 2 + Phase 6 reassembly).
//!
//! See `docs/architecture/correlation.md`. Key = `(tgid, fd)` (process fd table).
//! Latency (Q8) = response-half exit `ts_ns` − request-half exit `ts_ns`.
//! Stale states evicted after [`TIMEOUT`] (Q11 = 60s).
//!
//! # Limits (documented, not solved)
//! - Split writes are joined by [`crate::reassemble`] before this SM sees them.
//! - First iovec only (256 B); HTTP sitting only in iov[1+] is still invisible.
//! - `sendmsg`/`writev` are probed (Phase 6).
//! - TLS (`observe_client`): only write→read pairing so same-host OpenSSL
//!   client+server does not double-count HTTP rates.
//! - Peer address is joined from `SOCK_META` after emit (Phase 4 Q1); correlator
//!   itself does not look up maps.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use obsagent_common::{AF_INET, AF_INET6, IoDir, SockIoEvent, SockMeta};

/// Q11: align with rolling agg window.
pub const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SockKey {
    pub tgid: u32,
    pub fd: i32,
}

impl SockKey {
    pub fn from_event(ev: &SockIoEvent) -> Self {
        Self {
            tgid: ev.tgid,
            fd: ev.fd,
        }
    }
}

/// Peer from `SOCK_META` (network-order port; v4/v6 address bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PeerAddr {
    pub family: u8,
    pub dport_be: u16,
    pub daddr: [u8; 16],
}

impl PeerAddr {
    pub fn from_v4(daddr_be: u32, dport_be: u16) -> Self {
        Self::from_meta_unchecked(SockMeta::with_peer_v4(daddr_be, dport_be))
    }

    pub fn from_meta(m: SockMeta) -> Option<Self> {
        if m.has_addr() {
            Some(Self::from_meta_unchecked(m))
        } else {
            None
        }
    }

    fn from_meta_unchecked(m: SockMeta) -> Self {
        Self {
            family: m.family,
            dport_be: m.dport_be,
            daddr: m.daddr,
        }
    }

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

    pub fn is_v6(self) -> bool {
        self.family == AF_INET6 as u8
    }
}

#[derive(Clone, Debug)]
struct Pending {
    dir: IoDir,
    ts_ns: u64,
    prefix: Vec<u8>,
    at: Instant,
    cgroup_id: u64,
}

/// Completed HTTP exchange (raw prefixes; parse later).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exchange {
    pub tgid: u32,
    pub fd: i32,
    pub req_prefix: Vec<u8>,
    pub resp_prefix: Vec<u8>,
    pub t_start_ns: u64,
    pub t_end_ns: u64,
    /// Filled by userspace from `SOCK_META` after correlation (Phase 4 Q1).
    pub peer: Option<PeerAddr>,
    /// First half was Write (client). False = server read→write.
    pub req_is_write: bool,
    /// From request-half `SockIoEvent.cgroup_id` (k8s src identity).
    pub cgroup_id: u64,
}

impl Exchange {
    pub fn latency_ns(&self) -> u64 {
        self.t_end_ns.saturating_sub(self.t_start_ns)
    }
}

#[derive(Default)]
pub struct Correlator {
    states: HashMap<SockKey, Pending>,
}

impl Correlator {
    /// Cleartext sock I/O: accept client (write→read) or server (read→write) pairing.
    pub fn observe(&mut self, ev: &SockIoEvent, now: Instant) -> Option<Exchange> {
        self.observe_inner(ev, now, false)
    }

    /// TLS I/O: client-only (write→read). Same-host OpenSSL client+server both emit
    /// TlsIo; counting both doubles HTTP rates. Server read→write is ignored.
    pub fn observe_client(&mut self, ev: &SockIoEvent, now: Instant) -> Option<Exchange> {
        self.observe_inner(ev, now, true)
    }

    fn observe_inner(
        &mut self,
        ev: &SockIoEvent,
        now: Instant,
        client_only: bool,
    ) -> Option<Exchange> {
        self.evict_stale(now);

        let dir = IoDir::from_u8(ev.dir)?;
        let key = SockKey::from_event(ev);
        let plen = (ev.prefix_len as usize).min(ev.prefix.len());
        let prefix = ev.prefix[..plen].to_vec();

        match self.states.remove(&key) {
            None => {
                if ev.ret < 0 || plen == 0 || !looks_like_request(&prefix) {
                    return None;
                }
                if client_only && dir != IoDir::Write {
                    return None;
                }
                self.states.insert(
                    key,
                    Pending {
                        dir,
                        ts_ns: ev.ts_ns,
                        prefix,
                        at: now,
                        cgroup_id: ev.cgroup_id,
                    },
                );
                None
            }
            Some(pending) => {
                let opposite = if client_only {
                    matches!((pending.dir, dir), (IoDir::Write, IoDir::Read))
                } else {
                    matches!(
                        (pending.dir, dir),
                        (IoDir::Write, IoDir::Read) | (IoDir::Read, IoDir::Write)
                    )
                };
                if !opposite || ev.ret < 0 || !looks_like_response(&prefix) {
                    if ev.ret >= 0
                        && looks_like_request(&prefix)
                        && (!client_only || dir == IoDir::Write)
                    {
                        self.states.insert(
                            key,
                            Pending {
                                dir,
                                ts_ns: ev.ts_ns,
                                prefix,
                                at: now,
                                cgroup_id: ev.cgroup_id,
                            },
                        );
                    }
                    return None;
                }

                Some(Exchange {
                    tgid: key.tgid,
                    fd: key.fd,
                    req_prefix: pending.prefix,
                    resp_prefix: prefix,
                    t_start_ns: pending.ts_ns,
                    t_end_ns: ev.ts_ns,
                    peer: None,
                    req_is_write: pending.dir == IoDir::Write,
                    cgroup_id: pending.cgroup_id,
                })
            }
        }
    }

    fn evict_stale(&mut self, now: Instant) {
        self.states
            .retain(|_, s| now.duration_since(s.at) <= TIMEOUT);
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.states.len()
    }
}

fn looks_like_request(prefix: &[u8]) -> bool {
    prefix.starts_with(b"GET ")
        || prefix.starts_with(b"POST ")
        || prefix.starts_with(b"PUT ")
        || prefix.starts_with(b"HEAD ")
        || prefix.starts_with(b"DELETE ")
        || prefix.starts_with(b"OPTIONS ")
        || prefix.starts_with(b"PATCH ")
}

fn looks_like_response(prefix: &[u8]) -> bool {
    prefix.starts_with(b"HTTP/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsagent_common::{EventKind, SOCK_IO_PREFIX_LEN};

    fn io(dir: IoDir, fd: i32, ts_ns: u64, payload: &[u8]) -> SockIoEvent {
        let mut prefix = [0u8; SOCK_IO_PREFIX_LEN];
        let n = payload.len().min(SOCK_IO_PREFIX_LEN);
        prefix[..n].copy_from_slice(&payload[..n]);
        SockIoEvent {
            kind: EventKind::SockIo as u8,
            dir: dir as u8,
            prefix_len: n as u16,
            fd,
            pid: 1,
            tgid: 42,
            ret: n as i64,
            ts_ns,
            cgroup_id: 0,
            prefix,
        }
    }

    #[test]
    fn client_write_then_read_emits_latency() {
        let mut c = Correlator::default();
        let now = Instant::now();
        assert!(c
            .observe(&io(IoDir::Write, 3, 1_000, b"GET / HTTP/1.1\r\n"), now)
            .is_none());
        let ex = c
            .observe(
                &io(IoDir::Read, 3, 1_050_000_000, b"HTTP/1.1 200 OK\r\n"),
                now,
            )
            .expect("exchange");
        assert_eq!(ex.latency_ns(), 1_050_000_000 - 1_000);
        assert!(ex.req_is_write);
        assert!(ex.req_prefix.starts_with(b"GET "));
        assert!(ex.resp_prefix.starts_with(b"HTTP/"));
        assert_eq!(c.pending_len(), 0);
    }

    #[test]
    fn server_read_then_write_emits_latency() {
        let mut c = Correlator::default();
        let now = Instant::now();
        assert!(c
            .observe(&io(IoDir::Read, 7, 10, b"POST /x HTTP/1.1\r\n"), now)
            .is_none());
        let ex = c
            .observe(&io(IoDir::Write, 7, 500, b"HTTP/1.1 201 Created\r\n"), now)
            .expect("exchange");
        assert_eq!(ex.latency_ns(), 490);
        assert!(!ex.req_is_write);
    }

    #[test]
    fn client_only_ignores_server_read_then_write() {
        let mut c = Correlator::default();
        let now = Instant::now();
        assert!(c
            .observe_client(&io(IoDir::Read, 7, 10, b"POST /x HTTP/1.1\r\n"), now)
            .is_none());
        assert!(c
            .observe_client(
                &io(IoDir::Write, 7, 500, b"HTTP/1.1 201 Created\r\n"),
                now,
            )
            .is_none());
        assert_eq!(c.pending_len(), 0);
    }

    #[test]
    fn client_only_write_then_read_emits() {
        let mut c = Correlator::default();
        let now = Instant::now();
        assert!(c
            .observe_client(&io(IoDir::Write, 3, 1_000, b"GET / HTTP/1.1\r\n"), now)
            .is_none());
        let ex = c
            .observe_client(
                &io(IoDir::Read, 3, 1_050_000_000, b"HTTP/1.1 200 OK\r\n"),
                now,
            )
            .expect("exchange");
        assert_eq!(ex.latency_ns(), 1_050_000_000 - 1_000);
    }

    #[test]
    fn rejects_response_as_first_half() {
        let mut c = Correlator::default();
        let now = Instant::now();
        assert!(c
            .observe(&io(IoDir::Read, 3, 1, b"HTTP/1.1 200 OK\r\n"), now)
            .is_none());
        assert_eq!(c.pending_len(), 0);
    }

    #[test]
    fn rejects_non_response_second_half() {
        let mut c = Correlator::default();
        let now = Instant::now();
        c.observe(&io(IoDir::Write, 3, 1, b"GET /a HTTP/1.1\r\n"), now);
        assert!(c
            .observe(&io(IoDir::Read, 3, 2, b"GET /b HTTP/1.1\r\n"), now)
            .is_none());
        // Second GET restarts pending as request.
        assert_eq!(c.pending_len(), 1);
    }

    #[test]
    fn timeout_evicts_pending() {
        let mut c = Correlator::default();
        let t0 = Instant::now();
        c.observe(&io(IoDir::Write, 1, 1, b"GET / HTTP/1.1\r\n"), t0);
        assert_eq!(c.pending_len(), 1);
        let t1 = t0 + TIMEOUT + Duration::from_secs(1);
        c.observe(&io(IoDir::Write, 2, 2, b"GET / HTTP/1.1\r\n"), t1);
        assert_eq!(c.pending_len(), 1);
    }

    #[test]
    fn same_direction_restarts() {
        let mut c = Correlator::default();
        let now = Instant::now();
        c.observe(&io(IoDir::Write, 3, 1, b"GET /a HTTP/1.1\r\n"), now);
        assert!(c
            .observe(&io(IoDir::Write, 3, 2, b"GET /b HTTP/1.1\r\n"), now)
            .is_none());
        let ex = c
            .observe(&io(IoDir::Read, 3, 100, b"HTTP/1.1 200 OK\r\n"), now)
            .expect("exchange");
        assert!(ex.req_prefix.starts_with(b"GET /b"));
    }
}
