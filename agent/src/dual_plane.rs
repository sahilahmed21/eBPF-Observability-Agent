//! Content-primary TLS dual-plane join (Phase 8).
//!
//! Sock I/O on a TLS fd is timestamps only. Join decorates a completed TLS
//! HTTP/1.1 exchange with the latest preceding sock Write/Read in a window.
//! Nested OpenSSL: `write()`/`read()` exit before `SSL_write`/`SSL_read`.
//!
//! Buffer contract is **time** (correlator [`TIMEOUT`]), not a count cap.
//! HTTP/2 is not joined: sock times are per-fd, h2 latency is per-stream.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use obsagent_common::IoDir;

use crate::correlate::TIMEOUT;

const DEFAULT_JOIN_NS: u64 = 5_000_000;
const WIRE_SLACK_NS: u64 = 20_000_000;
/// OOM guard per direction per fd. Join still searches by time; this is not the window.
const MAX_TS_PER_DIR: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SockKey {
    tgid: u32,
    fd: i32,
}

struct FdTimes {
    writes: VecDeque<u64>,
    reads: VecDeque<u64>,
    last: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Hit,
    Fallback,
    Miss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinKind {
    Hit,
    Fallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Join {
    pub wire_ns: u64,
    pub kind: JoinKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JoinWindow {
    pub hit: u64,
    pub fallback: u64,
    pub miss: u64,
}

impl JoinWindow {
    pub fn total(self) -> u64 {
        self.hit + self.fallback + self.miss
    }
}

pub struct DualPlane {
    fds: HashMap<SockKey, FdTimes>,
    join_window_ns: u64,
    outcomes: VecDeque<(Instant, Outcome)>,
    hs_samples: VecDeque<(Instant, u64)>,
}

impl DualPlane {
    pub fn new(join_window_ns: u64) -> Self {
        Self {
            fds: HashMap::new(),
            join_window_ns,
            outcomes: VecDeque::new(),
            hs_samples: VecDeque::new(),
        }
    }

    pub fn from_env() -> Self {
        let ms = std::env::var("OBSAGENT_JOIN_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(5);
        Self::new(ms.saturating_mul(1_000_000))
    }
}

impl Default for DualPlane {
    fn default() -> Self {
        Self::new(DEFAULT_JOIN_NS)
    }
}

impl DualPlane {
    pub fn note(&mut self, tgid: u32, fd: i32, dir: IoDir, ts_ns: u64, now: Instant) {
        let key = SockKey { tgid, fd };
        let slot = self.fds.entry(key).or_insert_with(|| FdTimes {
            writes: VecDeque::new(),
            reads: VecDeque::new(),
            last: now,
        });
        slot.last = now;
        match dir {
            IoDir::Write => push_ts(&mut slot.writes, ts_ns),
            IoDir::Read => push_ts(&mut slot.reads, ts_ns),
        }
    }

    /// `client`: request half is Write (typical TLS client). Server is Read then Write.
    pub fn join(
        &mut self,
        tgid: u32,
        fd: i32,
        t_start_ns: u64,
        t_end_ns: u64,
        client: bool,
        now: Instant,
    ) -> Option<Join> {
        let (start_dir, end_dir) = if client {
            (IoDir::Write, IoDir::Read)
        } else {
            (IoDir::Read, IoDir::Write)
        };
        let w = self.join_window_ns;
        let start_lo = t_start_ns.saturating_sub(w);
        let end_lo = t_end_ns.saturating_sub(w);

        let start_ts = self.latest(tgid, fd, start_dir, start_lo, t_start_ns);
        let end_ts = self.latest(tgid, fd, end_dir, end_lo, t_end_ns);

        if let (Some(s), Some(e)) = (start_ts, end_ts) {
            if let Some(j) = self.accept_wire(s, e, t_start_ns, t_end_ns, JoinKind::Hit, now) {
                return Some(j);
            }
        }

        let end_fb = self.latest(
            tgid,
            fd,
            end_dir,
            t_end_ns.saturating_add(1),
            t_end_ns.saturating_add(w),
        );
        if let (Some(s), Some(e)) = (start_ts, end_fb) {
            if let Some(j) = self.accept_wire(s, e, t_start_ns, t_end_ns, JoinKind::Fallback, now)
            {
                return Some(j);
            }
        }

        self.outcomes.push_back((now, Outcome::Miss));
        None
    }

    fn accept_wire(
        &mut self,
        start_ts: u64,
        end_ts: u64,
        t_start_ns: u64,
        t_end_ns: u64,
        kind: JoinKind,
        now: Instant,
    ) -> Option<Join> {
        let wire_ns = end_ts.saturating_sub(start_ts);
        let content_ns = t_end_ns.saturating_sub(t_start_ns);
        if wire_ns == 0 || wire_ns > content_ns.saturating_add(WIRE_SLACK_NS) {
            return None;
        }
        let outcome = match kind {
            JoinKind::Hit => Outcome::Hit,
            JoinKind::Fallback => Outcome::Fallback,
        };
        self.outcomes.push_back((now, outcome));
        Some(Join { wire_ns, kind })
    }

    fn latest(&self, tgid: u32, fd: i32, dir: IoDir, lo: u64, hi: u64) -> Option<u64> {
        let slot = self.fds.get(&SockKey { tgid, fd })?;
        let q = match dir {
            IoDir::Write => &slot.writes,
            IoDir::Read => &slot.reads,
        };
        q.iter().rev().copied().find(|ts| *ts >= lo && *ts <= hi)
    }

    pub fn record_handshake(&mut self, latency_ns: u64, now: Instant) {
        if latency_ns > 0 {
            self.hs_samples.push_back((now, latency_ns));
        }
    }

    #[cfg(test)]
    pub fn evict(&mut self, tgid: u32, fd: i32) {
        self.fds.remove(&SockKey { tgid, fd });
    }

    pub fn evict_stale(&mut self, now: Instant) {
        self.fds
            .retain(|_, r| now.duration_since(r.last) <= TIMEOUT);
        prune_at(&mut self.outcomes, now);
        prune_at(&mut self.hs_samples, now);
    }

    pub fn evict_closed(&mut self, still_open: impl Fn(u32, i32) -> bool) {
        self.fds.retain(|k, _| still_open(k.tgid, k.fd));
    }

    pub fn window(&self, now: Instant) -> JoinWindow {
        let mut w = JoinWindow::default();
        for (at, o) in &self.outcomes {
            if now.duration_since(*at) > TIMEOUT {
                continue;
            }
            match o {
                Outcome::Hit => w.hit += 1,
                Outcome::Fallback => w.fallback += 1,
                Outcome::Miss => w.miss += 1,
            }
        }
        w
    }

    /// Hit / attempts over the 60s window. Fallback is not a hit.
    pub fn join_hit_rate(&self, now: Instant) -> f64 {
        let w = self.window(now);
        let n = w.total();
        if n == 0 {
            0.0
        } else {
            w.hit as f64 / n as f64
        }
    }

    pub fn handshake_p50_ns(&self) -> Option<u64> {
        percentile_ns(&self.hs_samples, 0.50)
    }

    pub fn handshake_count(&self) -> usize {
        self.hs_samples.len()
    }
}

fn push_ts(q: &mut VecDeque<u64>, ts_ns: u64) {
    q.push_back(ts_ns);
    let floor = ts_ns.saturating_sub(TIMEOUT.as_nanos() as u64);
    while q.front().is_some_and(|t| *t < floor) {
        q.pop_front();
    }
    while q.len() > MAX_TS_PER_DIR {
        q.pop_front();
    }
}

fn prune_at<T>(samples: &mut VecDeque<(Instant, T)>, now: Instant) {
    while samples
        .front()
        .is_some_and(|(at, _)| now.duration_since(*at) > TIMEOUT)
    {
        samples.pop_front();
    }
}

fn percentile_ns(samples: &VecDeque<(Instant, u64)>, q: f64) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let mut v: Vec<u64> = samples.iter().map(|(_, n)| *n).collect();
    v.sort_unstable();
    let idx = ((v.len() as f64 - 1.0) * q).round() as usize;
    v.get(idx.clamp(0, v.len() - 1)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const W: u64 = 5_000_000;
    const T0: u64 = 100_000_000;
    const T1: u64 = 150_000_000;

    fn plane() -> DualPlane {
        DualPlane::new(W)
    }

    #[test]
    fn preceding_write_and_read_hits() {
        let mut p = plane();
        let now = Instant::now();
        p.note(1, 3, IoDir::Write, T0 - 1_000_000, now);
        p.note(1, 3, IoDir::Read, T1 - 500_000, now);
        let j = p.join(1, 3, T0, T1, true, now).expect("hit");
        assert_eq!(j.kind, JoinKind::Hit);
        assert_eq!(j.wire_ns, (T1 - 500_000) - (T0 - 1_000_000));
        let w = p.window(now);
        assert_eq!(w.hit, 1);
        assert_eq!(w.miss, 0);
    }

    #[test]
    fn ticket_after_ssl_read_does_not_steal() {
        let mut p = plane();
        let now = Instant::now();
        p.note(1, 3, IoDir::Write, T0 - 1_000_000, now);
        p.note(1, 3, IoDir::Read, T1 - 100_000, now);
        p.note(1, 3, IoDir::Read, T1 + 1_000_000, now);
        let j = p.join(1, 3, T0, T1, true, now).expect("hit");
        assert_eq!(j.kind, JoinKind::Hit);
        assert_eq!(j.wire_ns, (T1 - 100_000) - (T0 - 1_000_000));
    }

    #[test]
    fn ticket_read_inside_end_window_is_selected() {
        // Residual: latest Read in [t_end−W, t_end] wins, including a ticket.
        let mut p = plane();
        let now = Instant::now();
        p.note(1, 3, IoDir::Write, T0 - 1_000_000, now);
        p.note(1, 3, IoDir::Read, T1 - 2_000_000, now);
        p.note(1, 3, IoDir::Read, T1 - 100_000, now);
        let j = p.join(1, 3, T0, T1, true, now).expect("hit");
        assert_eq!(j.wire_ns, (T1 - 100_000) - (T0 - 1_000_000));
    }

    #[test]
    fn miss_when_no_sock_times() {
        let mut p = plane();
        let now = Instant::now();
        assert!(p.join(1, 3, T0, T1, true, now).is_none());
        assert_eq!(p.window(now).miss, 1);
    }

    #[test]
    fn fallback_when_only_read_after_content() {
        let mut p = plane();
        let now = Instant::now();
        p.note(1, 3, IoDir::Write, T0 - 1_000_000, now);
        p.note(1, 3, IoDir::Read, T1 + 2_000_000, now);
        let j = p.join(1, 3, T0, T1, true, now).expect("fallback");
        assert_eq!(j.kind, JoinKind::Fallback);
        let w = p.window(now);
        assert_eq!(w.fallback, 1);
        assert_eq!(w.hit, 0);
    }

    #[test]
    fn outside_window_is_miss() {
        let mut p = plane();
        let now = Instant::now();
        p.note(1, 3, IoDir::Write, T0 - 1_000_000, now);
        p.note(1, 3, IoDir::Read, T1 + 10_000_000, now);
        assert!(p.join(1, 3, T0, T1, true, now).is_none());
        assert_eq!(p.window(now).miss, 1);
    }

    #[test]
    fn server_role_uses_read_then_write() {
        let mut p = plane();
        let now = Instant::now();
        p.note(9, 4, IoDir::Read, T0 - 200_000, now);
        p.note(9, 4, IoDir::Write, T1 - 200_000, now);
        let j = p.join(9, 4, T0, T1, false, now).expect("server hit");
        assert_eq!(j.kind, JoinKind::Hit);
        assert_eq!(j.wire_ns, (T1 - 200_000) - (T0 - 200_000));
    }

    #[test]
    fn wire_above_content_plus_slack_is_miss() {
        let mut p = DualPlane::new(50_000_000);
        let now = Instant::now();
        p.note(1, 3, IoDir::Write, T0 - 50_000_000, now);
        p.note(1, 3, IoDir::Read, T0 + 10_000_000, now);
        assert!(p.join(1, 3, T0, T0 + 10_000_000, true, now).is_none());
        assert_eq!(p.window(now).miss, 1);
    }

    #[test]
    fn join_hit_rate_excludes_fallback() {
        let mut p = plane();
        let now = Instant::now();
        p.note(1, 3, IoDir::Write, T0 - 1_000_000, now);
        p.note(1, 3, IoDir::Read, T1 + 2_000_000, now);
        let _ = p.join(1, 3, T0, T1, true, now);
        let _ = p.join(2, 3, T0, T1, true, now);
        let w = p.window(now);
        assert_eq!(w.hit, 0);
        assert_eq!(w.fallback, 1);
        assert_eq!(w.miss, 1);
        assert!((p.join_hit_rate(now) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn evict_drops_fd_ring() {
        let mut p = plane();
        let now = Instant::now();
        p.note(1, 3, IoDir::Write, T0 - 1_000_000, now);
        p.evict(1, 3);
        assert!(p.join(1, 3, T0, T1, true, now).is_none());
    }

    #[test]
    fn handshake_p50_nonzero() {
        let mut p = plane();
        let now = Instant::now();
        p.record_handshake(3_000_000, now);
        p.record_handshake(5_000_000, now);
        let p50 = p.handshake_p50_ns().expect("hs");
        assert!(p50 > 0);
        assert!(p50 < 50_000_000);
    }

    #[test]
    fn later_writes_do_not_evict_request_half() {
        let mut p = plane();
        let now = Instant::now();
        p.note(1, 3, IoDir::Write, T0 - 1_000_000, now);
        for i in 0..40u64 {
            p.note(1, 3, IoDir::Write, T0 + 1_000 + i, now);
        }
        p.note(1, 3, IoDir::Read, T1 - 500_000, now);
        let j = p.join(1, 3, T0, T1, true, now).expect("hit after flood");
        assert_eq!(j.kind, JoinKind::Hit);
        assert_eq!(j.wire_ns, (T1 - 500_000) - (T0 - 1_000_000));
    }

    #[test]
    fn sequential_exchanges_keep_distinct_halves() {
        let mut p = plane();
        let now = Instant::now();
        p.note(1, 3, IoDir::Write, T0 - 1_000_000, now);
        p.note(1, 3, IoDir::Read, T1 - 500_000, now);
        let a = p.join(1, 3, T0, T1, true, now).expect("a");
        let t2 = T0 + 80_000_000;
        let t3 = T1 + 80_000_000;
        p.note(1, 3, IoDir::Write, t2 - 1_000_000, now);
        p.note(1, 3, IoDir::Read, t3 - 400_000, now);
        let b = p.join(1, 3, t2, t3, true, now).expect("b");
        assert_eq!(a.wire_ns, (T1 - 500_000) - (T0 - 1_000_000));
        assert_eq!(b.wire_ns, (t3 - 400_000) - (t2 - 1_000_000));
        assert_eq!(p.window(now).hit, 2);
    }

    #[test]
    fn join_counts_follow_timeout_window() {
        let mut p = plane();
        let now = Instant::now();
        assert!(p.join(1, 3, T0, T1, true, now).is_none());
        assert_eq!(p.window(now).miss, 1);
        let later = now + TIMEOUT + Duration::from_secs(1);
        p.evict_stale(later);
        assert_eq!(p.window(later).miss, 0);
    }
}
