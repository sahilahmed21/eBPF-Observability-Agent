//! BPF I/O sample policy (Phase 11).
//!
//! This is **not** OTLP span sampling ([`crate::trace_export::sample_n_from_env`],
//! `OBSAGENT_TRACE_SAMPLE`). `OBSAGENT_SAMPLE_N` pins the BPF denominator and
//! disables auto. Auto uses `{1,2,4,8,16}`: double on drops, halve after 30 s
//! with no drops. `n <= 1` means keep every fd.

/// Auto policy ceiling (sticky keep still applies to already-decided fds).
pub const SAMPLE_N_MAX: u32 = 16;
/// 1 s ticks with `drops_delta == 0` before N is halved.
pub const DECAY_TICKS: u32 = 30;

/// Pin from `OBSAGENT_SAMPLE_N`. `None` = auto (start at 1).
///
/// `0` and non-numeric values are treated as unset.
pub fn bpf_sample_n_from_env() -> Option<u32> {
    parse_bpf_sample_n(std::env::var("OBSAGENT_SAMPLE_N").ok().as_deref())
}

/// Parse a BPF SAMPLE_N pin. `None` means auto.
pub fn parse_bpf_sample_n(raw: Option<&str>) -> Option<u32> {
    raw.and_then(|s| s.parse::<u32>().ok()).filter(|n| *n >= 1)
}

/// Keep this fd's I/O given SAMPLE_N and a uniform random `rnd`.
///
/// BPF uses `bpf_get_prandom_u32()` as `rnd`. Never `% 0`.
pub fn keep_io(n: u32, rnd: u32) -> bool {
    if n <= 1 {
        true
    } else {
        rnd % n == 0
    }
}

/// Next auto N and quiet-tick counter.
///
/// `drops_delta > 0` → `min(16, n*2)` (from 1 this is 2) and reset quiet.
/// After [`DECAY_TICKS`] quiet ticks → `max(1, n/2)` and reset quiet.
pub fn next_auto_n(current: u32, drops_delta: u64, quiet_ticks: u32) -> (u32, u32) {
    let n = current.clamp(1, SAMPLE_N_MAX);
    if drops_delta > 0 {
        return (n.saturating_mul(2).min(SAMPLE_N_MAX), 0);
    }
    let quiet = quiet_ticks.saturating_add(1);
    if quiet >= DECAY_TICKS {
        ((n / 2).max(1), 0)
    } else {
        (n, quiet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_all_when_n_leq_1() {
        assert!(keep_io(0, 7));
        assert!(keep_io(1, 7));
        assert!(keep_io(1, 0));
    }

    #[test]
    fn keep_io_n10_is_exact_tenth_on_sequential_rnd() {
        let kept = (0..10_000u32).filter(|r| keep_io(10, *r)).count();
        assert_eq!(kept, 1_000);
    }

    #[test]
    fn many_fds_n10_keep_about_one_tenth() {
        // One draw per fd (the sticky model). Not 10× fewer I/O on a single fd.
        let kept = (0..1_000u32)
            .filter(|fd| keep_io(10, fd.wrapping_mul(0x9E37_79B9)))
            .count();
        assert!(kept > 50 && kept < 200, "kept={kept}");
    }

    #[test]
    fn auto_doubles_on_drops_and_caps() {
        assert_eq!(next_auto_n(1, 1, 9), (2, 0));
        assert_eq!(next_auto_n(2, 3, 0), (4, 0));
        assert_eq!(next_auto_n(8, 1, 0), (16, 0));
        assert_eq!(next_auto_n(16, 99, 0), (16, 0));
    }

    #[test]
    fn auto_halves_after_30_quiet_ticks() {
        assert_eq!(next_auto_n(8, 0, 28), (8, 29));
        assert_eq!(next_auto_n(8, 0, 29), (4, 0));
        assert_eq!(next_auto_n(1, 0, 29), (1, 0));
    }

    #[test]
    fn drops_reset_quiet() {
        assert_eq!(next_auto_n(4, 1, 29), (8, 0));
    }

    #[test]
    fn env_pin_rejects_zero() {
        assert_eq!(parse_bpf_sample_n(Some("10")), Some(10));
        assert_eq!(parse_bpf_sample_n(Some("1")), Some(1));
        assert_eq!(parse_bpf_sample_n(Some("0")), None);
        assert_eq!(parse_bpf_sample_n(Some("nope")), None);
        assert_eq!(parse_bpf_sample_n(Some("")), None);
        assert_eq!(parse_bpf_sample_n(None), None);
    }
}
