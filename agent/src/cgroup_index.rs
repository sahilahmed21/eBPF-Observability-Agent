//! Cgroup inode → pod UID (Phase 10 WSL identity).
//!
//! `bpf_get_current_cgroup_id()` returns the cgroupfs inode of the task's
//! current cgroup. Walk host `/sys/fs/cgroup` and map those inodes to pod UIDs
//! parsed from kubepods paths — works when BPF `tgid` is missing from `/proc`.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use log::{debug, info, warn};
use tokio::time::{interval, MissedTickBehavior};

use crate::identity::extract_pod_uid;

/// Inode (`bpf_get_current_cgroup_id`) → Kubernetes pod UID.
#[derive(Debug, Default, Clone)]
pub struct CgroupPodIndex {
    by_inode: HashMap<u64, String>,
}

impl CgroupPodIndex {
    pub fn lookup(&self, cgroup_id: u64) -> Option<&str> {
        if cgroup_id == 0 {
            return None;
        }
        self.by_inode.get(&cgroup_id).map(|s| s.as_str())
    }

    pub fn len(&self) -> usize {
        self.by_inode.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_inode.is_empty()
    }
}

/// Scan `cgroup_root` for kubepods paths; every directory whose path embeds a
/// pod UID contributes its inode → that UID (pod slice and container scope).
pub fn scan_cgroup_fs(cgroup_root: &Path) -> CgroupPodIndex {
    let mut by_inode = HashMap::new();
    if !cgroup_root.is_dir() {
        return CgroupPodIndex { by_inode };
    }
    walk(cgroup_root, &mut by_inode);
    CgroupPodIndex { by_inode }
}

fn walk(dir: &Path, out: &mut HashMap<u64, String>) {
    let path_str = dir.to_string_lossy();
    if let Some(uid) = extract_pod_uid(&path_str) {
        if let Ok(meta) = fs::metadata(dir) {
            out.insert(meta.ino(), uid);
        }
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if p.is_dir() {
            walk(&p, out);
        }
    }
}

fn lock_index(m: &Mutex<CgroupPodIndex>) -> MutexGuard<'_, CgroupPodIndex> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Background refresh of the inode → pod UID map (DaemonSet mounts host cgroupfs).
pub fn spawn_cgroup_index_if_configured() -> std::sync::Arc<Mutex<CgroupPodIndex>> {
    let root = cgroup_root_path();
    let index = std::sync::Arc::new(Mutex::new(scan_cgroup_fs(&root)));
    {
        let n = lock_index(&index).len();
        if n == 0 {
            debug!("cgroup index: 0 entries under {}", root.display());
        } else {
            info!("cgroup index: initial entries={n} root={}", root.display());
        }
    }
    let idx = std::sync::Arc::clone(&index);
    let root_bg = root;
    tokio::task::spawn(async move {
        let mut tick = interval(Duration::from_secs(15));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let next = tokio::task::spawn_blocking({
                let root_bg = root_bg.clone();
                move || scan_cgroup_fs(&root_bg)
            })
            .await;
            match next {
                Ok(next) => {
                    info!("cgroup index: entries={}", next.len());
                    *lock_index(&idx) = next;
                }
                Err(e) => warn!("cgroup index refresh join: {e:#}"),
            }
        }
    });
    index
}

fn cgroup_root_path() -> PathBuf {
    std::env::var_os("OBSAGENT_CGROUP_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/sys/fs/cgroup"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn scan_maps_pod_slice_and_scope_inodes() {
        let tmp = tempfile_dir();
        let pod = tmp
            .join("kubepods.slice")
            .join("kubepods-besteffort.slice")
            .join("kubepods-besteffort-podaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee.slice");
        let scope = pod.join(
            "cri-containerd-ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff.scope",
        );
        fs::create_dir_all(&scope).unwrap();

        let idx = scan_cgroup_fs(&tmp);
        assert!(!idx.is_empty());
        let uid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let pod_ino = fs::metadata(&pod).unwrap().ino();
        let scope_ino = fs::metadata(&scope).unwrap().ino();
        assert_eq!(idx.lookup(pod_ino), Some(uid));
        assert_eq!(idx.lookup(scope_ino), Some(uid));
        assert_eq!(idx.lookup(0), None);
        let _ = fs::remove_dir_all(&tmp);
    }

    fn tempfile_dir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "obsagent-cgroup-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        // ensure traversable
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        let _ = fs::set_permissions(&p, perms);
        p
    }
}
