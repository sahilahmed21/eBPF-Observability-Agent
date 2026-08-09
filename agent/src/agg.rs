//! Rolling 60s aggregates keyed by remote endpoint + direction (Q5).

use std::collections::{HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use obsagent_common::{EventKind, SockLatencyEvent};

pub const WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EndpointKey {
    pub daddr_be: u32,
    pub dport_be: u16,
    pub kind: EventKind,
}

impl EndpointKey {
    pub fn from_event(ev: &SockLatencyEvent) -> Option<Self> {
        let kind = EventKind::from_u8(ev.kind)?;
        Some(Self {
            daddr_be: ev.daddr_be,
            dport_be: ev.dport_be,
            kind,
        })
    }

    pub fn label(&self) -> String {
        // `daddr_be` holds raw `sin_addr.s_addr` bytes (network order in memory).
        let ip = Ipv4Addr::from(self.daddr_be.to_ne_bytes());
        let port = u16::from_be(self.dport_be);
        let dir = match self.kind {
            EventKind::Connect => "connect",
            EventKind::Accept => "accept",
            EventKind::SockIo | EventKind::TlsIo => "sockio", // not used in TCP agg keys
        };
        format!("{ip}:{port} {dir}")
    }
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    at: Instant,
    latency_ns: u64,
    is_error: bool,
}

#[derive(Debug, Default)]
pub struct EndpointStats {
    samples: VecDeque<Sample>,
}

impl EndpointStats {
    fn push(&mut self, now: Instant, latency_ns: u64, is_error: bool) {
        self.samples.push_back(Sample {
            at: now,
            latency_ns,
            is_error,
        });
        self.prune(now);
    }

    fn prune(&mut self, now: Instant) {
        while self
            .samples
            .front()
            .is_some_and(|s| now.duration_since(s.at) > WINDOW)
        {
            self.samples.pop_front();
        }
    }

    pub fn snapshot(&self, now: Instant) -> RowSnapshot {
        let mut samples = self.samples.clone();
        while samples
            .front()
            .is_some_and(|s| now.duration_since(s.at) > WINDOW)
        {
            samples.pop_front();
        }
        let count = samples.len() as u64;
        let errors = samples.iter().filter(|s| s.is_error).count() as u64;
        let (p50, p95, p99) = percentiles_ns(samples.iter().map(|s| s.latency_ns));
        RowSnapshot {
            count,
            errors,
            p50_ns: p50,
            p95_ns: p95,
            p99_ns: p99,
        }
    }
}

fn percentiles_ns(latencies: impl Iterator<Item = u64>) -> (u64, u64, u64) {
    let mut h = Histogram::<u64>::new(3).expect("hdrhistogram sigfig=3");
    let mut any = false;
    for v in latencies {
        any = true;
        let _ = h.record(v.max(1));
    }
    if !any {
        return (0, 0, 0);
    }
    (
        h.value_at_quantile(0.50),
        h.value_at_quantile(0.95),
        h.value_at_quantile(0.99),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowSnapshot {
    pub count: u64,
    pub errors: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
}

#[derive(Debug, Default)]
pub struct Aggregator {
    by_endpoint: HashMap<EndpointKey, EndpointStats>,
}

impl Aggregator {
    pub fn record(&mut self, ev: &SockLatencyEvent, now: Instant) {
        let Some(key) = EndpointKey::from_event(ev) else {
            return;
        };
        let is_error = is_syscall_error(ev.ret);
        self.by_endpoint
            .entry(key)
            .or_default()
            .push(now, ev.latency_ns, is_error);
    }

    pub fn rows(&self, now: Instant) -> Vec<(EndpointKey, RowSnapshot)> {
        let mut out: Vec<_> = self
            .by_endpoint
            .iter()
            .map(|(k, s)| (*k, s.snapshot(now)))
            .filter(|(_, r)| r.count > 0)
            .collect();
        out.sort_by(|a, b| b.1.count.cmp(&a.1.count));
        out
    }
}

/// Syscall failed if return is negative (includes `-EINPROGRESS` as non-success).
pub fn is_syscall_error(ret: i64) -> bool {
    ret < 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsagent_common::EventKind;

    fn ev(kind: EventKind, latency_ns: u64, ret: i64, addr: u32, port: u16) -> SockLatencyEvent {
        SockLatencyEvent {
            kind: kind as u8,
            _pad0: [0; 7],
            pid: 1,
            tgid: 1,
            ret,
            latency_ns,
            ts_ns: 0,
            daddr_be: addr,
            dport_be: port,
            _pad1: 0,
        }
    }

    #[test]
    fn aggregates_by_endpoint_and_direction() {
        let mut agg = Aggregator::default();
        let now = Instant::now();
        let addr = u32::from_ne_bytes([1, 2, 3, 4]);
        let port = 80u16.to_be();
        agg.record(&ev(EventKind::Connect, 1_000, 0, addr, port), now);
        agg.record(&ev(EventKind::Accept, 2_000, 0, addr, port), now);
        let rows = agg.rows(now);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.iter().map(|(_, r)| r.count).sum::<u64>(), 2);
    }

    #[test]
    fn counts_errors_on_negative_ret() {
        let mut agg = Aggregator::default();
        let now = Instant::now();
        let addr = u32::from_ne_bytes([10, 0, 0, 1]);
        let port = 443u16.to_be();
        agg.record(&ev(EventKind::Connect, 100, 0, addr, port), now);
        agg.record(&ev(EventKind::Connect, 100, -115, addr, port), now); // EINPROGRESS
        let (_, row) = agg.rows(now).into_iter().next().unwrap();
        assert_eq!(row.count, 2);
        assert_eq!(row.errors, 1);
        assert!(is_syscall_error(-115));
        assert!(!is_syscall_error(0));
    }

    #[test]
    fn rolling_window_drops_old_samples() {
        let mut agg = Aggregator::default();
        let start = Instant::now();
        let addr = u32::from_ne_bytes([8, 8, 8, 8]);
        let port = 53u16.to_be();
        agg.record(&ev(EventKind::Connect, 50, 0, addr, port), start);
        let later = start + WINDOW + Duration::from_secs(1);
        let rows = agg.rows(later);
        assert!(rows.is_empty());
    }

    #[test]
    fn percentiles_reflect_distribution() {
        let mut agg = Aggregator::default();
        let now = Instant::now();
        let addr = u32::from_ne_bytes([1, 1, 1, 1]);
        let port = 1u16.to_be();
        for i in 1..=100 {
            agg.record(
                &ev(EventKind::Connect, i * 1_000, 0, addr, port),
                now,
            );
        }
        let (_, row) = agg.rows(now).into_iter().next().unwrap();
        assert_eq!(row.count, 100);
        assert!(row.p50_ns > 0);
        assert!(row.p95_ns >= row.p50_ns);
        assert!(row.p99_ns >= row.p95_ns);
    }

    #[test]
    fn endpoint_label_includes_direction() {
        let key = EndpointKey {
            daddr_be: u32::from_ne_bytes([127, 0, 0, 1]),
            dport_be: 8080u16.to_be(),
            kind: EventKind::Accept,
        };
        assert_eq!(key.label(), "127.0.0.1:8080 accept");
    }
}
