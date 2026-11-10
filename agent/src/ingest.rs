//! Pure userspace ingest step: reassemble → h2/HTTP1 correlate. No BPF maps, no clocks.
//!
//! The caller applies `clear_inflight` to the kernel `INFLIGHT` map. Keeping that write out
//! of here is what lets recorded event logs replay without BPF (see `replay.rs`).

use std::time::Instant;

use obsagent_common::{IoDir, SockIoEvent};

use crate::correlate::{Correlator, Exchange};
use crate::h2::{looks_like_h2, looks_like_http11, H2Exchange, H2Registry};
use crate::llm::{LlmCall, LlmTracker};
use crate::reassemble::Reassembler;

/// The stateful stages bundled for callers that own them (replay, tests).
/// The live agent keeps them in separate mutexes and calls [`ingest_pure`] directly.
#[derive(Default)]
pub struct IngestState {
    pub reassembler: Reassembler,
    pub h2: H2Registry,
    pub correlator: Correlator,
    pub llm: LlmTracker,
}

impl IngestState {
    pub fn ingest(&mut self, ev: SockIoEvent, now: Instant, client_only: bool) -> Ingested {
        ingest_pure(
            ev,
            now,
            client_only,
            &mut self.reassembler,
            &mut self.h2,
            &mut self.correlator,
            &mut self.llm,
        )
    }
}

#[derive(Default, Debug)]
pub struct Ingested {
    pub exchange: Option<Exchange>,
    /// Streamed LLM calls that finished on this event.
    pub llm: Vec<LlmCall>,
    pub h2: Vec<H2Exchange>,
    /// `(tgid, fd)` keys the caller must remove from the kernel `INFLIGHT` map.
    pub clear_inflight: Vec<(u32, i32)>,
    /// A complete HTTP/1 message was flushed by the reassembler (counter `reasm`).
    pub reassembled: bool,
}

pub fn ingest_pure(
    ev: SockIoEvent,
    now: Instant,
    client_only: bool,
    reassembler: &mut Reassembler,
    h2: &mut H2Registry,
    correlator: &mut Correlator,
    llm: &mut LlmTracker,
) -> Ingested {
    let mut out = Ingested::default();
    let plen = (ev.prefix_len as usize).min(ev.prefix.len());
    let chunk = &ev.prefix[..plen];

    if llm.is_streaming(ev.tgid, ev.fd) {
        let next_request = IoDir::from_u8(ev.dir) == Some(IoDir::Write);
        if let Some(call) = llm.chunk(&ev, now) {
            out.llm.push(call);
            out.clear_inflight.push((ev.tgid, ev.fd));
        }
        // A write is the client's next request: it still goes through the normal path below.
        if !next_request {
            return out;
        }
    }

    if h2.is_marked(ev.tgid, ev.fd) && looks_like_http11(chunk) {
        h2.drop_conn(ev.tgid, ev.fd);
        out.clear_inflight.extend(h2.take_unmark());
    }

    if h2.is_marked(ev.tgid, ev.fd) || looks_like_h2(chunk) {
        out.h2 = h2.feed(&ev, now, client_only);
        out.clear_inflight.extend(h2.take_unmark());
        return out;
    }

    let flushed = reassembler.push(&ev, now);
    out.clear_inflight.extend(reassembler.take_unmark());
    let Some(ev) = flushed else {
        return out;
    };
    out.reassembled = true;
    out.exchange = if client_only {
        correlator.observe_client(&ev, now)
    } else {
        correlator.observe(&ev, now)
    };
    // An LLM SSE stream keeps the kernel INFLIGHT flag so later reads keep arriving.
    let streaming = out.exchange.as_ref().is_some_and(|ex| llm.start(ex, now));
    if !streaming {
        out.clear_inflight.push((ev.tgid, ev.fd));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsagent_common::{EventKind, IoDir, SOCK_IO_PREFIX_LEN};

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
    fn http1_request_response_yields_exchange_and_inflight_clears() {
        let mut st = IngestState::default();
        let now = Instant::now();
        let req = io(
            IoDir::Write,
            7,
            1_000,
            b"GET /slow HTTP/1.1\r\nHost: x\r\n\r\n",
        );
        let resp = io(IoDir::Read, 7, 50_001_000, b"HTTP/1.1 200 OK\r\n\r\n");

        let a = st.ingest(req, now, false);
        assert!(a.exchange.is_none());
        assert!(a.reassembled);
        assert_eq!(a.clear_inflight, vec![(42, 7)]);

        let b = st.ingest(resp, now, false);
        let ex = b.exchange.expect("exchange");
        assert_eq!(ex.latency_ns(), 50_000_000);
        assert!(b.h2.is_empty());
        assert_eq!(b.clear_inflight, vec![(42, 7)]);
    }

    #[test]
    fn split_request_headers_join_before_correlation() {
        let mut st = IngestState::default();
        let now = Instant::now();
        let a = st.ingest(
            io(IoDir::Write, 7, 1_000, b"GET /slow HTTP/1.1\r\n"),
            now,
            false,
        );
        assert!(!a.reassembled);
        assert!(a.clear_inflight.is_empty());
        let b = st.ingest(io(IoDir::Write, 7, 2_000, b"Host: x\r\n\r\n"), now, false);
        assert!(b.reassembled);
        let c = st.ingest(
            io(IoDir::Read, 7, 9_000, b"HTTP/1.1 200 OK\r\n\r\n"),
            now,
            false,
        );
        assert_eq!(c.exchange.expect("exchange").latency_ns(), 9_000 - 1_000);
    }

    #[test]
    fn llm_sse_stream_keeps_inflight_until_stop_then_reports_call() {
        let mut st = IngestState::default();
        let now = Instant::now();
        let tls = |dir, ts, p: &[u8]| {
            let mut e = io(dir, 9, ts, p);
            e.kind = EventKind::TlsIo as u8;
            e
        };
        let req = tls(
            IoDir::Write,
            1_000,
            b"POST /v1/messages HTTP/1.1\r\nHost: api.anthropic.com\r\n\r\n",
        );
        let hdr = tls(
            IoDir::Read,
            300_000_000,
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n",
        );
        let delta = tls(IoDir::Read, 800_000_000, b"4f\r\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":15}}\n\n");
        let stop = tls(
            IoDir::Read,
            900_000_000,
            b"2e\r\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );

        assert_eq!(st.ingest(req, now, true).clear_inflight, vec![(42, 9)]);
        let h = st.ingest(hdr, now, true);
        assert!(h.exchange.is_some(), "TTFB exchange still reported");
        assert!(h.clear_inflight.is_empty(), "stream keeps kernel INFLIGHT");
        let d = st.ingest(delta, now, true);
        assert!(d.llm.is_empty() && d.clear_inflight.is_empty() && !d.reassembled);
        let s = st.ingest(stop, now, true);
        assert_eq!(s.clear_inflight, vec![(42, 9)]);
        let call = &s.llm[0];
        assert_eq!((call.ttfb_ns, call.total_ns), (299_999_000, 899_999_000));
        assert_eq!(call.output_tokens, Some(15));
        assert!(call.complete);
    }

    #[test]
    fn non_http_bytes_are_ignored() {
        let mut st = IngestState::default();
        let out = st.ingest(
            io(IoDir::Write, 7, 1, b"\x16\x03\x01binary"),
            Instant::now(),
            false,
        );
        assert!(out.exchange.is_none() && out.h2.is_empty() && !out.reassembled);
    }
}
