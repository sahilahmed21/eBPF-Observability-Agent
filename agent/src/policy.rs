//! Detection-only policy over file-open and exec events (Phase E). No blocking.
//!
//! Rules file, one per line (`#` comments):
//!   deny_open <absolute path prefix>   e.g. `deny_open /home/agent/.ssh`
//!   deny_exec <basename>               e.g. `deny_exec nc`
//! A prefix matches the path itself or anything below it (component boundary, so `/etc/ss`
//! does not match `/etc/ssh`). shortcut: no globs and no per-process-tree scoping; scope by
//! `OBSAGENT_COMM_ALLOW` and upgrade to a process tree when agents spawn many helpers.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Open,
    Exec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub op: Op,
    pub path: String,
    pub rule: String,
}

#[derive(Default, Debug)]
pub struct Policy {
    deny_open: Vec<String>,
    deny_exec: Vec<String>,
}

impl Policy {
    pub fn parse(text: &str) -> Result<Policy, String> {
        let mut p = Policy::default();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = |why: &str| format!("policy line {}: {why}: {line:?}", i + 1);
            match line
                .split_once(char::is_whitespace)
                .map(|(k, v)| (k, v.trim()))
            {
                Some(("deny_open", v)) if v.starts_with('/') => {
                    p.deny_open.push(v.trim_end_matches('/').to_string())
                }
                Some(("deny_open", _)) => return Err(bad("deny_open needs an absolute path")),
                Some(("deny_exec", v)) if !v.is_empty() && !v.contains('/') => {
                    p.deny_exec.push(v.to_string())
                }
                Some(("deny_exec", _)) => return Err(bad("deny_exec needs a bare basename")),
                _ => return Err(bad("unknown rule")),
            }
        }
        Ok(p)
    }

    pub fn check(&self, op: Op, path: &str) -> Option<Violation> {
        let rule = match op {
            Op::Open => self
                .deny_open
                .iter()
                .find(|d| {
                    path.strip_prefix(d.as_str())
                        .is_some_and(|r| r.is_empty() || r.starts_with('/'))
                })
                .map(|d| format!("deny_open {d}")),
            Op::Exec => {
                let base = path.rsplit('/').next().unwrap_or(path);
                self.deny_exec
                    .iter()
                    .find(|d| d.as_str() == base)
                    .map(|d| format!("deny_exec {d}"))
            }
        }?;
        Some(Violation {
            op,
            path: path.to_string(),
            rule,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> Policy {
        Policy::parse(
            "# rules\ndeny_open /home/agent/.ssh\ndeny_open /etc/shadow\ndeny_exec nc\n\n",
        )
        .unwrap()
    }

    #[test]
    fn open_under_denied_prefix_is_a_violation() {
        let v = p()
            .check(Op::Open, "/home/agent/.ssh/id_ed25519")
            .expect("violation");
        assert_eq!(v.rule, "deny_open /home/agent/.ssh");
        assert_eq!(v.path, "/home/agent/.ssh/id_ed25519");
        assert!(p().check(Op::Open, "/etc/shadow").is_some());
    }

    #[test]
    fn prefix_respects_component_boundary() {
        assert!(p().check(Op::Open, "/home/agent/.sshd/x").is_none());
        assert!(p().check(Op::Open, "/etc/shadow-backup").is_none());
    }

    #[test]
    fn exec_matches_basename_only() {
        assert!(p().check(Op::Exec, "/usr/bin/nc").is_some());
        assert!(p().check(Op::Exec, "nc").is_some());
        assert!(p().check(Op::Exec, "/usr/bin/ncat").is_none());
        // an exec rule never fires on opens and vice versa
        assert!(p().check(Op::Open, "/usr/bin/nc").is_none());
        assert!(p().check(Op::Exec, "/etc/shadow").is_none());
    }

    #[test]
    fn benign_paths_pass() {
        assert!(p()
            .check(Op::Open, "/lib/x86_64-linux-gnu/libc.so.6")
            .is_none());
        assert!(p().check(Op::Exec, "/usr/bin/git").is_none());
    }

    #[test]
    fn bad_rules_are_errors_not_ignored() {
        assert!(Policy::parse("deny_open relative/path").is_err());
        assert!(Policy::parse("deny_exec").is_err());
        assert!(Policy::parse("allow_everything /x").is_err());
    }
}
