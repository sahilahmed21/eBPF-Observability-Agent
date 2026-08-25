//! Local process identity: tgid → cgroup → container / pod UID (Phase 4 Q3).
//!
//! Parses `/proc/<tgid>/cgroup` and `/proc/<tgid>/comm`. Does not call the
//! Kubernetes API — pod *names* come from [`crate::k8s_index`] when available.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a resolved identity stays cached.
pub const CACHE_TTL: Duration = Duration::from_secs(30);

/// Max cached tgids (cardinality guard).
pub const CACHE_CAP: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    /// Stable label for maps / OTLP attributes.
    pub label: String,
    pub tgid: u32,
    pub comm: String,
    pub container_id: Option<String>,
    pub pod_uid: Option<String>,
}

impl NodeId {
    pub fn proc_fallback(tgid: u32, comm: &str) -> Self {
        let safe = if comm.is_empty() { "unknown" } else { comm };
        Self {
            label: format!("proc:{safe}:{tgid}"),
            tgid,
            comm: safe.to_string(),
            container_id: None,
            pod_uid: None,
        }
    }
}

#[derive(Clone)]
struct CacheEntry {
    id: NodeId,
    at: Instant,
}

/// Resolves local process identity with a bounded TTL cache.
pub struct IdentityResolver {
    proc_root: PathBuf,
    cache: HashMap<u32, CacheEntry>,
    comm_cache: HashMap<u32, (String, Instant)>,
}

impl Default for IdentityResolver {
    fn default() -> Self {
        let root = std::env::var_os("OBSAGENT_PROC_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                choose_proc_root(
                    Path::new("/host/proc"),
                    std::process::id(),
                    Path::new("/proc"),
                )
            });
        Self::new(root)
    }
}

/// Use `/host/proc` only when this process is visible there (hostPID DaemonSet).
/// An empty leftover `/host/proc` on a laptop must not hide `/proc/<pid>/comm`.
fn choose_proc_root(host_proc: &Path, self_pid: u32, fallback: &Path) -> PathBuf {
    if host_proc
        .join(self_pid.to_string())
        .join("comm")
        .is_file()
    {
        host_proc.to_path_buf()
    } else {
        fallback.to_path_buf()
    }
}

impl IdentityResolver {
    pub fn new(proc_root: impl Into<PathBuf>) -> Self {
        Self {
            proc_root: proc_root.into(),
            cache: HashMap::new(),
            comm_cache: HashMap::new(),
        }
    }

    /// `/proc` (or `/host/proc`) used for comm and cgroup reads.
    pub fn proc_root(&self) -> &Path {
        &self.proc_root
    }

    pub fn resolve(&mut self, tgid: u32, now: Instant) -> NodeId {
        if let Some(e) = self.cache.get(&tgid) {
            if now.duration_since(e.at) <= CACHE_TTL {
                return e.id.clone();
            }
        }
        let id = self.resolve_uncached(tgid);
        if self.cache.len() >= CACHE_CAP {
            self.cache.retain(|_, e| now.duration_since(e.at) <= CACHE_TTL);
        }
        if self.cache.len() >= CACHE_CAP {
            self.cache.clear();
        }
        self.cache.insert(
            tgid,
            CacheEntry {
                id: id.clone(),
                at: now,
            },
        );
        id
    }

    /// `/proc/<tgid>/comm` only. Does not parse cgroup or fill the identity cache.
    pub fn comm(&mut self, tgid: u32, now: Instant) -> String {
        if let Some((c, at)) = self.comm_cache.get(&tgid) {
            if now.duration_since(*at) <= CACHE_TTL {
                return c.clone();
            }
        }
        let dir = self.proc_root.join(tgid.to_string());
        let mut c = read_comm(&dir.join("comm"));
        if c.is_empty() {
            c = read_cmdline_basename(&dir.join("cmdline"));
        }
        if self.comm_cache.len() >= CACHE_CAP {
            self.comm_cache
                .retain(|_, (_, at)| now.duration_since(*at) <= CACHE_TTL);
        }
        if self.comm_cache.len() >= CACHE_CAP {
            self.comm_cache.clear();
        }
        self.comm_cache.insert(tgid, (c.clone(), now));
        c
    }

    /// Comm for the process, trying thread id if tgid's `/proc` entry is missing.
    pub fn comm_pair(&mut self, tgid: u32, pid: u32, now: Instant) -> String {
        let c = self.comm(tgid, now);
        if !c.is_empty() || pid == 0 || pid == tgid {
            return c;
        }
        self.comm(pid, now)
    }

    fn resolve_uncached(&self, tgid: u32) -> NodeId {
        let mut comm = read_comm(&self.proc_root.join(tgid.to_string()).join("comm"));
        if comm.is_empty() {
            comm = read_cmdline_basename(&self.proc_root.join(tgid.to_string()).join("cmdline"));
        }
        let cgroup = fs::read_to_string(self.proc_root.join(tgid.to_string()).join("cgroup"))
            .unwrap_or_default();
        let parsed = parse_cgroup(&cgroup);
        match (parsed.pod_uid, parsed.container_id) {
            (Some(uid), cid) => {
                let short = short_uid(&uid);
                NodeId {
                    label: format!("pod:{short}"),
                    tgid,
                    comm,
                    container_id: cid,
                    pod_uid: Some(uid),
                }
            }
            (None, Some(cid)) => {
                let short = short_cid(&cid);
                NodeId {
                    label: format!("ctr:{short}"),
                    tgid,
                    comm,
                    container_id: Some(cid),
                    pod_uid: None,
                }
            }
            (None, None) => NodeId::proc_fallback(tgid, &comm),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CgroupIds {
    pub pod_uid: Option<String>,
    pub container_id: Option<String>,
}

/// Extract Kubernetes pod UID / container id from cgroup v1/v2 text.
pub fn parse_cgroup(text: &str) -> CgroupIds {
    let mut out = CgroupIds::default();
    for line in text.lines() {
        let path = line.split(':').next_back().unwrap_or(line);
        if out.pod_uid.is_none() {
            if let Some(uid) = extract_pod_uid(path) {
                out.pod_uid = Some(uid);
            }
        }
        if out.container_id.is_none() {
            if let Some(cid) = extract_container_id(path) {
                out.container_id = Some(cid);
            }
        }
    }
    out
}

/// Parse a pod UID out of a cgroup path (slice or cgroupfs). `pub(crate)` for
/// [`crate::cgroup_index`] inode walks.
pub(crate) fn extract_pod_uid(path: &str) -> Option<String> {
    // systemd slice: kubepods-{qos}-pod<UID>.slice
    // cgroupfs (k3s): .../pod<UID>/...
    const SLICE_PREFIXES: &[&str] = &[
        "kubepods-guaranteed-pod",
        "kubepods-burstable-pod",
        "kubepods-besteffort-pod",
        "kubepods-pod",
    ];
    for part in path.split('/') {
        for prefix in SLICE_PREFIXES {
            if let Some(rest) = part.strip_prefix(prefix) {
                let uid = rest.strip_suffix(".slice").unwrap_or(rest);
                let uid = uid.replace('_', "-");
                if looks_like_uid(&uid) {
                    return Some(uid);
                }
            }
        }
        if let Some(rest) = part.strip_prefix("pod") {
            let uid = rest.replace('_', "-");
            if looks_like_uid(&uid) {
                return Some(uid);
            }
        }
    }
    None
}

fn extract_container_id(path: &str) -> Option<String> {
    for part in path.split('/') {
        if let Some(rest) = part.strip_prefix("cri-containerd-") {
            let id = rest.strip_suffix(".scope").unwrap_or(rest);
            if id.len() >= 12 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Some(id.to_string());
            }
        }
        if let Some(rest) = part.strip_prefix("docker-") {
            let id = rest.strip_suffix(".scope").unwrap_or(rest);
            if id.len() >= 12 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Some(id.to_string());
            }
        }
        // cgroup v1: .../docker/<64hex> or .../containerd/<64hex>
        if part.len() == 64 && part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(part.to_string());
        }
    }
    None
}

fn looks_like_uid(s: &str) -> bool {
    // 8-4-4-4-12 hex with dashes
    let b = s.as_bytes();
    b.len() == 36
        && b[8] == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
        && s.bytes()
            .filter(|&c| c != b'-')
            .all(|c| c.is_ascii_hexdigit())
}

fn short_uid(uid: &str) -> &str {
    uid.get(..8).unwrap_or(uid)
}

fn short_cid(cid: &str) -> &str {
    cid.get(..12).unwrap_or(cid)
}

fn read_comm(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|s| {
            s.trim_matches(|c: char| c.is_whitespace() || c == '\0')
                .to_string()
        })
        .unwrap_or_default()
}

/// Phase 5 Q9: when `comm` is empty, use first argv basename from cmdline.
fn read_cmdline_basename(path: &Path) -> String {
    let Ok(bytes) = fs::read(path) else {
        return String::new();
    };
    let first = bytes.split(|&b| b == 0).next().unwrap_or(&[]);
    if first.is_empty() {
        return String::new();
    }
    let path = String::from_utf8_lossy(first);
    Path::new(path.as_ref())
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path.as_ref())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_guaranteed_slice() {
        let text = "0::/kubepods.slice/kubepods-guaranteed.slice/kubepods-guaranteed-podaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee.slice/cri-containerd-ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff.scope\n";
        let ids = parse_cgroup(text);
        assert_eq!(
            ids.pod_uid.as_deref(),
            Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        );
    }

    #[test]
    fn parses_k3s_cgroupfs_pod_uid() {
        let text = "0::/kubepods/burstable/podaaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/cri-containerd-ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\n";
        let ids = parse_cgroup(text);
        assert_eq!(
            ids.pod_uid.as_deref(),
            Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        );
    }

    #[test]
    fn parses_cri_containerd_cgroup_v2() {
        let text = "0::/kubepods.slice/kubepods-burstable.slice/kubepods-burstable-podabc12345_def6_7890_abcd_ef1234567890.slice/cri-containerd-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.scope\n";
        let ids = parse_cgroup(text);
        assert_eq!(
            ids.pod_uid.as_deref(),
            Some("abc12345-def6-7890-abcd-ef1234567890")
        );
        assert!(
            ids.container_id
                .as_ref()
                .is_some_and(|c| c.starts_with("0123456789ab"))
        );
    }

    #[test]
    fn parses_docker_scope() {
        let text = "0::/system.slice/docker-deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef.scope\n";
        let ids = parse_cgroup(text);
        assert!(ids.pod_uid.is_none());
        assert_eq!(
            ids.container_id.as_deref(),
            Some("deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef")
        );
    }

    #[test]
    fn empty_cgroup_is_empty_ids() {
        assert_eq!(parse_cgroup(""), CgroupIds::default());
    }

    #[test]
    fn host_proc_ignored_unless_self_comm_exists() {
        let missing = std::env::temp_dir().join(format!(
            "obsagent-host-missing-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&missing);
        fs::create_dir_all(&missing).unwrap();
        let fb = Path::new("/proc");
        assert_eq!(choose_proc_root(&missing, 1, fb), fb);
        let present = missing.join("1");
        fs::create_dir_all(&present).unwrap();
        fs::write(present.join("comm"), "obsagent\n").unwrap();
        assert_eq!(choose_proc_root(&missing, 1, fb), missing);
        let _ = fs::remove_dir_all(&missing);
    }

    #[test]
    fn comm_reads_only_comm_file() {
        let root = std::env::temp_dir().join(format!("obsagent-comm-{}", std::process::id()));
        let p = root.join("9");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("comm"), "nginx\n").unwrap();
        let mut r = IdentityResolver::new(&root);
        assert_eq!(r.comm(9, Instant::now()), "nginx");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn comm_falls_back_to_cmdline() {
        let root = std::env::temp_dir().join(format!("obsagent-comm-cmd-{}", std::process::id()));
        let p = root.join("3");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("comm"), "\0\n").unwrap();
        fs::write(p.join("cmdline"), b"/tmp/obsotlpsink9\0").unwrap();
        let mut r = IdentityResolver::new(&root);
        assert_eq!(r.comm(3, Instant::now()), "obsotlpsink9");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolver_uses_proc_fixture() {
        let dir = tempfile_proc_tree();
        let mut r = IdentityResolver::new(&dir);
        let id = r.resolve(42, Instant::now());
        assert_eq!(id.comm, "demo");
        assert_eq!(id.pod_uid.as_deref(), Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"));
        assert!(id.label.starts_with("pod:aaaaaaaa"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cmdline_basename_when_comm_empty() {
        let root = std::env::temp_dir().join(format!("obsagent-cmd-{}", std::process::id()));
        let p = root.join("7");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("comm"), "\n").unwrap();
        // cmdline is NUL-separated
        fs::write(p.join("cmdline"), b"/usr/bin/python3\0-c\0pass\0").unwrap();
        fs::write(p.join("cgroup"), "0::/\n").unwrap();
        let mut r = IdentityResolver::new(&root);
        let id = r.resolve(7, Instant::now());
        assert_eq!(id.comm, "python3");
        assert!(id.label.contains("python3"));
        let _ = fs::remove_dir_all(&root);
    }

    fn tempfile_proc_tree() -> PathBuf {
        let root = std::env::temp_dir().join(format!("obsagent-id-{}", std::process::id()));
        let p = root.join("42");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("comm"), "demo\n").unwrap();
        let mut f = fs::File::create(p.join("cgroup")).unwrap();
        writeln!(
            f,
            "0::/kubepods.slice/kubepods-podaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee.slice/cri-containerd-ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff.scope"
        )
        .unwrap();
        root
    }
}
