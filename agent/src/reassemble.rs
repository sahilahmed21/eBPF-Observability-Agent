//! Per-(tgid, fd, dir) HTTP/1.1 header reassembly (Phase 6).
//!
//! BPF copies 256 B prefixes. Split writes (`GET /slow HTTP/1.1\r\n` then
//! `Host: …\r\n\r\n`) only become one correlator input after this layer.
//! Completeness is `\r\n\r\n`, not httparse-Complete (that waits for the body).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use obsagent_common::{IoDir, SOCK_IO_PREFIX_LEN, SockIoEvent, http_magic};

use crate::correlate::TIMEOUT;

const CAP: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    tgid: u32,
    fd: i32,
    dir: u8,
}

struct Buf {
    bytes: Vec<u8>,
    first_ts: u64,
    last_ts: u64,
    at: Instant,
    pid: u32,
    kind: u8,
}

#[derive(Default)]
pub struct Reassembler {
    bufs: HashMap<Key, Buf>,
    unmark: Vec<(u32, i32)>,
}

impl Reassembler {
    pub fn push(&mut self, ev: &SockIoEvent, now: Instant) -> Option<SockIoEvent> {
        self.evict_stale(now);
        let dir = IoDir::from_u8(ev.dir)?;
        let plen = (ev.prefix_len as usize).min(ev.prefix.len());
        if ev.ret < 0 || plen == 0 {
            return None;
        }
        let chunk = &ev.prefix[..plen];
        let key = Key {
            tgid: ev.tgid,
            fd: ev.fd,
            dir: dir as u8,
        };

        match self.bufs.remove(&key) {
            None => {
                if !is_http_start(chunk) {
                    return None;
                }
                if headers_done(chunk) || chunk.len() >= CAP {
                    return Some(synthetic(ev, chunk, ev.ts_ns));
                }
                self.bufs.insert(
                    key,
                    Buf {
                        bytes: chunk.to_vec(),
                        first_ts: ev.ts_ns,
                        last_ts: ev.ts_ns,
                        at: now,
                        pid: ev.pid,
                        kind: ev.kind,
                    },
                );
                None
            }
            Some(mut buf) => {
                let room = CAP.saturating_sub(buf.bytes.len());
                buf.bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
                buf.last_ts = ev.ts_ns;
                buf.at = now;
                if headers_done(&buf.bytes) || buf.bytes.len() >= CAP {
                    Some(flush(ev, buf))
                } else {
                    self.bufs.insert(key, buf);
                    None
                }
            }
        }
    }

    pub fn evict_stale(&mut self, now: Instant) {
        let mut drop = Vec::new();
        self.bufs.retain(|k, b| {
            let keep = now.duration_since(b.at) <= TIMEOUT;
            if !keep {
                drop.push((k.tgid, k.fd));
            }
            keep
        });
        self.unmark.extend(drop);
    }

    pub fn take_unmark(&mut self) -> Vec<(u32, i32)> {
        core::mem::take(&mut self.unmark)
    }
}

fn is_http_start(prefix: &[u8]) -> bool {
    http_magic(prefix, prefix.len().min(u16::MAX as usize) as u16)
}

fn headers_done(buf: &[u8]) -> bool {
    buf.windows(4).any(|w| w == b"\r\n\r\n")
}

fn flush(ev: &SockIoEvent, buf: Buf) -> SockIoEvent {
    let ts = if buf.bytes.starts_with(b"HTTP/") {
        buf.last_ts
    } else {
        buf.first_ts
    };
    let mut out = *ev;
    out.kind = buf.kind;
    out.pid = buf.pid;
    synthetic(&out, &buf.bytes, ts)
}

fn synthetic(ev: &SockIoEvent, bytes: &[u8], ts_ns: u64) -> SockIoEvent {
    let n = bytes.len().min(SOCK_IO_PREFIX_LEN);
    let mut prefix = [0u8; SOCK_IO_PREFIX_LEN];
    prefix[..n].copy_from_slice(&bytes[..n]);
    SockIoEvent {
        kind: ev.kind,
        dir: ev.dir,
        prefix_len: n as u16,
        fd: ev.fd,
        pid: ev.pid,
        tgid: ev.tgid,
        ret: n as i64,
        ts_ns,
        cgroup_id: ev.cgroup_id,
        prefix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsagent_common::{EventKind, IoDir};

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
    fn split_header_becomes_one_event() {
        let mut r = Reassembler::default();
        let now = Instant::now();
        assert!(r
            .push(&io(IoDir::Write, 3, 10, b"GET /slow HTTP/1.1\r\n"), now)
            .is_none());
        let out = r
            .push(
                &io(IoDir::Write, 3, 20, b"Host: x\r\n\r\n"),
                now,
            )
            .expect("flush");
        assert!(out.prefix[..out.prefix_len as usize].starts_with(b"GET /slow"));
        assert!(out.prefix[..out.prefix_len as usize].windows(4).any(|w| w == b"\r\n\r\n"));
        assert_eq!(out.ts_ns, 10);
        assert_eq!(r.bufs.len(), 0);
    }

    #[test]
    fn complete_first_chunk_passthrough() {
        let mut r = Reassembler::default();
        let now = Instant::now();
        let out = r
            .push(
                &io(
                    IoDir::Write,
                    3,
                    5,
                    b"GET / HTTP/1.1\r\nHost: x\r\n\r\n",
                ),
                now,
            )
            .expect("pass");
        assert_eq!(out.ts_ns, 5);
        assert_eq!(r.bufs.len(), 0);
    }

    #[test]
    fn dirs_do_not_mix() {
        let mut r = Reassembler::default();
        let now = Instant::now();
        assert!(r
            .push(&io(IoDir::Write, 3, 1, b"GET /a HTTP/1.1\r\n"), now)
            .is_none());
        let resp = r
            .push(
                &io(IoDir::Read, 3, 2, b"HTTP/1.1 200 OK\r\n\r\n"),
                now,
            )
            .expect("resp complete");
        assert!(resp.prefix.starts_with(b"HTTP/"));
        assert_eq!(r.bufs.len(), 1);
    }

    #[test]
    fn timeout_evicts() {
        let mut r = Reassembler::default();
        let t0 = Instant::now();
        r.push(&io(IoDir::Write, 1, 1, b"GET / HTTP/1.1\r\n"), t0);
        assert_eq!(r.bufs.len(), 1);
        let t1 = t0 + TIMEOUT + Duration::from_secs(1);
        r.push(&io(IoDir::Write, 2, 2, b"GET /b HTTP/1.1\r\n"), t1);
        assert_eq!(r.bufs.len(), 1);
        assert_eq!(r.take_unmark(), vec![(42, 1)]);
    }

    #[test]
    fn ignores_non_http() {
        let mut r = Reassembler::default();
        let now = Instant::now();
        assert!(r
            .push(&io(IoDir::Write, 3, 1, b"SSH-2.0-OpenSSH"), now)
            .is_none());
        assert_eq!(r.bufs.len(), 0);
    }

    #[test]
    fn response_ts_is_last_chunk() {
        let mut r = Reassembler::default();
        let now = Instant::now();
        assert!(r
            .push(&io(IoDir::Read, 4, 100, b"HTTP/1.1 200 OK\r\n"), now)
            .is_none());
        let out = r
            .push(&io(IoDir::Read, 4, 250, b"Content-Length: 0\r\n\r\n"), now)
            .expect("resp");
        assert_eq!(out.ts_ns, 250);
    }
}
