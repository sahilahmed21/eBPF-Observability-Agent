//! Per-socket HTTP correlation state machine (Phase 2).
//!
//! See `docs/architecture/correlation.md`. Key = `(tgid, fd)` (process fd table).
//! Latency (Q8) = response-half exit `ts_ns` − request-half exit `ts_ns`.
//! Stale states evicted after [`TIMEOUT`] (Q11 = 60s).
//!
//! # Limits (documented, not solved)
//! - No TCP reassembly: only the first HTTP-looking chunk per half is paired.
//! - Partial `recvfrom` slices that do not start with a method/`HTTP/` are dropped
//!   in-kernel; mid-stream chunks never join an exchange.
//! - Server responses via `sendmsg`/`writev` are not probed (Phase 2 attach set).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use obsagent_common::{IoDir, SockIoEvent};

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

#[derive(Clone, Debug)]
struct Pending {
    dir: IoDir,
    ts_ns: u64,
    prefix: Vec<u8>,
    at: Instant,
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
    pub fn observe(&mut self, ev: &SockIoEvent, now: Instant) -> Option<Exchange> {
        self.evict_stale(now);

        let dir = IoDir::from_u8(ev.dir)?;
        let key = SockKey::from_event(ev);
        let plen = (ev.prefix_len as usize).min(ev.prefix.len());
        let prefix = ev.prefix[..plen].to_vec();

        match self.states.remove(&key) {
            None => {
                if ev.ret < 0 || plen == 0 || !looks_like_request(&prefix) {
                    // Only start an exchange on a request-looking half.
                    return None;
                }
                self.states.insert(
                    key,
                    Pending {
                        dir,
                        ts_ns: ev.ts_ns,
                        prefix,
                        at: now,
                    },
                );
                None
            }
            Some(pending) => {
                let opposite = matches!(
                    (pending.dir, dir),
                    (IoDir::Write, IoDir::Read) | (IoDir::Read, IoDir::Write)
                );
                if !opposite || ev.ret < 0 || !looks_like_response(&prefix) {
                    // Restart only if this event is a new request half.
                    if ev.ret >= 0 && looks_like_request(&prefix) {
                        self.states.insert(
                            key,
                            Pending {
                                dir,
                                ts_ns: ev.ts_ns,
                                prefix,
                                at: now,
                            },
                        );
                    }
                    return None;
                }

                // pending must already be request-shaped (enforced on insert).
                Some(Exchange {
                    tgid: key.tgid,
                    fd: key.fd,
                    req_prefix: pending.prefix,
                    resp_prefix: prefix,
                    t_start_ns: pending.ts_ns,
                    t_end_ns: ev.ts_ns,
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
