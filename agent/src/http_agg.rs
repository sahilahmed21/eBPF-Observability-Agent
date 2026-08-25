//! Rolling 60s HTTP endpoint aggregates (Phase 2).

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;

use crate::http::{HttpEndpoint, ParsedExchange};

pub const WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug)]
struct Sample {
    at: Instant,
    latency_ns: u64,
    wire_ns: Option<u64>,
    status: u16,
}

#[derive(Debug, Default)]
struct EndpointStats {
    samples: VecDeque<Sample>,
}

#[derive(Clone, Debug)]
pub struct HttpRow {
    pub count: u64,
    pub rate_per_s: f64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub wire_p50_ns: Option<u64>,
    pub pct_4xx: f64,
    pub pct_5xx: f64,
}

#[derive(Default)]
pub struct HttpAggregator {
    endpoints: HashMap<HttpEndpoint, EndpointStats>,
}

impl HttpAggregator {
    pub fn record(&mut self, parsed: &ParsedExchange, now: Instant) {
        let stats = self.endpoints.entry(parsed.endpoint.clone()).or_default();
        stats.samples.push_back(Sample {
            at: now,
            latency_ns: parsed.latency_ns,
            wire_ns: parsed.wire_ns,
            status: parsed.status,
        });
        stats.prune(now);
    }

    pub fn rows(&self, now: Instant) -> Vec<(HttpEndpoint, HttpRow)> {
        let mut out: Vec<_> = self
            .endpoints
            .iter()
            .filter_map(|(k, s)| {
                let row = s.snapshot(now);
                if row.count == 0 {
                    None
                } else {
                    Some((k.clone(), row))
                }
            })
            .collect();
        out.sort_by(|a, b| b.1.count.cmp(&a.1.count));
        out
    }
}

impl EndpointStats {
    fn prune(&mut self, now: Instant) {
        while self
            .samples
            .front()
            .is_some_and(|s| now.duration_since(s.at) > WINDOW)
        {
            self.samples.pop_front();
        }
    }

    fn snapshot(&self, now: Instant) -> HttpRow {
        let samples: Vec<_> = self
            .samples
            .iter()
            .copied()
            .filter(|s| now.duration_since(s.at) <= WINDOW)
            .collect();
        let count = samples.len() as u64;
        if count == 0 {
            return HttpRow {
                count: 0,
                rate_per_s: 0.0,
                p50_ns: 0,
                p95_ns: 0,
                p99_ns: 0,
                wire_p50_ns: None,
                pct_4xx: 0.0,
                pct_5xx: 0.0,
            };
        }
        // ponytail: rebuild hist from windowed samples (same as tcp Aggregator).
        // Incremental hist can't drop expired samples; upgrade if rows() shows up in profiles.
        let mut hist = Histogram::<u64>::new(3).expect("hist");
        let mut wire_hist = Histogram::<u64>::new(3).expect("hist");
        let mut n_wire = 0u64;
        let mut n4 = 0u64;
        let mut n5 = 0u64;
        for s in &samples {
            let _ = hist.record(s.latency_ns.max(1));
            if let Some(w) = s.wire_ns {
                let _ = wire_hist.record(w.max(1));
                n_wire += 1;
            }
            if (400..500).contains(&s.status) {
                n4 += 1;
            } else if (500..600).contains(&s.status) {
                n5 += 1;
            }
        }
        HttpRow {
            count,
            rate_per_s: count as f64 / WINDOW.as_secs_f64(),
            p50_ns: hist.value_at_quantile(0.50),
            p95_ns: hist.value_at_quantile(0.95),
            p99_ns: hist.value_at_quantile(0.99),
            wire_p50_ns: if n_wire == 0 {
                None
            } else {
                Some(wire_hist.value_at_quantile(0.50))
            },
            pct_4xx: (n4 as f64) * 100.0 / count as f64,
            pct_5xx: (n5 as f64) * 100.0 / count as f64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpEndpoint, ParsedExchange};

    #[test]
    fn aggregates_by_endpoint_and_status_classes() {
        let mut a = HttpAggregator::default();
        let now = Instant::now();
        let ep = HttpEndpoint::http("GET", "/users/:id");
        for status in [200u16, 200, 404, 500] {
            a.record(
                &ParsedExchange {
                    endpoint: ep.clone(),
                    status,
                    latency_ns: 1_000_000,
                    wire_ns: None,
                },
                now,
            );
        }
        let rows = a.rows(now);
        assert_eq!(rows.len(), 1);
        let (_, r) = &rows[0];
        assert_eq!(r.count, 4);
        assert!((r.pct_4xx - 25.0).abs() < 0.01);
        assert!((r.pct_5xx - 25.0).abs() < 0.01);
        assert!(r.wire_p50_ns.is_none());
    }

    #[test]
    fn wire_p50_is_per_endpoint() {
        let mut a = HttpAggregator::default();
        let now = Instant::now();
        a.record(
            &ParsedExchange {
                endpoint: HttpEndpoint::http("GET", "/slow"),
                status: 200,
                latency_ns: 50_000_000,
                wire_ns: Some(49_000_000),
            },
            now,
        );
        a.record(
            &ParsedExchange {
                endpoint: HttpEndpoint::http("GET", "/fast"),
                status: 200,
                latency_ns: 1_000_000,
                wire_ns: Some(900_000),
            },
            now,
        );
        let rows = a.rows(now);
        let slow = rows
            .iter()
            .find(|(k, _)| k.path == "/slow")
            .map(|(_, r)| r.wire_p50_ns)
            .expect("slow");
        let fast = rows
            .iter()
            .find(|(k, _)| k.path == "/fast")
            .map(|(_, r)| r.wire_p50_ns)
            .expect("fast");
        assert!(slow.unwrap() > 10_000_000);
        assert!(fast.unwrap() < 5_000_000);
    }
}
