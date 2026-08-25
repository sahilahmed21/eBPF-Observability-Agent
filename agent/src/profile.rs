//! CPU stack sample store + span join (Phase 12).

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use obsagent_common::{
    PROFILE_MIN_LATENCY_NS_DEFAULT, PROFILE_STORE_CAP, PROFILE_STORE_TTL_SECS, PROFILE_TOP_FRAMES,
};

pub const DEFAULT_PROFILE_FREQ: u64 = 99;

/// Phase 12 env gate and tuning.
#[derive(Clone, Debug)]
pub struct ProfileConfig {
    pub enabled: bool,
    pub min_latency_ns: u64,
    pub store_cap: usize,
    pub store_ttl: Duration,
    pub freq_hz: u64,
}

impl ProfileConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var_os("OBSAGENT_PROFILE").is_some();
        let min_latency_ns = std::env::var("OBSAGENT_PROFILE_MIN_NS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(PROFILE_MIN_LATENCY_NS_DEFAULT);
        let freq_hz = std::env::var("OBSAGENT_PROFILE_FREQ")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_PROFILE_FREQ);
        Self {
            enabled,
            min_latency_ns,
            store_cap: PROFILE_STORE_CAP,
            store_ttl: Duration::from_secs(PROFILE_STORE_TTL_SECS),
            freq_hz,
        }
    }
}

#[derive(Clone, Debug)]
struct StoredSample {
    tgid: u32,
    ts_ns: u64,
    frames: Vec<String>,
}

pub struct ProfileStore {
    config: ProfileConfig,
    samples: VecDeque<StoredSample>,
    pub samples_ingested: u64,
    pub samples_dropped: u64,
    pub join_attempts: u64,
    pub join_hits: u64,
}

impl ProfileStore {
    pub fn new(config: ProfileConfig) -> Self {
        Self {
            config,
            samples: VecDeque::new(),
            samples_ingested: 0,
            samples_dropped: 0,
            join_attempts: 0,
            join_hits: 0,
        }
    }

    pub fn config(&self) -> &ProfileConfig {
        &self.config
    }

    /// Store a symbolized stack sample (symbolization happens on the STACKS drain path).
    pub fn push_symbolized(&mut self, tgid: u32, ts_ns: u64, frames: Vec<String>) {
        if frames.is_empty() {
            return;
        }
        self.evict_before(ts_ns);
        if self.samples.len() >= self.config.store_cap {
            self.samples.pop_front();
            self.samples_dropped += 1;
        }
        self.samples.push_back(StoredSample {
            tgid,
            ts_ns,
            frames,
        });
        self.samples_ingested += 1;
    }

    /// Join top [`PROFILE_TOP_FRAMES`] symbols for a completed span (BPF monotonic window).
    pub fn join(&mut self, tgid: u32, t_start_ns: u64, t_end_ns: u64) -> Option<[String; PROFILE_TOP_FRAMES]> {
        if t_end_ns <= t_start_ns {
            return None;
        }
        if t_end_ns.saturating_sub(t_start_ns) < self.config.min_latency_ns {
            return None;
        }
        self.join_attempts += 1;
        self.evict_before(t_end_ns);
        let mut counts: HashMap<String, u64> = HashMap::new();
        for s in self.samples.iter() {
            if s.tgid != tgid {
                continue;
            }
            if s.ts_ns < t_start_ns || s.ts_ns > t_end_ns {
                continue;
            }
            for frame in &s.frames {
                *counts.entry(frame.clone()).or_insert(0) += 1;
            }
        }
        if counts.is_empty() {
            return None;
        }
        let mut ranked: Vec<(String, u64)> = counts.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut out: [String; PROFILE_TOP_FRAMES] = std::array::from_fn(|_| String::new());
        for (i, (name, _)) in ranked.into_iter().take(PROFILE_TOP_FRAMES).enumerate() {
            out[i] = name;
        }
        if out[0].is_empty() {
            return None;
        }
        self.join_hits += 1;
        Some(out)
    }

    pub fn frames_contain<'a>(frames: &'a [String; PROFILE_TOP_FRAMES], needle: &str) -> bool {
        frames.iter().any(|f| f.contains(needle))
    }

    fn evict_before(&mut self, now_ts_ns: u64) {
        let ttl_ns = self.config.store_ttl.as_nanos() as u64;
        while let Some(front) = self.samples.front() {
            if now_ts_ns.saturating_sub(front.ts_ns) > ttl_ns {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ProfileConfig {
        ProfileConfig {
            enabled: true,
            min_latency_ns: 1,
            store_cap: 128,
            store_ttl: Duration::from_secs(2),
            freq_hz: 99,
        }
    }

    #[test]
    fn join_uses_bpf_window_not_wall_clock() {
        let mut store = ProfileStore::new(cfg());
        store.samples.push_back(StoredSample {
            tgid: 10,
            ts_ns: 1_000_000,
            frames: vec!["slow_handler_sleep".into(), "other".into()],
        });
        store.samples.push_back(StoredSample {
            tgid: 10,
            ts_ns: 1_050_000,
            frames: vec!["slow_handler_sleep".into()],
        });
        store.samples.push_back(StoredSample {
            tgid: 99,
            ts_ns: 1_020_000,
            frames: vec!["wrong_tgid".into()],
        });
        let got = store.join(10, 990_000, 1_100_000).unwrap();
        assert!(ProfileStore::frames_contain(&got, "slow_handler_sleep"));
        assert!(!ProfileStore::frames_contain(&got, "wrong_tgid"));
    }

    #[test]
    fn join_skips_short_spans() {
        let mut store = ProfileStore::new(ProfileConfig {
            min_latency_ns: 20_000_000,
            ..cfg()
        });
        store.samples.push_back(StoredSample {
            tgid: 1,
            ts_ns: 100,
            frames: vec!["fn".into()],
        });
        assert!(store.join(1, 0, 1_000).is_none());
    }

    #[test]
    fn top_frames_by_count() {
        let mut store = ProfileStore::new(cfg());
        for _ in 0..3 {
            store.samples.push_back(StoredSample {
                tgid: 1,
                ts_ns: 500,
                frames: vec!["hot".into(), "cold".into()],
            });
        }
        store.samples.push_back(StoredSample {
            tgid: 1,
            ts_ns: 600,
            frames: vec!["cold".into()],
        });
        let got = store.join(1, 0, 1_000_000).unwrap();
        assert_eq!(got[0], "cold");
        assert_eq!(got[1], "hot");
    }
}
