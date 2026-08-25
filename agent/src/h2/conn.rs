//! Per-(tgid, fd) HTTP/2 leftover + stream pairing (Phase 7).
//!
//! Latency = response HEADERS END_HEADERS `ts_ns` − request HEADERS END_HEADERS
//! `ts_ns`. DATA is ignored. HPACK is one decoder per direction.

use std::collections::HashMap;
use std::time::Instant;

use obsagent_common::{IoDir, SockIoEvent};

use super::frame::{
    headers_fragment, looks_like_h2, FrameHeader, CLIENT_PREFACE, FLAG_END_HEADERS, HEADER_LEN,
    TYPE_CONTINUATION, TYPE_GOAWAY, TYPE_HEADERS, TYPE_RST_STREAM,
};
use super::hpack::Decoder;
use crate::correlate::TIMEOUT;

const LEFTOVER_CAP: usize = 8 * 1024;
const MAX_CONNS: usize = 8192;
const MAX_STREAMS: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ConnKey {
    tgid: u32,
    fd: i32,
}

#[derive(Clone, Debug)]
pub struct H2Exchange {
    pub tgid: u32,
    pub fd: i32,
    pub stream_id: u32,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub t_start_ns: u64,
    pub t_end_ns: u64,
    /// `h2` or `grpc`.
    pub protocol: &'static str,
    /// Direction the request HEADERS arrived on (Write = client).
    pub req_dir: IoDir,
    pub cgroup_id: u64,
}

impl H2Exchange {
    pub fn latency_ns(&self) -> u64 {
        self.t_end_ns.saturating_sub(self.t_start_ns)
    }
}

#[derive(Clone, Debug, Default)]
pub struct H2Stats {
    pub conn_count: usize,
    pub exchanges: u64,
    pub dropped_conns: u64,
    pub dropped_streams: u64,
    pub missing_path: u64,
    pub headers_req: u64,
    pub headers_resp: u64,
    pub unpaired_status: u64,
    pub desync: u64,
}

struct PendingStream {
    method: String,
    path: String,
    t_start_ns: u64,
    req_dir: IoDir,
}

struct DirPipe {
    leftover: Vec<u8>,
    /// Payload bytes still to discard (DATA / oversized frames). Never stored.
    skip: usize,
    decoder: Decoder,
    header_block: Option<(u32, Vec<u8>)>,
}

impl DirPipe {
    fn new() -> Self {
        Self {
            leftover: Vec::new(),
            skip: 0,
            decoder: Decoder::default(),
            header_block: None,
        }
    }
}

struct H2Conn {
    read: DirPipe,
    write: DirPipe,
    streams: HashMap<u32, PendingStream>,
    at: Instant,
}

impl H2Conn {
    fn new(now: Instant) -> Self {
        Self {
            read: DirPipe::new(),
            write: DirPipe::new(),
            streams: HashMap::new(),
            at: now,
        }
    }

    fn pipe_mut(&mut self, dir: IoDir) -> &mut DirPipe {
        match dir {
            IoDir::Read => &mut self.read,
            IoDir::Write => &mut self.write,
        }
    }
}

#[derive(Default)]
pub struct H2Registry {
    conns: HashMap<ConnKey, H2Conn>,
    unmark: Vec<(u32, i32)>,
    pub stats: H2Stats,
}

impl H2Registry {
    pub fn is_marked(&self, tgid: u32, fd: i32) -> bool {
        self.conns.contains_key(&ConnKey { tgid, fd })
    }

    pub fn drop_conn(&mut self, tgid: u32, fd: i32) {
        if self.conns.remove(&ConnKey { tgid, fd }).is_some() {
            self.unmark.push((tgid, fd));
        }
    }

    pub fn take_unmark(&mut self) -> Vec<(u32, i32)> {
        core::mem::take(&mut self.unmark)
    }

    pub fn evict_stale(&mut self, now: Instant) {
        let mut drop = Vec::new();
        self.conns.retain(|k, c| {
            let keep = now.duration_since(c.at) <= TIMEOUT;
            if !keep {
                drop.push((k.tgid, k.fd));
            }
            keep
        });
        self.unmark.extend(drop);
        self.stats.conn_count = self.conns.len();
    }

    /// Drop conns whose BPF `SOCK_META` (or equivalent live check) is gone — fd close/reuse.
    pub fn evict_closed(&mut self, is_live: impl Fn(u32, i32) -> bool) {
        let mut drop = Vec::new();
        self.conns.retain(|k, _| {
            let keep = is_live(k.tgid, k.fd);
            if !keep {
                drop.push((k.tgid, k.fd));
            }
            keep
        });
        self.unmark.extend(drop);
        self.stats.conn_count = self.conns.len();
    }

    /// Append `ev` bytes. Returns completed unary exchanges.
    ///
    /// `client_only`: emit only if request HEADERS arrived on Write (TLS same-host).
    pub fn feed(&mut self, ev: &SockIoEvent, now: Instant, client_only: bool) -> Vec<H2Exchange> {
        let Some(dir) = IoDir::from_u8(ev.dir) else {
            return Vec::new();
        };
        let plen = (ev.prefix_len as usize).min(ev.prefix.len());
        if ev.ret < 0 || plen == 0 {
            return Vec::new();
        }
        let chunk = &ev.prefix[..plen];
        let key = ConnKey {
            tgid: ev.tgid,
            fd: ev.fd,
        };

        // Full client preface on a live conn is a new TCP session (fd reuse), not more of the old HPACK table.
        if chunk.starts_with(CLIENT_PREFACE) {
            self.conns.remove(&key);
        }

        if !self.conns.contains_key(&key) {
            if self.conns.len() >= MAX_CONNS {
                self.stats.dropped_conns += 1;
                return Vec::new();
            }
            if !looks_like_h2(chunk) {
                return Vec::new();
            }
            self.conns.insert(key, H2Conn::new(now));
        }

        let (mut out, goaway, desync, missing, dropped, reqs, resps, unpaired) = {
            let Some(conn) = self.conns.get_mut(&key) else {
                return Vec::new();
            };
            conn.at = now;
            let mut exchanges = Vec::new();
            let outcome = push_bytes(conn, dir, chunk, ev.ts_ns, &mut exchanges);
            (
                exchanges,
                outcome.goaway,
                outcome.desync,
                outcome.missing_path,
                outcome.dropped_streams,
                outcome.headers_req,
                outcome.headers_resp,
                outcome.unpaired_status,
            )
        };
        self.stats.missing_path += missing;
        self.stats.dropped_streams += dropped;
        self.stats.headers_req += reqs;
        self.stats.headers_resp += resps;
        self.stats.unpaired_status += unpaired;
        if desync {
            self.stats.desync += 1;
        }
        for x in &mut out {
            x.tgid = ev.tgid;
            x.fd = ev.fd;
            x.cgroup_id = ev.cgroup_id;
        }
        if client_only {
            out.retain(|x| x.req_dir == IoDir::Write);
        }
        self.stats.exchanges += out.len() as u64;
        if goaway || desync {
            self.conns.remove(&key);
            self.unmark.push((key.tgid, key.fd));
        }
        self.stats.conn_count = self.conns.len();
        out
    }
}

struct FrameOutcome {
    goaway: bool,
    desync: bool,
    missing_path: u64,
    dropped_streams: u64,
    headers_req: u64,
    headers_resp: u64,
    unpaired_status: u64,
}

fn empty_outcome() -> FrameOutcome {
    FrameOutcome {
        goaway: false,
        desync: false,
        missing_path: 0,
        dropped_streams: 0,
        headers_req: 0,
        headers_resp: 0,
        unpaired_status: 0,
    }
}

fn merge_outcome(dst: &mut FrameOutcome, src: FrameOutcome) {
    dst.goaway |= src.goaway;
    dst.desync |= src.desync;
    dst.missing_path += src.missing_path;
    dst.dropped_streams += src.dropped_streams;
    dst.headers_req += src.headers_req;
    dst.headers_resp += src.headers_resp;
    dst.unpaired_status += src.unpaired_status;
}

fn take_skip<'a>(pipe: &mut DirPipe, src: &'a [u8]) -> &'a [u8] {
    if pipe.skip == 0 || src.is_empty() {
        return src;
    }
    let n = src.len().min(pipe.skip);
    pipe.skip -= n;
    &src[n..]
}

/// DATA and control payloads are not stored. HEADERS larger than leftover cap are skipped.
fn skip_payload(hdr: FrameHeader) -> bool {
    match hdr.ty {
        TYPE_HEADERS | TYPE_CONTINUATION => hdr.total_len() > LEFTOVER_CAP,
        _ => true,
    }
}

fn push_bytes(
    conn: &mut H2Conn,
    dir: IoDir,
    mut src: &[u8],
    ts_ns: u64,
    out: &mut Vec<H2Exchange>,
) -> FrameOutcome {
    let mut outcome = empty_outcome();
    loop {
        src = take_skip(conn.pipe_mut(dir), src);
        if src.is_empty() {
            merge_outcome(&mut outcome, drain_frames(conn, dir, ts_ns, out));
            break;
        }
        let room = {
            let pipe = conn.pipe_mut(dir);
            LEFTOVER_CAP.saturating_sub(pipe.leftover.len())
        };
        if room == 0 {
            merge_outcome(&mut outcome, drain_frames(conn, dir, ts_ns, out));
            let pipe = conn.pipe_mut(dir);
            if pipe.skip == 0 && pipe.leftover.len() >= LEFTOVER_CAP {
                outcome.desync = true;
                pipe.leftover.clear();
                pipe.header_block = None;
                pipe.decoder.reset();
                break;
            }
            continue;
        }
        let n = src.len().min(room);
        conn.pipe_mut(dir).leftover.extend_from_slice(&src[..n]);
        src = &src[n..];
        merge_outcome(&mut outcome, drain_frames(conn, dir, ts_ns, out));
        if outcome.goaway || outcome.desync {
            break;
        }
    }
    outcome
}

fn drain_frames(
    conn: &mut H2Conn,
    dir: IoDir,
    ts_ns: u64,
    out: &mut Vec<H2Exchange>,
) -> FrameOutcome {
    let mut buf = core::mem::take(&mut conn.pipe_mut(dir).leftover);
    let mut outcome = empty_outcome();
    if waiting_preface(&buf) {
        conn.pipe_mut(dir).leftover = buf;
        return outcome;
    }
    strip_preface(&mut buf);
    loop {
        if buf.len() < HEADER_LEN {
            break;
        }
        let Some(hdr) = FrameHeader::parse(&buf) else {
            outcome.desync = true;
            buf.clear();
            break;
        };
        let total = hdr.total_len();
        if skip_payload(hdr) {
            if hdr.ty == TYPE_HEADERS || hdr.ty == TYPE_CONTINUATION {
                outcome.missing_path += 1;
            }
            if buf.len() >= total {
                buf.drain(..total);
                apply_control(conn, dir, hdr, &mut outcome);
            } else {
                conn.pipe_mut(dir).skip = total - buf.len();
                buf.clear();
                apply_control(conn, dir, hdr, &mut outcome);
            }
            if outcome.goaway || outcome.desync {
                break;
            }
            continue;
        }
        if buf.len() < total {
            break;
        }
        let payload = buf[HEADER_LEN..total].to_vec();
        buf.drain(..total);
        apply_headers_or_control(conn, dir, hdr, &payload, ts_ns, out, &mut outcome);
        if outcome.goaway || outcome.desync {
            break;
        }
    }
    conn.pipe_mut(dir).leftover = buf;
    outcome
}

fn apply_control(
    conn: &mut H2Conn,
    dir: IoDir,
    hdr: FrameHeader,
    outcome: &mut FrameOutcome,
) {
    match hdr.ty {
        TYPE_RST_STREAM => {
            conn.streams.remove(&hdr.stream_id);
        }
        TYPE_GOAWAY => {
            outcome.goaway = true;
        }
        TYPE_HEADERS | TYPE_CONTINUATION => {
            conn.pipe_mut(dir).header_block = None;
        }
        _ => {}
    }
}

fn apply_headers_or_control(
    conn: &mut H2Conn,
    dir: IoDir,
    hdr: FrameHeader,
    payload: &[u8],
    ts_ns: u64,
    out: &mut Vec<H2Exchange>,
    outcome: &mut FrameOutcome,
) {
    match hdr.ty {
        TYPE_HEADERS => {
            if hdr.flags & FLAG_END_HEADERS != 0 {
                if let Some(frag) = headers_fragment(hdr.flags, payload) {
                    on_header_block(conn, dir, hdr.stream_id, frag, ts_ns, out, outcome);
                }
            } else if let Some(frag) = headers_fragment(hdr.flags, payload) {
                conn.pipe_mut(dir).header_block = Some((hdr.stream_id, frag.to_vec()));
            }
        }
        TYPE_CONTINUATION => {
            let Some((sid, mut acc)) = conn.pipe_mut(dir).header_block.take() else {
                return;
            };
            if sid != hdr.stream_id {
                outcome.desync = true;
                return;
            }
            if acc.len() + payload.len() > LEFTOVER_CAP {
                outcome.missing_path += 1;
                return;
            }
            acc.extend_from_slice(payload);
            if hdr.flags & FLAG_END_HEADERS != 0 {
                on_header_block(conn, dir, sid, &acc, ts_ns, out, outcome);
            } else {
                conn.pipe_mut(dir).header_block = Some((sid, acc));
            }
        }
        TYPE_RST_STREAM => {
            conn.streams.remove(&hdr.stream_id);
        }
        TYPE_GOAWAY => {
            outcome.goaway = true;
        }
        _ => {}
    }
}

fn waiting_preface(buf: &[u8]) -> bool {
    buf.starts_with(b"PRI")
        && buf.len() < CLIENT_PREFACE.len()
        && CLIENT_PREFACE.starts_with(buf)
}

fn strip_preface(buf: &mut Vec<u8>) {
    if buf.starts_with(CLIENT_PREFACE) {
        buf.drain(..CLIENT_PREFACE.len());
    }
}

fn on_header_block(
    conn: &mut H2Conn,
    dir: IoDir,
    stream_id: u32,
    frag: &[u8],
    ts_ns: u64,
    out: &mut Vec<H2Exchange>,
    outcome: &mut FrameOutcome,
) {
    if stream_id == 0 {
        return;
    }
    let decoded = match conn.pipe_mut(dir).decoder.decode_block(frag) {
        Some(p) => p,
        None => {
            outcome.missing_path += 1;
            return;
        }
    };
    if let Some(status) = decoded.status {
        outcome.headers_resp += 1;
        let Some(pending) = conn.streams.remove(&stream_id) else {
            outcome.unpaired_status += 1;
            return;
        };
        if pending.path.is_empty() {
            outcome.missing_path += 1;
            return;
        }
        let protocol = classify_protocol(&pending.path);
        out.push(H2Exchange {
            tgid: 0,
            fd: 0,
            stream_id,
            method: pending.method,
            path: pending.path,
            status,
            t_start_ns: pending.t_start_ns,
            t_end_ns: ts_ns,
            protocol,
            req_dir: pending.req_dir,
            cgroup_id: 0,
        });
        return;
    }
    let Some(method) = decoded.method else {
        return;
    };
    let Some(path) = decoded.path.filter(|p| !p.is_empty()) else {
        outcome.missing_path += 1;
        return;
    };
    if conn.streams.len() >= MAX_STREAMS && !conn.streams.contains_key(&stream_id) {
        outcome.dropped_streams += 1;
        return;
    }
    conn.streams.insert(
        stream_id,
        PendingStream {
            method,
            path,
            t_start_ns: ts_ns,
            req_dir: dir,
        },
    );
    outcome.headers_req += 1;
}

fn classify_protocol(path: &str) -> &'static str {
    let rest = path.strip_prefix('/').unwrap_or(path);
    if rest.contains('.') && rest.contains('/') {
        "grpc"
    } else {
        "h2"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::h2::frame::{FLAG_END_STREAM, TYPE_DATA, TYPE_SETTINGS};
    use obsagent_common::{EventKind, SOCK_IO_PREFIX_LEN};

    fn frame(ty: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
        let n = payload.len() as u32;
        let mut b = vec![
            (n >> 16) as u8,
            (n >> 8) as u8,
            n as u8,
            ty,
            flags,
            (stream >> 24) as u8,
            (stream >> 16) as u8,
            (stream >> 8) as u8,
            stream as u8,
        ];
        b.extend_from_slice(payload);
        b
    }

    fn hpack_post(path: &[u8]) -> Vec<u8> {
        let mut v = vec![0x83, 0x04, path.len() as u8];
        v.extend_from_slice(path);
        v
    }

    fn io(dir: IoDir, ts: u64, bytes: &[u8]) -> SockIoEvent {
        let mut prefix = [0u8; SOCK_IO_PREFIX_LEN];
        let n = bytes.len().min(SOCK_IO_PREFIX_LEN);
        prefix[..n].copy_from_slice(&bytes[..n]);
        SockIoEvent {
            kind: EventKind::SockIo as u8,
            dir: dir as u8,
            prefix_len: n as u16,
            fd: 3,
            pid: 1,
            tgid: 42,
            ret: n as i64,
            ts_ns: ts,
            cgroup_id: 0,
            prefix,
        }
    }

    #[test]
    fn two_streams_no_mispair() {
        let mut r = H2Registry::default();
        let now = Instant::now();
        let mut first = CLIENT_PREFACE.to_vec();
        first.extend(frame(TYPE_SETTINGS, 0, 0, &[]));
        first.extend(frame(
            TYPE_HEADERS,
            FLAG_END_HEADERS,
            1,
            &hpack_post(b"/a.A/One"),
        ));
        r.feed(&io(IoDir::Write, 1_000, &first), now, false);

        r.feed(
            &io(
                IoDir::Write,
                2_000,
                &frame(TYPE_HEADERS, FLAG_END_HEADERS, 3, &hpack_post(b"/b.B/Two")),
            ),
            now,
            false,
        );

        let s1 = r.feed(
            &io(
                IoDir::Read,
                50_000_000,
                &frame(TYPE_HEADERS, FLAG_END_HEADERS | FLAG_END_STREAM, 1, &[0x88]),
            ),
            now,
            false,
        );
        let s3 = r.feed(
            &io(
                IoDir::Read,
                10_000_000,
                &frame(TYPE_HEADERS, FLAG_END_HEADERS | FLAG_END_STREAM, 3, &[0x88]),
            ),
            now,
            false,
        );

        assert_eq!(s1.len(), 1);
        assert_eq!(s3.len(), 1);
        assert_eq!(s1[0].stream_id, 1);
        assert_eq!(s1[0].path, "/a.A/One");
        assert_eq!(s1[0].latency_ns(), 50_000_000 - 1_000);
        assert_eq!(s3[0].stream_id, 3);
        assert_eq!(s3[0].path, "/b.B/Two");
        assert_eq!(s3[0].latency_ns(), 10_000_000 - 2_000);
        assert_eq!(s1[0].protocol, "grpc");
        assert_eq!(s3[0].protocol, "grpc");
    }

    #[test]
    fn rst_drops_pending() {
        let mut r = H2Registry::default();
        let now = Instant::now();
        let mut b = CLIENT_PREFACE.to_vec();
        b.extend(frame(
            TYPE_HEADERS,
            FLAG_END_HEADERS,
            1,
            &hpack_post(b"/x.Y/Z"),
        ));
        r.feed(&io(IoDir::Write, 1, &b), now, false);
        let rst = frame(TYPE_RST_STREAM, 0, 1, &[0, 0, 0, 8]);
        assert!(r.feed(&io(IoDir::Read, 2, &rst), now, false).is_empty());
        let resp = frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &[0x88]);
        assert!(r.feed(&io(IoDir::Read, 3, &resp), now, false).is_empty());
    }

    #[test]
    fn goaway_unmarks() {
        let mut r = H2Registry::default();
        let now = Instant::now();
        r.feed(&io(IoDir::Write, 1, CLIENT_PREFACE), now, false);
        assert!(r.is_marked(42, 3));
        let ga = frame(TYPE_GOAWAY, 0, 0, &[0, 0, 0, 0, 0, 0, 0, 0]);
        r.feed(&io(IoDir::Read, 2, &ga), now, false);
        assert!(!r.is_marked(42, 3));
        assert_eq!(r.take_unmark(), vec![(42, 3)]);
    }

    #[test]
    fn client_only_skips_server_read_request() {
        let mut r = H2Registry::default();
        let now = Instant::now();
        let mut b = CLIENT_PREFACE.to_vec();
        b.extend(frame(
            TYPE_HEADERS,
            FLAG_END_HEADERS,
            1,
            &hpack_post(b"/p.S/M"),
        ));
        r.feed(&io(IoDir::Read, 1, &b), now, true);
        let xs = r.feed(
            &io(
                IoDir::Write,
                50,
                &frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &[0x88]),
            ),
            now,
            true,
        );
        assert!(xs.is_empty());
    }

    #[test]
    fn timeout_unmarks() {
        let mut r = H2Registry::default();
        let t0 = Instant::now();
        r.feed(&io(IoDir::Write, 1, CLIENT_PREFACE), t0, false);
        r.evict_stale(t0 + TIMEOUT + std::time::Duration::from_secs(1));
        assert_eq!(r.take_unmark(), vec![(42, 3)]);
        assert!(!r.is_marked(42, 3));
    }

    #[test]
    fn probe_shaped_write_pairs_status() {
        let mut r = H2Registry::default();
        let now = Instant::now();
        let path = b"/slow.Slow/Sleep";
        let auth = b"127.0.0.1:18097";
        let mut hpack = vec![0x83, 0x86, 0x04, path.len() as u8];
        hpack.extend_from_slice(path);
        hpack.push(0x41);
        hpack.push(auth.len() as u8);
        hpack.extend_from_slice(auth);
        hpack.push(0x5f);
        hpack.extend_from_slice(&[b"application/grpc".len() as u8]);
        hpack.extend_from_slice(b"application/grpc");
        hpack.extend_from_slice(&[0x00, 0x02, b't', b'e', 0x08]);
        hpack.extend_from_slice(b"trailers");
        let mut b = CLIENT_PREFACE.to_vec();
        b.extend(frame(TYPE_SETTINGS, 0, 0, &[]));
        b.extend(frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &hpack));
        let xs = r.feed(&io(IoDir::Write, 1_000, &b), now, false);
        assert!(xs.is_empty());
        assert_eq!(r.stats.headers_req, 1);
        let got = r.feed(
            &io(
                IoDir::Read,
                50_000_000,
                &frame(TYPE_HEADERS, FLAG_END_HEADERS | FLAG_END_STREAM, 1, &[0x88]),
            ),
            now,
            false,
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/slow.Slow/Sleep");
        assert_eq!(got[0].protocol, "grpc");
    }

    fn feed_slices(
        r: &mut H2Registry,
        dir: IoDir,
        ts: u64,
        bytes: &[u8],
        now: Instant,
    ) -> Vec<H2Exchange> {
        let mut out = Vec::new();
        for (i, chunk) in bytes.chunks(SOCK_IO_PREFIX_LEN).enumerate() {
            out.extend(r.feed(&io(dir, ts + i as u64, chunk), now, false));
        }
        out
    }

    #[test]
    fn capture_slices_preface_then_headers_pair() {
        let mut r = H2Registry::default();
        let now = Instant::now();
        let xs = r.feed(&io(IoDir::Write, 1, CLIENT_PREFACE), now, false);
        assert!(xs.is_empty());
        let req = frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &hpack_post(b"/a.A/One"));
        r.feed(&io(IoDir::Write, 2, &req), now, false);
        let got = r.feed(
            &io(
                IoDir::Read,
                50,
                &frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &[0x88]),
            ),
            now,
            false,
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/a.A/One");
    }

    #[test]
    fn skip_16kib_data_then_status_still_pairs() {
        let mut r = H2Registry::default();
        let now = Instant::now();
        let mut open = CLIENT_PREFACE.to_vec();
        open.extend(frame(
            TYPE_HEADERS,
            FLAG_END_HEADERS,
            1,
            &hpack_post(b"/a.A/One"),
        ));
        r.feed(&io(IoDir::Write, 1, &open), now, false);
        let mut raw = vec![0, 0x40, 0, TYPE_DATA, FLAG_END_STREAM, 0, 0, 0, 1];
        raw.extend(vec![0u8; 16 * 1024]);
        assert!(feed_slices(&mut r, IoDir::Write, 10, &raw, now).is_empty());
        let got = r.feed(
            &io(
                IoDir::Read,
                50,
                &frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &[0x88]),
            ),
            now,
            false,
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/a.A/One");
    }

    #[test]
    fn preface_resets_hpack_on_fd_reuse() {
        let mut r = H2Registry::default();
        let now = Instant::now();
        let path = b"/old.S/M";
        let mut first = CLIENT_PREFACE.to_vec();
        let mut blk = vec![0x83, 0x44, path.len() as u8];
        blk.extend_from_slice(path);
        first.extend(frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &blk));
        r.feed(&io(IoDir::Write, 1, &first), now, false);
        r.feed(
            &io(
                IoDir::Read,
                2,
                &frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &[0x88]),
            ),
            now,
            false,
        );
        let mut second = CLIENT_PREFACE.to_vec();
        second.extend(frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &[0x83, 0xBE]));
        r.feed(&io(IoDir::Write, 3, &second), now, false);
        assert!(r.stats.missing_path >= 1);
        let req = frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &hpack_post(b"/new.S/M"));
        r.feed(&io(IoDir::Write, 4, &req), now, false);
        let got = r.feed(
            &io(
                IoDir::Read,
                5,
                &frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &[0x88]),
            ),
            now,
            false,
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/new.S/M");
    }

    #[test]
    fn evict_closed_drops_stale_hpack() {
        let mut r = H2Registry::default();
        let now = Instant::now();
        r.feed(&io(IoDir::Write, 1, CLIENT_PREFACE), now, false);
        assert!(r.is_marked(42, 3));
        r.evict_closed(|_, _| false);
        assert!(!r.is_marked(42, 3));
        assert_eq!(r.take_unmark(), vec![(42, 3)]);
    }

    #[test]
    fn unpaired_status_is_counted() {
        let mut r = H2Registry::default();
        let now = Instant::now();
        r.feed(&io(IoDir::Write, 1, CLIENT_PREFACE), now, false);
        let xs = r.feed(
            &io(
                IoDir::Read,
                2,
                &frame(TYPE_HEADERS, FLAG_END_HEADERS, 1, &[0x88]),
            ),
            now,
            false,
        );
        assert!(xs.is_empty());
        assert_eq!(r.stats.unpaired_status, 1);
        assert_eq!(r.stats.headers_resp, 1);
        assert_eq!(r.stats.exchanges, 0);
    }
}
