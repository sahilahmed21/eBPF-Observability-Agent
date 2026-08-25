//! Sampled OTLP/HTTP JSON traces from completed exchanges (Phase 9).
//!
//! One root span per exchange/stream. Handshake is a one-shot attribute on the
//! first span for that fd — not a child (handshake ends before HTTP starts).
//! Drain never `.await`s the collector.

use std::collections::{HashMap, VecDeque};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::correlate::TIMEOUT;

pub const SPAN_QUEUE_CAP: usize = 1024;
pub const DEFAULT_SAMPLE_N: u64 = 10;

const KIND_SERVER: i32 = 2;
const KIND_CLIENT: i32 = 3;
const STATUS_UNSET: i32 = 0;
const STATUS_ERROR: i32 = 2;

#[derive(Clone, Copy, Debug)]
pub struct TraceMeta {
    pub tgid: u32,
    pub fd: i32,
    pub stream_id: u32,
    pub t_start_ns: u64,
    /// Request half was Write (client). Not the process TLS pairing flag.
    pub is_client: bool,
}

#[derive(Clone, Debug)]
pub struct SpanRec {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub name: String,
    pub start_unix_ns: u128,
    pub end_unix_ns: u128,
    pub status_code: u16,
    pub kind_client: bool,
    pub src: String,
    pub dst: String,
    pub method: String,
    pub route: String,
    pub protocol: String,
    pub wire_ns: Option<u64>,
    pub handshake_ns: Option<u64>,
    pub profile_frames: Option<[String; 5]>,
}

pub fn sample_n_from_env() -> u64 {
    std::env::var("OBSAGENT_TRACE_SAMPLE")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_SAMPLE_N)
}

pub fn sampled(n: u64, meta: TraceMeta) -> bool {
    if n <= 1 {
        return true;
    }
    mix64(meta.tgid, meta.fd, meta.stream_id, meta.t_start_ns) % n == 0
}

pub fn span_ids(meta: TraceMeta) -> ([u8; 16], [u8; 8]) {
    let a = splitmix64(
        meta.t_start_ns ^ ((meta.tgid as u64) << 32) ^ (meta.fd as u32 as u64),
    );
    let b = splitmix64(a ^ ((meta.stream_id as u64) << 32) ^ meta.t_start_ns.rotate_left(17));
    let c = splitmix64(b ^ 0xA5A5_A5A5_A5A5_A5A5);
    let mut trace = [0u8; 16];
    trace[..8].copy_from_slice(&a.to_be_bytes());
    trace[8..].copy_from_slice(&b.to_be_bytes());
    if trace.iter().all(|&x| x == 0) {
        trace[15] = 1;
    }
    let mut span = c.to_be_bytes();
    if span.iter().all(|&x| x == 0) {
        span[7] = 1;
    }
    (trace, span)
}

pub fn protocol_attr(internal: &str) -> &str {
    match internal {
        "http" => "http/1.1",
        other => other,
    }
}

pub fn strip_query(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

pub fn unix_range_from_duration(latency_ns: u64) -> (u128, u128) {
    let end = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let start = end.saturating_sub(u128::from(latency_ns));
    (start, end)
}

pub fn build_span(
    meta: TraceMeta,
    src: &str,
    dst: &str,
    method: &str,
    route: &str,
    protocol: &str,
    status: u16,
    latency_ns: u64,
    wire_ns: Option<u64>,
    handshake_ns: Option<u64>,
    profile_frames: Option<[String; 5]>,
) -> SpanRec {
    let (trace_id, span_id) = span_ids(meta);
    let route = strip_query(route);
    let (start_unix_ns, end_unix_ns) = unix_range_from_duration(latency_ns);
    SpanRec {
        trace_id,
        span_id,
        name: format!("{method} {route}"),
        start_unix_ns,
        end_unix_ns,
        status_code: status,
        kind_client: meta.is_client,
        src: src.to_string(),
        dst: dst.to_string(),
        method: method.to_string(),
        route: route.to_string(),
        protocol: protocol_attr(protocol).to_string(),
        wire_ns,
        handshake_ns,
        profile_frames,
    }
}

pub fn to_otlp_traces_json(spans: &[SpanRec]) -> String {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for s in spans {
        let svc = if s.src.is_empty() {
            "unknown".to_string()
        } else {
            s.src.clone()
        };
        let bucket = groups.entry(svc.clone()).or_insert_with(|| {
            order.push(svc.clone());
            Vec::new()
        });
        bucket.push(span_json(s));
    }
    let resources: Vec<String> = order
        .iter()
        .map(|svc| {
            let body = groups.get(svc).map(|v| v.join(",")).unwrap_or_default();
            format!(
                r#"{{"resource":{{"attributes":[{svc_attr},{sdk_attr}]}},"scopeSpans":[{{"scope":{{"name":"obsagent","version":"0.1.0"}},"spans":[{body}]}}]}}"#,
                svc_attr = kv_str("service.name", svc),
                sdk_attr = kv_str("telemetry.sdk.name", "obsagent"),
            )
        })
        .collect();
    format!(r#"{{"resourceSpans":[{}]}}"#, resources.join(","))
}

fn span_json(s: &SpanRec) -> String {
    let kind = if s.kind_client { KIND_CLIENT } else { KIND_SERVER };
    let status = span_otel_status(&s.protocol, s.status_code);
    let mut attrs = vec![
        kv_str("src", &s.src),
        kv_str("dst", &s.dst),
        kv_str("http.method", &s.method),
        kv_str("http.route", &s.route),
        kv_str("obsagent.protocol", &s.protocol),
    ];
    if s.protocol == "grpc" {
        attrs.push(kv_int("rpc.grpc.status_code", i64::from(s.status_code)));
    } else {
        attrs.push(kv_int("http.status_code", i64::from(s.status_code)));
    }
    if let Some(w) = s.wire_ns {
        attrs.push(kv_int("obsagent.wire_latency_ns", w as i64));
    }
    if let Some(h) = s.handshake_ns {
        attrs.push(kv_int("obsagent.tls.handshake_ns", h as i64));
    }
    let events = profile_events_json(s);
    format!(
        r#"{{"traceId":"{tid}","spanId":"{sid}","name":{name},"kind":{kind},"startTimeUnixNano":"{start}","endTimeUnixNano":"{end}","status":{{"code":{status}}},"attributes":[{attrs}]{events}}}"#,
        tid = hex_lower(&s.trace_id),
        sid = hex_lower(&s.span_id),
        name = json_str(&s.name),
        start = s.start_unix_ns,
        end = s.end_unix_ns,
        attrs = attrs.join(","),
        events = events,
    )
}

fn profile_events_json(s: &SpanRec) -> String {
    let Some(frames) = &s.profile_frames else {
        return String::new();
    };
    let mut attrs: Vec<String> = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        if f.is_empty() {
            break;
        }
        attrs.push(kv_str(&format!("frame.{i}"), f));
    }
    if attrs.is_empty() {
        return String::new();
    }
    format!(
        r#","events":[{{"timeUnixNano":"{end}","name":"profile","attributes":[{attrs}]}}]"#,
        end = s.end_unix_ns,
        attrs = attrs.join(","),
    )
}

fn span_otel_status(protocol: &str, status_code: u16) -> i32 {
    if protocol == "grpc" {
        if status_code != 0 {
            STATUS_ERROR
        } else {
            STATUS_UNSET
        }
    } else if status_code >= 500 {
        STATUS_ERROR
    } else {
        STATUS_UNSET
    }
}

fn kv_str(key: &str, value: &str) -> String {
    format!(
        r#"{{"key":{k},"value":{{"stringValue":{v}}}}}"#,
        k = json_str(key),
        v = json_str(value)
    )
}

fn kv_int(key: &str, value: i64) -> String {
    format!(
        r#"{{"key":{k},"value":{{"intValue":"{value}"}}}}"#,
        k = json_str(key)
    )
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn hex_lower(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(H[(b >> 4) as usize] as char);
        out.push(H[(b & 0xf) as usize] as char);
    }
    out
}

fn mix64(tgid: u32, fd: i32, stream_id: u32, t_start_ns: u64) -> u64 {
    splitmix64(t_start_ns ^ ((tgid as u64) << 32) ^ (fd as u32 as u64) ^ ((stream_id as u64) << 17))
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub struct SpanQueue {
    q: VecDeque<SpanRec>,
    cap: usize,
    pub dropped: u64,
}

impl SpanQueue {
    pub fn new(cap: usize) -> Self {
        Self {
            q: VecDeque::new(),
            cap,
            dropped: 0,
        }
    }

    pub fn push(&mut self, span: SpanRec) -> bool {
        if self.q.len() >= self.cap {
            self.dropped += 1;
            return false;
        }
        self.q.push_back(span);
        true
    }

    pub fn take_all(&mut self) -> Vec<SpanRec> {
        self.q.drain(..).collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.q.len()
    }
}

impl Default for SpanQueue {
    fn default() -> Self {
        Self::new(SPAN_QUEUE_CAP)
    }
}

struct Hs {
    latency_ns: u64,
    ts_ns: u64,
    at: Instant,
}

/// Last successful handshake per fd; consumed once by the first preceding span.
pub struct HandshakeIndex {
    by_fd: HashMap<(u32, i32), Hs>,
}

impl Default for HandshakeIndex {
    fn default() -> Self {
        Self {
            by_fd: HashMap::new(),
        }
    }
}

impl HandshakeIndex {
    pub fn note(&mut self, tgid: u32, fd: i32, latency_ns: u64, ts_ns: u64, now: Instant) {
        if latency_ns == 0 || fd < 0 {
            return;
        }
        self.evict_stale(now);
        self.by_fd.insert(
            (tgid, fd),
            Hs {
                latency_ns,
                ts_ns,
                at: now,
            },
        );
    }

    pub fn peek_preceding(
        &mut self,
        tgid: u32,
        fd: i32,
        t_start_ns: u64,
        now: Instant,
    ) -> Option<u64> {
        self.evict_stale(now);
        let hs = self.by_fd.get(&(tgid, fd))?;
        if hs.ts_ns > t_start_ns {
            return None;
        }
        Some(hs.latency_ns)
    }

    pub fn consume(&mut self, tgid: u32, fd: i32) {
        self.by_fd.remove(&(tgid, fd));
    }

    #[cfg(test)]
    pub fn take_preceding(
        &mut self,
        tgid: u32,
        fd: i32,
        t_start_ns: u64,
        now: Instant,
    ) -> Option<u64> {
        let lat = self.peek_preceding(tgid, fd, t_start_ns, now)?;
        self.consume(tgid, fd);
        Some(lat)
    }

    pub fn evict_stale(&mut self, now: Instant) {
        self.by_fd
            .retain(|_, h| now.duration_since(h.at) <= TIMEOUT);
    }

    pub fn evict_closed(&mut self, still_open: impl Fn(u32, i32) -> bool) {
        self.by_fd.retain(|(tgid, fd), _| still_open(*tgid, *fd));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn meta(t_start: u64) -> TraceMeta {
        TraceMeta {
            tgid: 7,
            fd: 3,
            stream_id: 0,
            t_start_ns: t_start,
            is_client: true,
        }
    }

    fn span_http(status: u16, latency_ns: u64, route: &str) -> SpanRec {
        build_span(
            meta(100),
            "src",
            "dst",
            "GET",
            route,
            "http",
            status,
            latency_ns,
            None,
            None,
            None,
        )
    }

    #[test]
    fn traces_json_parses_and_duration_matches() {
        let s = span_http(200, 50_000_000, "/slow");
        assert_eq!(s.end_unix_ns - s.start_unix_ns, 50_000_000);
        let body = to_otlp_traces_json(&[s]);
        let v: serde_json::Value = serde_json::from_str(&body).expect("json");
        let span = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(span["name"], "GET /slow");
        assert_eq!(span["kind"], KIND_CLIENT);
        assert_eq!(span["status"]["code"], STATUS_UNSET);
        let tid = span["traceId"].as_str().unwrap();
        let sid = span["spanId"].as_str().unwrap();
        assert_eq!(tid.len(), 32);
        assert_eq!(sid.len(), 16);
        assert!(tid.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!body.contains("Authorization"));
        assert!(body.contains("http/1.1"));
        assert!(body.contains("http.status_code"));
        assert!(body.contains("\"service.name\""));
        assert!(body.contains("src"));
        assert!(body.contains("telemetry.sdk.name"));
    }

    #[test]
    fn five_xx_is_error_status() {
        let body = to_otlp_traces_json(&[span_http(503, 1_000, "/x")]);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let span = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(span["status"]["code"], STATUS_ERROR);
    }

    #[test]
    fn grpc_uses_rpc_status_and_strips_query() {
        let s = build_span(
            TraceMeta {
                tgid: 1,
                fd: 4,
                stream_id: 13,
                t_start_ns: 9,
                is_client: true,
            },
            "a",
            "b",
            "POST",
            "/slow.Slow/Sleep?x=1",
            "grpc",
            0,
            10,
            None,
            Some(2_000_000),
            None,
        );
        assert_eq!(s.route, "/slow.Slow/Sleep");
        assert_eq!(s.name, "POST /slow.Slow/Sleep");
        let body = to_otlp_traces_json(&[s]);
        assert!(body.contains("rpc.grpc.status_code"));
        assert!(body.contains("obsagent.tls.handshake_ns"));
        assert!(!body.contains("?x=1"));
    }

    #[test]
    fn ids_are_deterministic() {
        let m = meta(42);
        assert_eq!(span_ids(m), span_ids(m));
        let other = TraceMeta {
            stream_id: 1,
            ..m
        };
        assert_ne!(span_ids(m).0, span_ids(other).0);
    }

    #[test]
    fn sample_n1_always_n10_not_all() {
        let hits: usize = (0..200)
            .filter(|i| {
                sampled(
                    10,
                    TraceMeta {
                        tgid: 1,
                        fd: 1,
                        stream_id: 0,
                        t_start_ns: *i as u64 * 1_000_000,
                        is_client: true,
                    },
                )
            })
            .count();
        assert!(hits > 5 && hits < 60, "hits={hits}");
        assert!(sampled(1, meta(1)));
    }

    #[test]
    fn queue_drops_incoming_when_full() {
        let mut q = SpanQueue::new(2);
        assert!(q.push(span_http(200, 1, "/a")));
        assert!(q.push(span_http(200, 1, "/b")));
        assert!(!q.push(span_http(200, 1, "/c")));
        assert_eq!(q.dropped, 1);
        assert_eq!(q.len(), 2);
        assert_eq!(q.take_all().len(), 2);
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn handshake_consumed_once_if_preceding() {
        let mut idx = HandshakeIndex::default();
        let now = Instant::now();
        idx.note(1, 3, 3_000_000, 50, now);
        assert_eq!(idx.take_preceding(1, 3, 100, now), Some(3_000_000));
        assert_eq!(idx.take_preceding(1, 3, 200, now), None);
        idx.note(1, 3, 1_000, 500, now);
        assert_eq!(idx.take_preceding(1, 3, 100, now), None);
    }

    #[test]
    fn handshake_ttl_evicts() {
        let mut idx = HandshakeIndex::default();
        let now = Instant::now();
        idx.note(1, 3, 3_000_000, 1, now);
        idx.evict_stale(now + TIMEOUT + Duration::from_secs(1));
        assert_eq!(
            idx.take_preceding(1, 3, 10, now + TIMEOUT + Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn span_json_includes_profile_event() {
        let mut frames: [String; 5] = std::array::from_fn(|_| String::new());
        frames[0] = "slow_handler_sleep".into();
        frames[1] = "other_fn".into();
        let mut s = span_http(200, 50_000_000, "/slow");
        s.profile_frames = Some(frames);
        let body = to_otlp_traces_json(&[s]);
        assert!(body.contains("\"name\":\"profile\""));
        assert!(body.contains("slow_handler_sleep"));
    }

    #[test]
    fn grpc_nonzero_status_is_error() {
        let s = build_span(
            TraceMeta {
                tgid: 1,
                fd: 4,
                stream_id: 1,
                t_start_ns: 1,
                is_client: true,
            },
            "a",
            "b",
            "POST",
            "/svc/M",
            "grpc",
            14,
            10,
            None,
            None,
            None,
        );
        let body = to_otlp_traces_json(&[s]);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let span = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(span["status"]["code"], STATUS_ERROR);
    }

    #[test]
    fn server_role_is_server_kind() {
        let s = build_span(
            TraceMeta {
                is_client: false,
                ..meta(1)
            },
            "srv",
            "cli",
            "GET",
            "/",
            "http",
            200,
            1,
            None,
            None,
            None,
        );
        let body = to_otlp_traces_json(&[s]);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let span = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(span["kind"], KIND_SERVER);
        assert_eq!(
            v["resourceSpans"][0]["resource"]["attributes"][0]["value"]["stringValue"],
            "srv"
        );
    }

    #[test]
    fn traces_json_groups_resource_by_src() {
        let a = build_span(meta(1), "frontend", "api", "GET", "/a", "http", 200, 1, None, None, None);
        let b = build_span(meta(2), "api", "db", "GET", "/b", "http", 200, 1, None, None, None);
        let body = to_otlp_traces_json(&[a, b]);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let rs = v["resourceSpans"].as_array().unwrap();
        assert_eq!(rs.len(), 2);
        let names: Vec<&str> = rs
            .iter()
            .map(|r| r["resource"]["attributes"][0]["value"]["stringValue"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["frontend", "api"]);
    }

    #[test]
    fn grafana_queries_pinned_scrape_names() {
        let dash = include_str!("../../deploy/grafana/dashboards/obsagent.json");
        for name in [
            "http_client_duration_milliseconds_bucket",
            "http_client_requests_total",
            "tls_handshake_duration_milliseconds_bucket",
            "obsagent_events_dropped_total",
            "obsagent_traces_sampled_total",
            "obsagent_traces_not_sampled_total",
            "obsagent_traces_dropped_total",
            "obsagent_traces_export_failed_total",
            "http_protocol",
        ] {
            assert!(dash.contains(name), "dashboard missing {name}");
        }
        assert!(!dash.contains("Authorization"));
    }
}
