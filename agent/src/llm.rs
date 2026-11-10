//! Streamed LLM API calls (SSE) seen as plaintext TLS I/O: time-to-first-byte, time-to-last-event,
//! and token usage.
//!
//! The HTTP/1 correlator ends an exchange at the response *headers*. For `text/event-stream`
//! responses on known LLM paths the ingest step keeps the kernel `INFLIGHT` flag set so the kernel
//! keeps emitting the (≤256 B) head of each later read; this tracker parses those heads.
//!
//! Limits (by design, not bugs): HTTP/1.1 only (h2 `DATA` frames are not parsed); each read is
//! seen only up to 256 B, so a usage object cut by that limit yields `None` (never `0`); no prompt
//! or response text is stored, only the numbers and the `Host`/path.

use std::collections::HashMap;
use std::time::Instant;

use obsagent_common::{IoDir, SockIoEvent};

use crate::correlate::{Exchange, TIMEOUT};

const LLM_PATHS: [&str; 3] = ["/v1/messages", "/v1/chat/completions", "/v1/responses"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmCall {
    pub tgid: u32,
    pub fd: i32,
    pub host: String,
    pub path: String,
    pub status: u16,
    /// Monotonic (`CLOCK_MONOTONIC`) timestamp of the request write; the call ends at
    /// `start_ts_ns + total_ns`, which places it in wall-clock time whenever it is reported.
    pub start_ts_ns: u64,
    /// Request write → response headers.
    pub ttfb_ns: u64,
    /// Request write → last observed stream event.
    pub total_ns: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// A stop marker (`message_stop` / `[DONE]`) was seen.
    pub complete: bool,
}

struct Stream {
    call: LlmCall,
    t_start_ns: u64,
    at: Instant,
}

#[derive(Default)]
pub struct LlmTracker {
    streams: HashMap<(u32, i32), Stream>,
}

impl LlmTracker {
    pub fn is_streaming(&self, tgid: u32, fd: i32) -> bool {
        self.streams.contains_key(&(tgid, fd))
    }

    /// Call with the correlated exchange at response headers. `true` = an LLM SSE stream starts
    /// and the caller must keep the kernel `INFLIGHT` flag for this fd.
    pub fn start(&mut self, ex: &Exchange, now: Instant) -> bool {
        let Some(path) = request_path(&ex.req_prefix) else {
            return false;
        };
        if !LLM_PATHS.contains(&path.as_str()) || !is_sse(&ex.resp_prefix) {
            return false;
        }
        let ttfb_ns = ex.t_end_ns.saturating_sub(ex.t_start_ns);
        let call = LlmCall {
            tgid: ex.tgid,
            fd: ex.fd,
            host: header(&ex.req_prefix, "host").unwrap_or_default(),
            path,
            status: status_code(&ex.resp_prefix),
            start_ts_ns: ex.t_start_ns,
            ttfb_ns,
            total_ns: ttfb_ns,
            input_tokens: None,
            output_tokens: None,
            complete: false,
        };
        self.streams.insert(
            (ex.tgid, ex.fd),
            Stream {
                call,
                t_start_ns: ex.t_start_ns,
                at: now,
            },
        );
        true
    }

    /// Feed a later TLS read/write on a streaming fd. `Some` when the call is finished.
    pub fn chunk(&mut self, ev: &SockIoEvent, now: Instant) -> Option<LlmCall> {
        let key = (ev.tgid, ev.fd);
        let s = self.streams.get_mut(&key)?;
        // A write means the client sent its next request: the stream is over.
        if IoDir::from_u8(ev.dir) == Some(IoDir::Write) {
            return self.streams.remove(&key).map(|s| s.call);
        }
        let n = (ev.prefix_len as usize).min(ev.prefix.len());
        let text = String::from_utf8_lossy(&ev.prefix[..n]);
        s.at = now;
        s.call.total_ns = ev.ts_ns.saturating_sub(s.t_start_ns);
        if let Some(v) =
            u64_after(&text, "input_tokens").or_else(|| u64_after(&text, "prompt_tokens"))
        {
            s.call.input_tokens = Some(v);
        }
        // `message_start` carries a placeholder output_tokens; only deltas are cumulative totals.
        let out = if text.contains("message_delta") {
            u64_after(&text, "output_tokens")
        } else {
            None
        };
        if let Some(v) = out.or_else(|| u64_after(&text, "completion_tokens")) {
            s.call.output_tokens = Some(v);
        }
        if text.contains("message_stop") || text.contains("[DONE]") {
            let mut call = self.streams.remove(&key)?.call;
            call.complete = true;
            return Some(call);
        }
        None
    }

    /// Streams with no event for `TIMEOUT`, reported as incomplete.
    pub fn evict_stale(&mut self, now: Instant) -> Vec<LlmCall> {
        let stale: Vec<(u32, i32)> = self
            .streams
            .iter()
            .filter(|(_, s)| now.duration_since(s.at) > TIMEOUT)
            .map(|(k, _)| *k)
            .collect();
        let mut out: Vec<LlmCall> = stale
            .iter()
            .filter_map(|k| self.streams.remove(k).map(|s| s.call))
            .collect();
        out.sort_by_key(|c| (c.tgid, c.fd));
        out
    }
}

/// Request-line path without the query string.
fn request_path(req: &[u8]) -> Option<String> {
    let line = req.split(|b| *b == b'\r').next()?;
    let mut parts = std::str::from_utf8(line).ok()?.split(' ');
    parts.next()?;
    Some(parts.next()?.split('?').next()?.to_string())
}

fn status_code(resp: &[u8]) -> u16 {
    std::str::from_utf8(&resp[..resp.len().min(16)])
        .ok()
        .and_then(|s| s.split(' ').nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0)
}

/// Case-insensitive header value (trimmed), if present in the captured prefix.
fn header(buf: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(buf);
    text.split("\r\n").skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case(name)
            .then(|| v.trim().to_string())
    })
}

fn is_sse(resp: &[u8]) -> bool {
    header(resp, "content-type")
        .is_some_and(|v| v.to_ascii_lowercase().contains("text/event-stream"))
}

/// First unsigned integer after `"key"` and its colon (spaces allowed).
fn u64_after(text: &str, key: &str) -> Option<u64> {
    let rest = &text[text.find(&format!("\"{key}\""))? + key.len() + 2..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsagent_common::{EventKind, SOCK_IO_PREFIX_LEN};
    use std::time::Duration;

    const REQ: &[u8] = b"POST /v1/messages HTTP/1.1\r\nHost: api.anthropic.com\r\nContent-Type: application/json\r\n\r\n";
    const RESP_SSE: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream; charset=utf-8\r\ntransfer-encoding: chunked\r\n\r\n";
    // Shapes taken from https://platform.claude.com/docs/en/build-with-claude/streaming (2026-11-03).
    const START: &[u8] = b"5a\r\nevent: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}\n\n\r\n";
    const DELTA: &[u8] = b"4f\r\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":15}}\n\n\r\n";
    const DELTA_NO_USAGE: &[u8] =
        b"3f\r\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{}}\n\n\r\n";
    const STOP: &[u8] = b"2e\r\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n\r\n";

    fn io(dir: IoDir, ts_ns: u64, payload: &[u8]) -> SockIoEvent {
        let mut prefix = [0u8; SOCK_IO_PREFIX_LEN];
        let n = payload.len().min(SOCK_IO_PREFIX_LEN);
        prefix[..n].copy_from_slice(&payload[..n]);
        SockIoEvent {
            kind: EventKind::TlsIo as u8,
            dir: dir as u8,
            prefix_len: n as u16,
            fd: 9,
            pid: 1,
            tgid: 42,
            ret: n as i64,
            ts_ns,
            cgroup_id: 0,
            prefix,
        }
    }

    fn exchange(resp: &[u8]) -> Exchange {
        Exchange {
            tgid: 42,
            fd: 9,
            req_prefix: REQ.to_vec(),
            resp_prefix: resp.to_vec(),
            t_start_ns: 1_000,
            t_end_ns: 400_000_000 + 1_000,
            peer: None,
            req_is_write: true,
            cgroup_id: 0,
        }
    }

    #[test]
    fn sse_stream_yields_ttfb_total_and_usage() {
        let mut t = LlmTracker::default();
        let now = Instant::now();
        assert!(t.start(&exchange(RESP_SSE), now));
        assert!(t.is_streaming(42, 9));
        assert!(t.chunk(&io(IoDir::Read, 500_000_000, START), now).is_none());
        assert!(t.chunk(&io(IoDir::Read, 900_000_000, DELTA), now).is_none());
        let call = t
            .chunk(&io(IoDir::Read, 910_000_000, STOP), now)
            .expect("finished");
        assert_eq!(call.host, "api.anthropic.com");
        assert_eq!(call.path, "/v1/messages");
        assert_eq!(call.status, 200);
        assert_eq!(call.ttfb_ns, 400_000_000);
        assert_eq!(call.total_ns, 910_000_000 - 1_000);
        assert_eq!(
            (call.input_tokens, call.output_tokens),
            (Some(25), Some(15))
        );
        assert!(call.complete);
        assert!(!t.is_streaming(42, 9));
    }

    #[test]
    fn missing_usage_is_none_not_zero() {
        let mut t = LlmTracker::default();
        let now = Instant::now();
        t.start(&exchange(RESP_SSE), now);
        t.chunk(&io(IoDir::Read, 2_000, DELTA_NO_USAGE), now);
        let call = t.chunk(&io(IoDir::Read, 3_000, STOP), now).unwrap();
        assert_eq!((call.input_tokens, call.output_tokens), (None, None));
    }

    #[test]
    fn last_cumulative_delta_wins() {
        let mut t = LlmTracker::default();
        let now = Instant::now();
        t.start(&exchange(RESP_SSE), now);
        t.chunk(
            &io(
                IoDir::Read,
                2_000,
                b"x\r\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\n\n",
            ),
            now,
        );
        t.chunk(&io(IoDir::Read, 3_000, DELTA), now);
        let call = t.chunk(&io(IoDir::Read, 4_000, STOP), now).unwrap();
        assert_eq!(call.output_tokens, Some(15));
    }

    #[test]
    fn openai_done_marker_and_usage_names() {
        let mut t = LlmTracker::default();
        let now = Instant::now();
        let mut ex = exchange(RESP_SSE);
        ex.req_prefix =
            b"POST /v1/chat/completions HTTP/1.1\r\nHost: api.openai.com\r\n\r\n".to_vec();
        assert!(t.start(&ex, now));
        t.chunk(&io(IoDir::Read, 2_000, b"9\r\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":22}}\n\n"), now);
        let call = t
            .chunk(&io(IoDir::Read, 3_000, b"e\r\ndata: [DONE]\n\n\r\n"), now)
            .unwrap();
        assert_eq!(
            (call.input_tokens, call.output_tokens),
            (Some(11), Some(22))
        );
        assert!(call.complete);
    }

    #[test]
    fn non_sse_or_non_llm_path_is_not_a_stream() {
        let mut t = LlmTracker::default();
        let now = Instant::now();
        assert!(!t.start(
            &exchange(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n"),
            now
        ));
        let mut ex = exchange(RESP_SSE);
        ex.req_prefix = b"GET /events HTTP/1.1\r\nHost: x\r\n\r\n".to_vec();
        assert!(!t.start(&ex, now));
        assert!(!t.is_streaming(42, 9));
    }

    #[test]
    fn next_request_on_same_fd_ends_stream_incomplete() {
        let mut t = LlmTracker::default();
        let now = Instant::now();
        t.start(&exchange(RESP_SSE), now);
        t.chunk(&io(IoDir::Read, 2_000, START), now);
        let call = t.chunk(&io(IoDir::Write, 9_000, REQ), now).expect("ended");
        assert!(!call.complete);
        assert_eq!(call.input_tokens, Some(25));
        assert!(!t.is_streaming(42, 9));
    }

    #[test]
    fn stale_stream_is_reported_incomplete() {
        let mut t = LlmTracker::default();
        let now = Instant::now();
        t.start(&exchange(RESP_SSE), now);
        assert!(t.evict_stale(now + Duration::from_secs(1)).is_empty());
        let out = t.evict_stale(now + TIMEOUT + Duration::from_secs(1));
        assert_eq!(out.len(), 1);
        assert!(!out[0].complete);
    }
}
