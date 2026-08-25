//! Cumulative metrics registry (Phase 5).
//!
//! Aggregates in-process, then OTLP flush exports snapshots. Replaces
//! gauge-per-sample export (Phase 4 defect vs locked Q4 / Phase 5 Q1–Q3).

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Explicit latency bucket upper bounds in milliseconds (Phase 5 Q1).
pub const LATENCY_BOUNDS_MS: &[f64] = &[
    1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0,
    10000.0,
];

/// Max distinct HTTP series before overflow (Phase 5 Q6).
pub const MAX_SERIES: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SeriesKey {
    pub src: String,
    pub dst: String,
    pub method: String,
    pub route: String,
    pub status_class: String,
    pub protocol: String,
}

impl SeriesKey {
    pub fn other() -> Self {
        Self {
            src: "_other".into(),
            dst: "_other".into(),
            method: "_".into(),
            route: "_".into(),
            status_class: "other".into(),
            protocol: "_".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct HttpObs {
    pub src: String,
    pub dst: String,
    pub method: String,
    pub route: String,
    pub latency_ns: u64,
    pub status: u16,
    pub protocol: String,
    pub wire_ns: Option<u64>,
}

#[derive(Clone, Debug)]
struct SeriesStats {
    count: u64,
    sum_ms: f64,
    /// Counts per explicit bound + final +Inf bucket.
    buckets: Vec<u64>,
    errors: u64,
}

impl SeriesStats {
    fn new() -> Self {
        Self {
            count: 0,
            sum_ms: 0.0,
            buckets: vec![0; LATENCY_BOUNDS_MS.len() + 1],
            errors: 0,
        }
    }

    fn record(&mut self, latency_ns: u64, status: u16) {
        let ms = latency_ns as f64 / 1_000_000.0;
        self.count += 1;
        self.sum_ms += ms;
        if status >= 500 {
            self.errors += 1;
        }
        let mut placed = false;
        for (i, bound) in LATENCY_BOUNDS_MS.iter().enumerate() {
            if ms <= *bound {
                self.buckets[i] += 1;
                placed = true;
                break;
            }
        }
        if !placed {
            *self.buckets.last_mut().unwrap() += 1;
        }
    }
}

/// Cumulative since process start (Prometheus-friendly restart semantics).
#[derive(Clone)]
pub struct MetricsRegistry {
    series: HashMap<SeriesKey, SeriesStats>,
    overflow_count: u64,
    start_unix_nano: u128,
    /// Agent self-metrics (updated by callers).
    pub events_dropped: u64,
    pub otlp_dropped: u64,
    pub otlp_flushes_ok: u64,
    pub edge_count: u64,
    pub tls_unmapped: u64,
    pub traces_sampled: u64,
    pub traces_not_sampled: u64,
    pub traces_dropped: u64,
    pub traces_export_failed: u64,
    /// Current BPF SAMPLE_N (gauge, not a counter).
    pub sample_n: u32,
    wire: HashMap<SeriesKey, SeriesStats>,
    handshake: SeriesStats,
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            series: HashMap::new(),
            overflow_count: 0,
            start_unix_nano: now_unix_nano(),
            events_dropped: 0,
            otlp_dropped: 0,
            otlp_flushes_ok: 0,
            edge_count: 0,
            tls_unmapped: 0,
            traces_sampled: 0,
            traces_not_sampled: 0,
            traces_dropped: 0,
            traces_export_failed: 0,
            sample_n: 1,
            wire: HashMap::new(),
            handshake: SeriesStats::new(),
        }
    }

    pub fn record(&mut self, obs: &HttpObs) {
        let key = SeriesKey {
            src: obs.src.clone(),
            dst: obs.dst.clone(),
            method: obs.method.clone(),
            route: obs.route.clone(),
            status_class: status_class(obs.status),
            protocol: obs.protocol.clone(),
        };
        let key = if !self.series.contains_key(&key) && self.series.len() >= MAX_SERIES {
            self.overflow_count += 1;
            SeriesKey::other()
        } else {
            key
        };
        self.series
            .entry(key.clone())
            .or_insert_with(SeriesStats::new)
            .record(obs.latency_ns, obs.status);
        if let Some(w) = obs.wire_ns {
            self.wire
                .entry(key)
                .or_insert_with(SeriesStats::new)
                .record(w, obs.status);
        }
    }

    pub fn record_handshake(&mut self, latency_ns: u64) {
        self.handshake.record(latency_ns, 200);
    }

    pub fn series_len(&self) -> usize {
        self.series.len()
    }

    pub fn overflow_count(&self) -> u64 {
        self.overflow_count
    }

    pub fn total_requests(&self) -> u64 {
        self.series.values().map(|s| s.count).sum()
    }

    /// Whether a metrics POST would carry anything besides zeroed agent counters.
    pub fn has_otlp_payload(&self) -> bool {
        self.total_requests() > 0
            || self.events_dropped > 0
            || self.tls_unmapped > 0
            || self.traces_sampled > 0
            || self.traces_not_sampled > 0
            || self.traces_dropped > 0
            || self.traces_export_failed > 0
            || self.handshake.count > 0
            || !self.wire.is_empty()
    }

    /// Build OTLP/HTTP JSON (cumulative histograms + sums). Metrics-only — no prefixes.
    pub fn to_otlp_json(&self) -> String {
        let ts = now_unix_nano();
        let start = self.start_unix_nano;
        let mut metrics: Vec<String> = Vec::new();

        let mut hist_points = Vec::new();
        let mut req_points = Vec::new();
        let mut err_points = Vec::new();

        for (k, s) in &self.series {
            let attrs = otlp_attrs(
                &k.src,
                &k.dst,
                &k.method,
                &k.route,
                &k.status_class,
                &k.protocol,
            );
            // OTLP bucketCounts are per-bucket (not Prometheus cumulative).
            // Last slot is +Inf. Sum of bucketCounts must equal `count` or
            // the collector returns HTTP 400.
            hist_points.push(hist_data_point(start, ts, s, &attrs));
            req_points.push(format!(
                r#"{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","asInt":"{count}","attributes":{attrs}}}"#,
                count = s.count,
            ));
            if s.errors > 0 {
                err_points.push(format!(
                    r#"{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","asInt":"{err}","attributes":{attrs}}}"#,
                    err = s.errors,
                ));
            }
        }

        if !hist_points.is_empty() {
            metrics.push(format!(
                r#"{{"name":"http.client.duration","unit":"ms","description":"Outbound HTTP client latency","histogram":{{"aggregationTemporality":2,"dataPoints":[{}]}}}}"#,
                hist_points.join(",")
            ));
        }
        if !req_points.is_empty() {
            metrics.push(format!(
                r#"{{"name":"http.client.requests","unit":"1","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{}]}}}}"#,
                req_points.join(",")
            ));
        }
        if !err_points.is_empty() {
            metrics.push(format!(
                r#"{{"name":"http.client.errors","unit":"1","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{}]}}}}"#,
                err_points.join(",")
            ));
        }

        if !self.wire.is_empty() {
            let mut wire_points = Vec::new();
            for (k, s) in &self.wire {
                let attrs = otlp_attrs(
                    &k.src,
                    &k.dst,
                    &k.method,
                    &k.route,
                    &k.status_class,
                    &k.protocol,
                );
                wire_points.push(hist_data_point(start, ts, s, &attrs));
            }
            metrics.push(format!(
                r#"{{"name":"http.client.wire.duration","unit":"ms","description":"TLS HTTP/1.1 wire latency (syscall pair)","histogram":{{"aggregationTemporality":2,"dataPoints":[{}]}}}}"#,
                wire_points.join(",")
            ));
        }
        if self.handshake.count > 0 {
            metrics.push(unlabeled_hist(
                "tls.handshake.duration",
                "SSL_do_handshake duration",
                start,
                ts,
                &self.handshake,
            ));
        }

        metrics.push(format!(
            r#"{{"name":"obsagent.events_dropped","unit":"1","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","asInt":"{}"}}]}}}}"#,
            self.events_dropped
        ));
        metrics.push(format!(
            r#"{{"name":"obsagent.tls_unmapped","unit":"1","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","asInt":"{}"}}]}}}}"#,
            self.tls_unmapped
        ));
        metrics.push(format!(
            r#"{{"name":"obsagent.otlp_dropped","unit":"1","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","asInt":"{}"}}]}}}}"#,
            self.otlp_dropped
        ));
        metrics.push(format!(
            r#"{{"name":"obsagent.traces.sampled","unit":"1","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","asInt":"{}"}}]}}}}"#,
            self.traces_sampled
        ));
        metrics.push(format!(
            r#"{{"name":"obsagent.traces.not_sampled","unit":"1","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","asInt":"{}"}}]}}}}"#,
            self.traces_not_sampled
        ));
        metrics.push(format!(
            r#"{{"name":"obsagent.traces.dropped","unit":"1","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","asInt":"{}"}}]}}}}"#,
            self.traces_dropped
        ));
        metrics.push(format!(
            r#"{{"name":"obsagent.traces.export_failed","unit":"1","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","asInt":"{}"}}]}}}}"#,
            self.traces_export_failed
        ));
        metrics.push(format!(
            r#"{{"name":"obsagent.service_map.edges","unit":"1","gauge":{{"dataPoints":[{{"timeUnixNano":"{ts}","asInt":"{}"}}]}}}}"#,
            self.edge_count
        ));
        metrics.push(format!(
            r#"{{"name":"obsagent.sample_n","unit":"1","gauge":{{"dataPoints":[{{"timeUnixNano":"{ts}","asInt":"{}"}}]}}}}"#,
            self.sample_n
        ));

        format!(
            r#"{{"resourceMetrics":[{{"resource":{{"attributes":[{{"key":"service.name","value":{{"stringValue":"obsagent"}}}}]}},"scopeMetrics":[{{"scope":{{"name":"obsagent","version":"0.1.0"}},"metrics":[{}]}}]}}]}}"#,
            metrics.join(",")
        )
    }
}

pub fn status_class(status: u16) -> String {
    match status {
        200..=299 => "2xx".into(),
        300..=399 => "3xx".into(),
        400..=499 => "4xx".into(),
        500..=599 => "5xx".into(),
        _ => "other".into(),
    }
}

fn hist_data_point(start: u128, ts: u128, s: &SeriesStats, attrs: &str) -> String {
    let bounds: String = LATENCY_BOUNDS_MS
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let buckets: String = s
        .buckets
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","count":"{count}","sum":{sum},"bucketCounts":[{buckets}],"explicitBounds":[{bounds}],"attributes":{attrs}}}"#,
        count = s.count,
        sum = s.sum_ms,
    )
}

fn unlabeled_hist(name: &str, desc: &str, start: u128, ts: u128, s: &SeriesStats) -> String {
    let bounds: String = LATENCY_BOUNDS_MS
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let buckets: String = s
        .buckets
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"name":"{name}","unit":"ms","description":"{desc}","histogram":{{"aggregationTemporality":2,"dataPoints":[{{"startTimeUnixNano":"{start}","timeUnixNano":"{ts}","count":"{count}","sum":{sum},"bucketCounts":[{buckets}],"explicitBounds":[{bounds}]}}]}}}}"#,
        count = s.count,
        sum = s.sum_ms,
    )
}

fn otlp_attrs(
    src: &str,
    dst: &str,
    method: &str,
    route: &str,
    status_class: &str,
    protocol: &str,
) -> String {
    format!(
        "[{},{},{},{},{},{}]",
        kv("src", src),
        kv("dst", dst),
        kv("http.method", method),
        kv("http.route", route),
        kv("http.status_class", status_class),
        kv("http.protocol", protocol),
    )
}

fn kv(key: &str, value: &str) -> String {
    format!(
        "{{\"key\":{},\"value\":{{\"stringValue\":{}}}}}",
        json_str(key),
        json_str(value)
    )
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn now_unix_nano() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_into_correct_bucket() {
        let mut r = MetricsRegistry::new();
        r.record(&HttpObs {
            src: "a".into(),
            dst: "b".into(),
            method: "GET".into(),
            route: "/slow".into(),
            latency_ns: 50_000_000, // 50ms
            status: 200,
            protocol: "http".into(),
            wire_ns: None,
        });
        assert_eq!(r.total_requests(), 1);
        let body = r.to_otlp_json();
        assert!(body.contains("http.client.duration"));
        assert!(body.contains("aggregationTemporality\":2"));
        assert!(body.contains("histogram"));
        assert!(!body.contains("\"gauge\":{\"dataPoints\":[{\"asDouble\""));
        assert!(body.contains("/slow"));
        assert!(body.contains("http.protocol"));
        assert!(body.contains("obsagent.traces.sampled"));
        assert!(body.contains("obsagent.traces.not_sampled"));
        assert!(body.contains("obsagent.traces.dropped"));
        assert!(body.contains("obsagent.traces.export_failed"));
        assert!(body.contains("obsagent.sample_n"));
        assert!(!body.contains("Authorization"));
        let v: serde_json::Value = serde_json::from_str(&body).expect("otlp json");
        let hist = &v["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["histogram"]
            ["dataPoints"][0];
        let count: u64 = hist["count"].as_str().unwrap().parse().unwrap();
        let buckets: u64 = hist["bucketCounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_str().unwrap().parse::<u64>().unwrap())
            .sum();
        assert_eq!(count, 1);
        assert_eq!(buckets, count);
        assert_eq!(
            hist["bucketCounts"].as_array().unwrap().len(),
            LATENCY_BOUNDS_MS.len() + 1
        );
        assert_eq!(
            hist["explicitBounds"].as_array().unwrap().len(),
            LATENCY_BOUNDS_MS.len()
        );
        // 50ms lands in the 50 bound, not a running cumulative of earlier bounds.
        let idx = LATENCY_BOUNDS_MS.iter().position(|b| *b == 50.0).unwrap();
        assert_eq!(hist["bucketCounts"][idx], "1");
        assert_eq!(hist["bucketCounts"][0], "0");
    }

    #[test]
    fn overflow_uses_other_series() {
        let mut r = MetricsRegistry::new();
        for i in 0..(MAX_SERIES + 5) {
            r.record(&HttpObs {
                src: format!("s{i}"),
                dst: format!("d{i}"),
                method: "GET".into(),
                route: "/".into(),
                latency_ns: 1_000_000,
                status: 200,
                protocol: "http".into(),
                wire_ns: None,
            });
        }
        assert!(r.overflow_count() >= 5);
        assert!(r.series_len() <= MAX_SERIES + 1);
    }

    #[test]
    fn status_class_mapping() {
        assert_eq!(status_class(200), "2xx");
        assert_eq!(status_class(404), "4xx");
        assert_eq!(status_class(503), "5xx");
    }

    #[test]
    fn wire_histogram_carries_route_labels() {
        let mut r = MetricsRegistry::new();
        r.record(&HttpObs {
            src: "a".into(),
            dst: "b".into(),
            method: "GET".into(),
            route: "/slow".into(),
            latency_ns: 50_000_000,
            status: 200,
            protocol: "http".into(),
            wire_ns: Some(49_000_000),
        });
        r.record(&HttpObs {
            src: "a".into(),
            dst: "b".into(),
            method: "GET".into(),
            route: "/fast".into(),
            latency_ns: 1_000_000,
            status: 200,
            protocol: "http".into(),
            wire_ns: Some(900_000),
        });
        let body = r.to_otlp_json();
        assert!(body.contains("http.client.wire.duration"));
        assert!(!body.contains("Authorization"));
        let v: serde_json::Value = serde_json::from_str(&body).expect("otlp json");
        let metrics = v["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
            .as_array()
            .unwrap();
        let wire = metrics
            .iter()
            .find(|m| m["name"] == "http.client.wire.duration")
            .expect("wire metric");
        let points = wire["histogram"]["dataPoints"].as_array().unwrap();
        assert_eq!(points.len(), 2);
        let routes: Vec<String> = points
            .iter()
            .flat_map(|p| p["attributes"].as_array().unwrap())
            .filter(|a| a["key"] == "http.route")
            .map(|a| a["value"]["stringValue"].as_str().unwrap().to_string())
            .collect();
        assert!(routes.contains(&"/slow".into()));
        assert!(routes.contains(&"/fast".into()));
    }
}
