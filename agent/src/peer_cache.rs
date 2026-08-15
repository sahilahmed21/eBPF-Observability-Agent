//! Userspace peer cache (Phase 5 Q8).
//!
//! Mirrors successful `SOCK_META` lookups with a short TTL so a transient map
//! miss does not force `dst=unknown` on an in-flight exchange.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::correlate::PeerV4;

pub const TTL: Duration = Duration::from_secs(120);
const CAP: usize = 16384;

#[derive(Clone, Copy)]
struct Entry {
    peer: PeerV4,
    at: Instant,
}

#[derive(Default)]
pub struct PeerCache {
    map: HashMap<(u32, i32), Entry>,
}

impl PeerCache {
    pub fn insert(&mut self, tgid: u32, fd: i32, peer: PeerV4, now: Instant) {
        if self.map.len() >= CAP {
            self.map
                .retain(|_, e| now.duration_since(e.at) <= TTL);
        }
        if self.map.len() >= CAP {
            self.map.clear();
        }
        self.map.insert((tgid, fd), Entry { peer, at: now });
    }

    pub fn get(&mut self, tgid: u32, fd: i32, now: Instant) -> Option<PeerV4> {
        match self.map.get(&(tgid, fd)) {
            Some(e) if now.duration_since(e.at) <= TTL => Some(e.peer),
            Some(_) => {
                self.map.remove(&(tgid, fd));
                None
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_cached_peer_within_ttl() {
        let mut c = PeerCache::default();
        let now = Instant::now();
        let peer = PeerV4 {
            daddr_be: 1,
            dport_be: 2,
        };
        c.insert(1, 3, peer, now);
        assert_eq!(c.get(1, 3, now), Some(peer));
        assert_eq!(c.get(1, 3, now + TTL + Duration::from_secs(1)), None);
    }
}
