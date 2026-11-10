//! Deterministic replay of an event log (`evlog.rs`) through the userspace pipeline.
//!
//! Covers decode → reassemble → h2/HTTP1 correlate → parse. It does NOT cover kernel-side
//! filtering/sampling/`INFLIGHT` (already applied when the log was recorded), comm/identity
//! lookups, or export. The clock is virtual: `now = base + (ts_ns - first_ts)`, so TIMEOUT
//! eviction depends only on the log.

use std::time::{Duration, Instant};

use crate::correlate::TIMEOUT;
use crate::decode::{decode_event, DecodedEvent};
use crate::http::{normalize_path, parse_exchange, HttpEndpoint, ParsedExchange};
use crate::ingest::IngestState;
use crate::llm::LlmCall;

pub fn replay(records: &[Vec<u8>]) -> Vec<ParsedExchange> {
    replay_full(records).0
}

/// Also returns streamed LLM calls; streams still open at the end are reported incomplete.
pub fn replay_full(records: &[Vec<u8>]) -> (Vec<ParsedExchange>, Vec<LlmCall>) {
    let mut st = IngestState::default();
    let base = Instant::now();
    let mut first_ts: Option<u64> = None;
    let mut out = Vec::new();
    let mut calls = Vec::new();
    let mut last_now = base;
    for rec in records {
        let (ev, client_only) = match decode_event(rec) {
            Some(DecodedEvent::Io(ev)) => (ev, false),
            // Same default as the live agent (OBSAGENT_TLS_SERVER unset): TLS is client-only.
            Some(DecodedEvent::TlsIo(ev)) => (ev, true),
            _ => continue,
        };
        let t0 = *first_ts.get_or_insert(ev.ts_ns);
        let now = base + Duration::from_nanos(ev.ts_ns.saturating_sub(t0));
        last_now = now;
        let ing = st.ingest(ev, now, client_only);
        calls.extend(ing.llm);
        if let Some(ex) = ing.exchange.as_ref().and_then(parse_exchange) {
            out.push(ex);
        }
        for x in ing.h2 {
            let latency_ns = x.latency_ns();
            out.push(ParsedExchange {
                endpoint: HttpEndpoint::labeled(x.protocol, x.method, normalize_path(&x.path)),
                status: x.status,
                latency_ns,
                wire_ns: None,
            });
        }
    }
    calls.extend(st.llm.evict_stale(last_now + TIMEOUT + Duration::from_secs(1)));
    (out, calls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evlog::{read_all, Writer};
    use obsagent_common::{EventKind, IoDir, SockIoEvent, SOCK_IO_EVENT_SIZE, SOCK_IO_PREFIX_LEN};

    fn rec(kind: EventKind, dir: IoDir, fd: i32, ts_ns: u64, payload: &[u8]) -> Vec<u8> {
        let mut prefix = [0u8; SOCK_IO_PREFIX_LEN];
        prefix[..payload.len()].copy_from_slice(payload);
        let ev = SockIoEvent {
            kind: kind as u8,
            dir: dir as u8,
            prefix_len: payload.len() as u16,
            fd,
            pid: 1,
            tgid: 42,
            ret: payload.len() as i64,
            ts_ns,
            cgroup_id: 0,
            prefix,
        };
        // SAFETY: repr(C) plain data; test-only byte view.
        unsafe { core::slice::from_raw_parts((&ev as *const SockIoEvent).cast::<u8>(), SOCK_IO_EVENT_SIZE) }
            .to_vec()
    }

    fn log(records: &[Vec<u8>]) -> Vec<Vec<u8>> {
        let mut buf = Vec::new();
        let mut w = Writer::new(&mut buf).unwrap();
        for r in records {
            w.write(r).unwrap();
        }
        read_all(&buf[..]).unwrap()
    }

    const REQ: &[u8] = b"GET /slow HTTP/1.1\r\nHost: x\r\n\r\n";
    const RESP: &[u8] = b"HTTP/1.1 200 OK\r\n\r\n";

    #[test]
    fn replays_recorded_exchange_with_exact_latency() {
        let l = log(&[
            rec(EventKind::SockIo, IoDir::Write, 7, 1_000, REQ),
            rec(EventKind::SockIo, IoDir::Read, 7, 50_001_000, RESP),
        ]);
        let rows = replay(&l);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].endpoint, HttpEndpoint::http("GET", "/slow"));
        assert_eq!(rows[0].status, 200);
        assert_eq!(rows[0].latency_ns, 50_000_000);
    }

    #[test]
    fn twenty_replays_are_identical() {
        let l = log(&[
            rec(EventKind::SockIo, IoDir::Write, 7, 1_000, REQ),
            rec(EventKind::SockIo, IoDir::Read, 7, 2_000, RESP),
            rec(EventKind::SockIo, IoDir::Write, 8, 3_000, REQ),
            rec(EventKind::SockIo, IoDir::Read, 8, 9_000, RESP),
        ]);
        let first = replay(&l);
        assert_eq!(first.len(), 2);
        for _ in 0..20 {
            assert_eq!(replay(&l), first);
        }
    }

    #[test]
    fn virtual_clock_applies_the_60s_timeout() {
        let l = log(&[
            rec(EventKind::SockIo, IoDir::Write, 7, 1_000, REQ),
            rec(EventKind::SockIo, IoDir::Read, 7, 61_000_000_000, RESP),
        ]);
        assert!(replay(&l).is_empty());
    }

    #[test]
    fn tls_events_pair_client_write_then_read() {
        let l = log(&[
            rec(EventKind::TlsIo, IoDir::Write, 9, 1_000, REQ),
            rec(EventKind::TlsIo, IoDir::Read, 9, 4_000, RESP),
        ]);
        assert_eq!(replay(&l).len(), 1);
    }

    /// Real capture of 8 probe requests to a 50 ms `/slow` server (agent live output for the
    /// same run: `GET /slow count=15 p50=50.53ms`). Needs no root or BPF.
    #[test]
    fn golden_real_slow_50ms_capture_replays_to_live_result() {
        let bytes = include_bytes!("../../tests/fixtures/slow-50ms.evlog");
        let mut rows = replay(&read_all(&bytes[..]).unwrap());
        assert_eq!(rows.len(), 15);
        assert!(rows.iter().all(|r| r.endpoint == HttpEndpoint::http("GET", "/slow") && r.status == 200));
        rows.sort_by_key(|r| r.latency_ns);
        let p50_ms = rows[rows.len() / 2].latency_ns as f64 / 1e6;
        assert!((45.0..=56.0).contains(&p50_ms), "p50={p50_ms}ms");
    }

    /// xorshift64: deterministic pseudo-random source for fault/stress tests.
    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    #[test]
    fn recovery_equals_exchanges_whose_both_halves_survive_record_loss() {
        for drop_pct in [0u64, 10, 50] {
            let mut seed = 0x9E37_79B9_7F4A_7C15 ^ drop_pct;
            let (mut records, mut expected) = (Vec::new(), 0usize);
            for i in 0..200i32 {
                let t = i as u64 * 1_000_000;
                let keep_req = xs(&mut seed) % 100 >= drop_pct;
                let keep_resp = xs(&mut seed) % 100 >= drop_pct;
                if keep_req {
                    records.push(rec(EventKind::SockIo, IoDir::Write, i, t, REQ));
                }
                if keep_resp {
                    records.push(rec(EventKind::SockIo, IoDir::Read, i, t + 500, RESP));
                }
                expected += (keep_req && keep_resp) as usize;
            }
            assert_eq!(replay(&log(&records)).len(), expected, "drop {drop_pct}%");
        }
    }

    #[test]
    fn random_bytes_never_panic_the_pipeline() {
        let seeds: [&[u8]; 5] = [b"GET / HTTP/1.1\r\n", b"HTTP/1.1 200", b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n", b"\x00\x00\x05\x01\x04\x00\x00\x00\x01", b""];
        let mut s = 0xDEAD_BEEF_CAFE_F00D_u64;
        let mut records = Vec::new();
        for i in 0..20_000u64 {
            let mut p = seeds[(xs(&mut s) % 5) as usize].to_vec();
            for _ in 0..(xs(&mut s) % 200) {
                p.push(xs(&mut s) as u8);
            }
            p.truncate(SOCK_IO_PREFIX_LEN);
            let kind = if xs(&mut s) % 2 == 0 { EventKind::SockIo } else { EventKind::TlsIo };
            let dir = if xs(&mut s) % 2 == 0 { IoDir::Write } else { IoDir::Read };
            records.push(rec(kind, dir, (xs(&mut s) % 8) as i32, i * 1_000, &p));
        }
        let _ = replay(&records);
    }

    #[test]
    fn garbage_and_non_io_records_are_skipped() {
        let l = vec![vec![0xFFu8; 5], vec![]];
        assert!(replay(&l).is_empty());
    }
}
