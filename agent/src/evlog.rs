//! Event log: raw RingBuf records, length-prefixed, for deterministic replay (see `replay.rs`).
//!
//! Format: `OBSEVL1\n` then repeated `[u32 LE len][len bytes]`. Recording is opt-in
//! (`OBSAGENT_RECORD=<path>`) because records hold plaintext prefixes. Header values for
//! secrets are overwritten with `*` before writing; HTTP/2 header blocks are HPACK-encoded
//! and are NOT redacted, so record h2 only from test traffic.

use std::io::{self, Read, Write};

use obsagent_common::{EventKind, SOCK_IO_EVENT_SIZE, SOCK_IO_PREFIX_LEN, SockIoEvent};

use crate::http::redact_headers;

pub const MAGIC: &[u8; 8] = b"OBSEVL1\n";

pub struct Writer<W: Write> {
    w: W,
}

impl<W: Write> Writer<W> {
    pub fn new(mut w: W) -> io::Result<Self> {
        w.write_all(MAGIC)?;
        Ok(Self { w })
    }

    pub fn write(&mut self, rec: &[u8]) -> io::Result<()> {
        let mut buf = rec.to_vec();
        redact_record(&mut buf);
        self.w.write_all(&(buf.len() as u32).to_le_bytes())?;
        self.w.write_all(&buf)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }
}

fn redact_record(buf: &mut [u8]) {
    let is_io = buf
        .first()
        .and_then(|k| EventKind::from_u8(*k))
        .is_some_and(|k| matches!(k, EventKind::SockIo | EventKind::TlsIo));
    if is_io && buf.len() >= SOCK_IO_EVENT_SIZE {
        let off = core::mem::offset_of!(SockIoEvent, prefix);
        redact_headers(&mut buf[off..off + SOCK_IO_PREFIX_LEN]);
    }
}

pub fn read_all(mut r: impl Read) -> io::Result<Vec<Vec<u8>>> {
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not an obsagent event log"));
    }
    let mut out = Vec::new();
    loop {
        let mut len = [0u8; 4];
        match r.read_exact(&mut len) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(out),
            Err(e) => return Err(e),
        }
        let mut rec = vec![0u8; u32::from_le_bytes(len) as usize];
        r.read_exact(&mut rec)?;
        out.push(rec);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsagent_common::{IoDir, SockIoEvent};

    pub fn io_bytes(kind: EventKind, dir: IoDir, fd: i32, ts_ns: u64, payload: &[u8]) -> Vec<u8> {
        let mut prefix = [0u8; SOCK_IO_PREFIX_LEN];
        let n = payload.len().min(SOCK_IO_PREFIX_LEN);
        prefix[..n].copy_from_slice(&payload[..n]);
        let ev = SockIoEvent {
            kind: kind as u8,
            dir: dir as u8,
            prefix_len: n as u16,
            fd,
            pid: 1,
            tgid: 42,
            ret: n as i64,
            ts_ns,
            cgroup_id: 0,
            prefix,
        };
        // SAFETY: SockIoEvent is repr(C) plain data; test-only byte view.
        unsafe {
            core::slice::from_raw_parts((&ev as *const SockIoEvent).cast::<u8>(), SOCK_IO_EVENT_SIZE)
        }
        .to_vec()
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn roundtrip_preserves_records() {
        let rec = io_bytes(EventKind::SockIo, IoDir::Write, 7, 5, b"GET /a HTTP/1.1\r\nHost: x\r\n\r\n");
        let mut buf = Vec::new();
        let mut w = Writer::new(&mut buf).unwrap();
        w.write(&rec).unwrap();
        w.write(&rec).unwrap();
        let got = read_all(&buf[..]).unwrap();
        assert_eq!(got, vec![rec.clone(), rec]);
    }

    #[test]
    fn secrets_never_reach_the_log() {
        let req = b"POST /v1/messages HTTP/1.1\r\nx-api-key: sk-ant-SECRET1\r\nAuthorization: Bearer SECRET2\r\ncookie: a=SECRET3\r\nCookie: b=SECRET4\r\n\r\n";
        for kind in [EventKind::SockIo, EventKind::TlsIo] {
            let rec = io_bytes(kind, IoDir::Write, 7, 5, req);
            let mut buf = Vec::new();
            Writer::new(&mut buf).unwrap().write(&rec).unwrap();
            for secret in [&b"SECRET1"[..], b"SECRET2", b"SECRET3", b"SECRET4"] {
                assert!(!contains(&buf, secret), "{kind:?} leaked {secret:?}");
            }
            assert!(contains(&buf, b"POST /v1/messages"), "non-secret bytes kept");
        }
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(read_all(&b"NOTALOG!"[..]).is_err());
    }
}
