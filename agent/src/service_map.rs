//! In-memory service map: nodes + directed edges (Phase 4).
//!
//! No graph DB. Cardinality capped (Q8). Stats are a rolling 60s window for CLI;
//! OTLP export reads the same snapshot and also accumulates process counters.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::http::HttpEndpoint;
use crate::identity::NodeId;
use hdrhistogram::Histogram;

pub const WINDOW: Duration = Duration::from_secs(60);
pub const MAX_EDGES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DstId {
    Pod { namespace: String, name: String },
    IpPort { addr: String },
    Unknown,
}

impl DstId {
    pub fn label(&self) -> String {
        match self {
            Self::Pod { namespace, name } => format!("{namespace}/{name}"),
            Self::IpPort { addr } => addr.clone(),
            Self::Unknown => "unknown".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EdgeKey {
    pub src: String,
    pub dst: DstId,
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    at: Instant,
    latency_ns: u64,
    status: u16,
}

#[derive(Default)]
struct EdgeStats {
    samples: VecDeque<Sample>,
    /// Cumulative counters for OTLP (not window-pruned).
    cum_count: u64,
    cum_errors: u64,
    cum_latency_ns_sum: u128,
}

#[derive(Clone, Debug)]
pub struct EdgeRow {
    pub count: u64,
    pub rate_per_s: f64,
    pub p50_ns: u64,
    pub p99_ns: u64,
    pub pct_5xx: f64,
    pub cum_count: u64,
    pub cum_errors: u64,
}

#[derive(Default)]
pub struct ServiceMap {
    edges: HashMap<EdgeKey, EdgeStats>,
    overflow_count: u64,
}

#[derive(Clone, Debug)]
pub struct MapRecord {
    pub src: NodeId,
    pub dst: DstId,
    pub endpoint: HttpEndpoint,
    pub latency_ns: u64,
    pub status: u16,
}

impl ServiceMap {
    pub fn record(&mut self, rec: &MapRecord, now: Instant) {
        let key = EdgeKey {
            src: rec.src.label.clone(),
            dst: rec.dst.clone(),
        };
        if !self.edges.contains_key(&key) && self.edges.len() >= MAX_EDGES {
            self.overflow_count += 1;
            let key = EdgeKey {
                src: "_other".into(),
                dst: DstId::Unknown,
            };
            self.record_key(key, rec, now);
            return;
        }
        self.record_key(key, rec, now);
    }

    fn record_key(&mut self, key: EdgeKey, rec: &MapRecord, now: Instant) {
        let stats = self.edges.entry(key).or_default();
        stats.samples.push_back(Sample {
            at: now,
            latency_ns: rec.latency_ns,
            status: rec.status,
        });
        stats.cum_count += 1;
        stats.cum_latency_ns_sum += rec.latency_ns as u128;
        if rec.status >= 500 {
            stats.cum_errors += 1;
        }
        stats.prune(now);
        let _ = &rec.endpoint; // reserved for per-route extension
    }

    pub fn rows(&self, now: Instant) -> Vec<(EdgeKey, EdgeRow)> {
        let mut out: Vec<_> = self
            .edges
            .iter()
            .filter_map(|(k, s)| {
                let row = s.snapshot(now);
                if row.count == 0 && row.cum_count == 0 {
                    None
                } else {
                    Some((k.clone(), row))
                }
            })
            .collect();
        out.sort_by(|a, b| b.1.count.cmp(&a.1.count));
        out
    }

    pub fn overflow_count(&self) -> u64 {
        self.overflow_count
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

impl EdgeStats {
    fn prune(&mut self, now: Instant) {
        while self
            .samples
            .front()
            .is_some_and(|s| now.duration_since(s.at) > WINDOW)
        {
            self.samples.pop_front();
        }
    }

    fn snapshot(&self, now: Instant) -> EdgeRow {
        let samples: Vec<_> = self
            .samples
            .iter()
            .filter(|s| now.duration_since(s.at) <= WINDOW)
            .copied()
            .collect();
        let count = samples.len() as u64;
        let mut pct_5xx = 0.0;
        let (p50_ns, p99_ns) = if count == 0 {
            (0, 0)
        } else {
            let err = samples.iter().filter(|s| s.status >= 500).count() as f64;
            pct_5xx = 100.0 * err / count as f64;
            let mut h = Histogram::<u64>::new(3).expect("hist");
            for s in &samples {
                let _ = h.record(s.latency_ns.max(1));
            }
            (h.value_at_quantile(0.50), h.value_at_quantile(0.99))
        };
        EdgeRow {
            count,
            rate_per_s: count as f64 / WINDOW.as_secs_f64(),
            p50_ns,
            p99_ns,
            pct_5xx,
            cum_count: self.cum_count,
            cum_errors: self.cum_errors,
        }
    }
}

/// Format IPv4 `daddr_be` + `dport_be` as `a.b.c.d:port`.
///
/// `daddr_be` holds raw `sin_addr.s_addr` bytes (same convention as `agg.rs`).
pub fn format_ip_port(daddr_be: u32, dport_be: u16) -> String {
    let ip = std::net::Ipv4Addr::from(daddr_be.to_ne_bytes());
    let port = u16::from_be(dport_be);
    format!("{ip}:{port}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpEndpoint;

    #[test]
    fn records_edge_and_p50() {
        let mut m = ServiceMap::default();
        let now = Instant::now();
        let rec = MapRecord {
            src: NodeId::proc_fallback(1, "curl"),
            dst: DstId::IpPort {
                addr: "10.0.0.1:80".into(),
            },
            endpoint: HttpEndpoint {
                method: "GET".into(),
                path: "/".into(),
            },
            latency_ns: 50_000_000,
            status: 200,
        };
        m.record(&rec, now);
        let rows = m.rows(now);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.count, 1);
        assert_eq!(rows[0].1.cum_count, 1);
        assert!(rows[0].1.p50_ns > 0);
    }

    #[test]
    fn format_loopback_http() {
        // 127.0.0.1 = 0x7f000001 BE
        let s = format_ip_port(u32::from_ne_bytes([127, 0, 0, 1]), u16::to_be(8080));
        assert_eq!(s, "127.0.0.1:8080");
    }

    #[test]
    fn overflow_bucket() {
        let mut m = ServiceMap::default();
        let now = Instant::now();
        for i in 0..(MAX_EDGES + 3) {
            let rec = MapRecord {
                src: NodeId {
                    label: format!("src{i}"),
                    tgid: i as u32,
                    comm: "x".into(),
                    container_id: None,
                    pod_uid: None,
                },
                dst: DstId::IpPort {
                    addr: format!("10.0.0.1:{i}"),
                },
                endpoint: HttpEndpoint {
                    method: "GET".into(),
                    path: "/".into(),
                },
                latency_ns: 1,
                status: 200,
            };
            m.record(&rec, now);
        }
        assert!(m.overflow_count() >= 3);
        assert!(m.edge_count() <= MAX_EDGES + 1);
    }
}
