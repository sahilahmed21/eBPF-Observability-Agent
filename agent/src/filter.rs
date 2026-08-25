//! Process comm allow/deny (Phase 6) and BPF tgid maps (Phase 11).
//!
//! Filter before reassemble/correlate so denied comms never become HTTP series.
//! The agent comm (`obsagent`) is always dropped. k8s defaults also drop the
//! shipped collector (`otelcol` / `otelcol-contrib`) — identity, not :4318.
//! Userspace still filters on drain.
//!
//! BPF polarity:
//! - Deny-list (default): skip if `DENIED_TGID` contains the tgid.
//! - Allow-only (`OBSAGENT_COMM_ALLOW` set): skip unless `ALLOWED_TGID` contains
//!   the tgid. Do not insert the rest of the host into `DENIED_TGID`.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

const K8S_DEFAULT_DENY: &[&str] = &[
    "dockerd",
    "containerd",
    "kubelet",
    "kube-proxy",
    // Collector we ship in deploy/k8s/otel-collector.yaml (not a port denylist).
    "otelcol",
    "otelcol-contrib",
];
const K8S_TOKEN: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";
/// TASK_COMM_LEN basename of this binary. Always dropped, even on an allow list.
pub const SELF_COMM: &str = "obsagent";

/// Why an event is dropped before TCP/HTTP ingest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngestSkip {
    Keep,
    SelfProcess,
    Denied,
}

#[derive(Debug, Clone)]
pub struct CommFilter {
    deny: HashSet<String>,
    allow: Option<HashSet<String>>,
}

impl CommFilter {
    pub fn from_env() -> Self {
        let extra_deny = parse_comm_list(std::env::var("OBSAGENT_COMM_DENY").ok().as_deref());
        let allow = std::env::var("OBSAGENT_COMM_ALLOW")
            .ok()
            .map(|s| parse_comm_list(Some(&s)));
        let k8s = k8s_defaults_enabled(
            std::env::var("OBSAGENT_K8S").ok().as_deref(),
            Path::new(K8S_TOKEN).exists(),
        );
        Self::new(extra_deny, allow, k8s)
    }

    pub fn new(
        extra_deny: HashSet<String>,
        allow: Option<HashSet<String>>,
        k8s_defaults: bool,
    ) -> Self {
        let allow = allow.filter(|s| !s.is_empty());
        let mut deny = extra_deny;
        if k8s_defaults {
            for c in K8S_DEFAULT_DENY {
                deny.insert((*c).to_string());
            }
        }
        Self { deny, allow }
    }

    /// No operator allow/deny. Drain still classifies self-comm / self-tgid.
    pub fn is_passthrough(&self) -> bool {
        self.allow.is_none() && self.deny.is_empty()
    }

    /// `true` = keep event.
    pub fn allow(&self, comm: &str) -> bool {
        let comm = comm.trim();
        if is_self_comm(comm) {
            return false;
        }
        if let Some(allow) = &self.allow {
            return allow.contains(comm);
        }
        !self.deny.contains(comm)
    }

    /// Exclusive allow-list (`OBSAGENT_COMM_ALLOW` non-empty). BPF uses
    /// `ALLOW_ONLY` + `ALLOWED_TGID` instead of stuffing the host into `DENIED_TGID`.
    pub fn is_allow_only(&self) -> bool {
        self.allow.is_some()
    }
}

/// RingBuf `tgid` of this process. Skip before reassemble/correlate so OTLP
/// export HTTP is not ingested as application traffic.
#[allow(dead_code)]
pub fn is_self_tgid(event_tgid: u32, agent_tgid: u32) -> bool {
    event_tgid == agent_tgid
}

pub fn is_self_comm(comm: &str) -> bool {
    comm.trim() == SELF_COMM
}

/// OTLP/HTTP paths this agent (and the collector we ship) speak. Dropped by
/// route so Grafana HTTP series are not the telemetry channel. Not a port list.
pub fn is_otlp_export_route(method: &str, path: &str) -> bool {
    if !method.eq_ignore_ascii_case("POST") {
        return false;
    }
    matches!(
        path.trim_end_matches('/'),
        "/v1/traces" | "/v1/metrics" | "/v1/logs"
    )
}

/// Drop this process and deny-listed comms (including k8s otelcol) before
/// TCP aggregates and HTTP correlate. Not a collector-port denylist.
pub fn classify_ingest(
    event_tgid: u32,
    event_pid: u32,
    agent_ids: &HashSet<u32>,
    comm: &str,
    filter: &CommFilter,
) -> IngestSkip {
    if is_self_ids(event_tgid, event_pid, agent_ids) || is_self_comm(comm) {
        return IngestSkip::SelfProcess;
    }
    if !filter.allow(comm) {
        return IngestSkip::Denied;
    }
    IngestSkip::Keep
}

fn is_self_ids(event_tgid: u32, event_pid: u32, agent_ids: &HashSet<u32>) -> bool {
    agent_ids.contains(&event_tgid) || agent_ids.contains(&event_pid)
}

/// Max keys we publish into `DENIED_TGID` (matches [`obsagent_common::DENIED_TGID_ENTRIES`]).
pub const DENIED_TGID_CAP: usize = obsagent_common::DENIED_TGID_ENTRIES as usize;
/// Max keys we publish into `ALLOWED_TGID`.
pub const ALLOWED_TGID_CAP: usize = obsagent_common::ALLOWED_TGID_ENTRIES as usize;

/// Thread-group leader: `/proc/<pid>/status` `Tgid:` equals `pid`.
///
/// Numeric `/proc` names include tids. BPF `DENIED_TGID` / `ALLOWED_TGID` keys
/// are tgids (`bpf_get_current_pid_tgid() >> 32`).
pub fn is_thread_group_leader(status: &str, pid: u32) -> bool {
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Tgid:") {
            return rest.trim().parse::<u32>().ok() == Some(pid);
        }
    }
    false
}

fn for_each_leader_comm(proc_root: &Path, mut visit: impl FnMut(u32, &str) -> bool) {
    let Ok(rd) = fs::read_dir(proc_root) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let status = fs::read_to_string(e.path().join("status")).unwrap_or_default();
        if !is_thread_group_leader(&status, pid) {
            continue;
        }
        let comm = fs::read_to_string(e.path().join("comm"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if comm.is_empty() {
            continue;
        }
        if !visit(pid, &comm) {
            break;
        }
    }
}

/// Tgids that must be in `DENIED_TGID` in deny-list mode: `extra` (agent tgid
/// and calib ids) plus thread-group leaders whose comm [`CommFilter::allow`] rejects.
pub fn denied_tgids_from_proc(
    proc_root: &Path,
    filter: &CommFilter,
    extra: &HashSet<u32>,
) -> HashSet<u32> {
    let mut out = extra.clone();
    for_each_leader_comm(proc_root, |tgid, comm| {
        if out.len() >= DENIED_TGID_CAP {
            return false;
        }
        if !out.contains(&tgid) && !filter.allow(comm) {
            out.insert(tgid);
        }
        true
    });
    out
}

/// Thread-group leaders whose comm is allowed. Used when [`CommFilter::is_allow_only`].
pub fn allowed_tgids_from_proc(proc_root: &Path, filter: &CommFilter) -> HashSet<u32> {
    let mut out = HashSet::new();
    for_each_leader_comm(proc_root, |tgid, comm| {
        if out.len() >= ALLOWED_TGID_CAP {
            return false;
        }
        if filter.allow(comm) {
            out.insert(tgid);
        }
        true
    });
    out
}

/// `(remove, insert)` to move a published BPF tgid set to `desired`.
///
/// Apply **remove first**, then insert. Only record keys whose map op succeeded.
pub fn tgid_map_delta(
    published: &HashSet<u32>,
    desired: &HashSet<u32>,
) -> (Vec<u32>, Vec<u32>) {
    let remove: Vec<u32> = published.difference(desired).copied().collect();
    let insert: Vec<u32> = desired.difference(published).copied().collect();
    (remove, insert)
}

/// Kernel and namespace ids for this process: tgid, NSpid list, and live tids.
/// Userspace [`classify_ingest`] uses this set. BPF deny extras are the agent
/// **tgid** (plus calib ids), not every tid.
pub fn collect_self_ids(agent_tgid: u32) -> HashSet<u32> {
    let mut ids = HashSet::new();
    ids.insert(agent_tgid);
    if let Ok(st) = fs::read_to_string("/proc/self/status") {
        for line in st.lines() {
            if let Some(rest) = line.strip_prefix("NSpid:") {
                for p in rest.split_whitespace() {
                    if let Ok(n) = p.parse() {
                        ids.insert(n);
                    }
                }
            }
            if let Some(rest) = line.strip_prefix("Tgid:") {
                if let Ok(n) = rest.trim().parse::<u32>() {
                    ids.insert(n);
                }
            }
        }
    }
    if let Ok(rd) = fs::read_dir("/proc/self/task") {
        for e in rd.flatten() {
            if let Ok(n) = e.file_name().to_string_lossy().parse() {
                ids.insert(n);
            }
        }
    }
    ids
}

fn parse_comm_list(raw: Option<&str>) -> HashSet<String> {
    raw.unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn k8s_defaults_enabled(val: Option<&str>, token_exists: bool) -> bool {
    if token_exists {
        return true;
    }
    match val {
        None => false,
        Some(s) => {
            let s = s.trim();
            !(s.is_empty() || s == "0" || s.eq_ignore_ascii_case("false") || s.eq_ignore_ascii_case("no"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn dockerd_denied_with_k8s_defaults() {
        let f = CommFilter::new(HashSet::new(), None, true);
        assert!(!f.allow("dockerd"));
        assert!(!f.allow("kubelet"));
        assert!(!f.allow("obsagent"));
        assert!(!f.allow("otelcol"));
        assert!(!f.allow("otelcol-contrib"));
    }

    #[test]
    fn extra_deny() {
        let f = CommFilter::new(
            ["nginx".into()].into_iter().collect(),
            None,
            false,
        );
        assert!(!f.allow("nginx"));
        assert!(f.allow("dockerd"));
    }

    #[test]
    fn allow_list_is_exclusive() {
        let f = CommFilter::new(
            HashSet::new(),
            Some(["app".into()].into_iter().collect()),
            true,
        );
        assert!(f.allow("app"));
        assert!(!f.allow("dockerd"));
        assert!(!f.allow("other"));
    }

    #[test]
    fn parse_comma() {
        let s = parse_comm_list(Some(" dockerd, kubelet , "));
        assert!(s.contains("dockerd"));
        assert!(s.contains("kubelet"));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn empty_allow_set_is_not_exclusive() {
        let f = CommFilter::new(HashSet::new(), Some(HashSet::new()), false);
        assert!(f.allow("nginx"));
        assert!(f.allow("dockerd"));
    }

    #[test]
    fn k8s_zero_is_off() {
        assert!(!k8s_defaults_enabled(Some("0"), false));
        assert!(!k8s_defaults_enabled(Some("false"), false));
        assert!(k8s_defaults_enabled(Some("1"), false));
        assert!(k8s_defaults_enabled(None, true));
        assert!(!k8s_defaults_enabled(None, false));
    }

    #[test]
    fn self_tgid_is_identity() {
        assert!(is_self_tgid(42, 42));
        assert!(!is_self_tgid(42, 7));
    }

    #[test]
    fn obsagent_comm_always_denied() {
        let f = CommFilter::new(HashSet::new(), None, false);
        assert!(!f.allow("obsagent"));
        assert!(f.allow("python3"));
        let allow_self = CommFilter::new(
            HashSet::new(),
            Some(["obsagent".into()].into_iter().collect()),
            false,
        );
        assert!(!allow_self.allow("obsagent"));
    }

    #[test]
    fn classify_ingest_self_without_matching_tgid() {
        let f = CommFilter::new(HashSet::new(), None, false);
        let agent = HashSet::from([1u32]);
        assert_eq!(
            classify_ingest(99, 99, &agent, "obsagent", &f),
            IngestSkip::SelfProcess
        );
        assert_eq!(
            classify_ingest(1, 7, &agent, "python3", &f),
            IngestSkip::SelfProcess
        );
        assert_eq!(
            classify_ingest(99, 7, &agent, "python3", &f),
            IngestSkip::Keep
        );
        let tid = HashSet::from([1u32, 88]);
        assert_eq!(
            classify_ingest(88, 88, &tid, "python3", &f),
            IngestSkip::SelfProcess
        );
        let k8s = CommFilter::new(HashSet::new(), None, true);
        assert_eq!(
            classify_ingest(99, 99, &agent, "otelcol", &k8s),
            IngestSkip::Denied
        );
    }

    #[test]
    fn collect_self_ids_includes_process_id() {
        let pid = std::process::id();
        let ids = collect_self_ids(pid);
        assert!(ids.contains(&pid));
    }

    #[test]
    fn otlp_export_routes_are_post_v1() {
        assert!(is_otlp_export_route("POST", "/v1/traces"));
        assert!(is_otlp_export_route("POST", "/v1/metrics/"));
        assert!(!is_otlp_export_route("GET", "/v1/traces"));
        assert!(!is_otlp_export_route("POST", "/slow"));
    }

    fn write_proc(root: &Path, pid: u32, comm: &str, tgid: u32) {
        let dir = root.join(pid.to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
        fs::write(
            dir.join("status"),
            format!("Name:\t{comm}\nTgid:\t{tgid}\nPid:\t{pid}\n"),
        )
        .unwrap();
    }

    #[test]
    fn thread_group_leader_requires_matching_tgid() {
        assert!(is_thread_group_leader("Name:\tkubelet\nTgid:\t1001\n", 1001));
        assert!(!is_thread_group_leader("Name:\tkubelet\nTgid:\t1001\n", 3003));
        assert!(!is_thread_group_leader("Name:\tkubelet\n", 1001));
    }

    #[test]
    fn denied_tgids_from_fake_proc() {
        let root = std::env::temp_dir().join(format!(
            "obsagent-deny-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        write_proc(&root, 1001, "kubelet", 1001);
        write_proc(&root, 2002, "nginx", 2002);
        write_proc(&root, 3003, "kubelet", 1001); // tid, not a tgid
        fs::create_dir_all(root.join("notapid")).unwrap();
        let f = CommFilter::new(HashSet::new(), None, true);
        let extra = HashSet::from([42u32]);
        let got = denied_tgids_from_proc(&root, &f, &extra);
        assert!(got.contains(&42));
        assert!(got.contains(&1001));
        assert!(!got.contains(&2002));
        assert!(!got.contains(&3003));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn allowed_tgids_from_fake_proc_allow_only() {
        let root = std::env::temp_dir().join(format!(
            "obsagent-allow-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        write_proc(&root, 1001, "kubelet", 1001);
        write_proc(&root, 2002, "app", 2002);
        write_proc(&root, 3003, "app", 2002);
        let f = CommFilter::new(
            HashSet::new(),
            Some(["app".into()].into_iter().collect()),
            true,
        );
        assert!(f.is_allow_only());
        let got = allowed_tgids_from_proc(&root, &f);
        assert!(got.contains(&2002));
        assert!(!got.contains(&1001));
        assert!(!got.contains(&3003));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tgid_map_delta_remove_then_insert() {
        let published = HashSet::from([1u32, 2]);
        let desired = HashSet::from([2u32, 3]);
        let (del, ins) = tgid_map_delta(&published, &desired);
        assert_eq!(del.into_iter().collect::<HashSet<_>>(), HashSet::from([1u32]));
        assert_eq!(ins.into_iter().collect::<HashSet<_>>(), HashSet::from([3u32]));
    }
}
