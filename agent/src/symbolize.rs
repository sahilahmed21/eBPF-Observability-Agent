//! blazesym wrapper for perf stack IPs (Phase 12).

use std::collections::HashMap;

use blazesym::symbolize::source::{Process, Source};
use blazesym::symbolize::{Input, Symbolized, Symbolizer};
use blazesym::{Addr, Pid};

const MAX_SYM_LEN: usize = 120;

pub struct StackSymbolizer {
    symbolizer: Symbolizer,
    /// `(tgid, ip)` → truncated symbol name.
    cache: HashMap<(u32, u64), String>,
}

impl StackSymbolizer {
    pub fn new() -> Self {
        Self {
            symbolizer: Symbolizer::new(),
            cache: HashMap::new(),
        }
    }

    pub fn symbolize_ip(&mut self, tgid: u32, ip: u64) -> String {
        if ip == 0 {
            return String::new();
        }
        if let Some(s) = self.cache.get(&(tgid, ip)) {
            return s.clone();
        }
        let name = self.resolve_one(tgid, ip).unwrap_or_else(|| format!("{ip:#x}"));
        let name = truncate_sym(&name);
        self.cache.insert((tgid, ip), name.clone());
        name
    }

    pub fn symbolize_ips(&mut self, tgid: u32, ips: &[u64]) -> Vec<String> {
        ips.iter()
            .map(|&ip| self.symbolize_ip(tgid, ip))
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn evict_tgid(&mut self, tgid: u32) {
        self.cache.retain(|(t, _), _| *t != tgid);
    }

    fn resolve_one(&self, tgid: u32, ip: u64) -> Option<String> {
        if tgid == 0 {
            return None;
        }
        let src = Source::Process(Process::new(Pid::from(tgid)));
        let addr = Addr::from(ip);
        let out = self.symbolizer.symbolize(&src, Input::AbsAddr(&[addr])).ok()?;
        match out.into_iter().next()? {
            Symbolized::Sym(sym) => Some(sym.name.into_owned()),
            Symbolized::Unknown(_) => None,
        }
    }
}

impl Default for StackSymbolizer {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate_sym(s: &str) -> String {
    if s.len() <= MAX_SYM_LEN {
        return s.to_string();
    }
    let mut out = s[..MAX_SYM_LEN].to_string();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_long_symbol() {
        let long = "a".repeat(200);
        let t = truncate_sym(&long);
        assert!(t.len() <= MAX_SYM_LEN + 3);
    }
}
