//! Best-effort Kubernetes IP → namespace/name index (Phase 4 Q2, Phase 10 dest join).
//!
//! Polls the in-cluster API with the service-account token. Soft-fails when
//! not running in a cluster (`KUBERNETES_SERVICE_HOST` unset) or on HTTP errors.
//!
//! Destination join (locked, Phase 10):
//! `connect()`/`accept()` sockaddr is what BPF records. A client to
//! `api.demo.svc` therefore has **ClusterIP** as `daddr`, not a backend pod IP.
//! Mapping that ClusterIP through EndpointSlice would invent a replica the
//! syscall never saw. Honest dest for a ClusterIP is the **Service** `ns/name`.
//! Pod IPs still map to the pod. Lookup order: pod IP, then ClusterIP.
//!
//! List parse failure (invalid JSON, missing `items`) is an error: the last
//! snapshot is kept. A decoded `items: []` is the only legal empty replace.
//! Pod IPs are indexed only for Running, non-hostNetwork, non-deleting pods.

use std::collections::HashMap;
use std::fs;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use log::{debug, info, warn};
use serde::Deserialize;
use tokio::time::MissedTickBehavior;

const LIST_PAGE: &str = "500";
const MAX_LIST_PAGES: u32 = 512;
const POLL_SECS: u64 = 15;

/// Snapshot shared with the drain/export path.
#[derive(Default, Clone)]
pub struct PodIndex {
    /// Pod IPv4 (`status.podIP` / `status.podIPs`) → (namespace, name).
    by_pod_ip: HashMap<u32, (String, String)>,
    /// Service ClusterIP (`spec.clusterIP` / `spec.clusterIPs`) → (namespace, name).
    by_cluster_ip: HashMap<u32, (String, String)>,
    /// Pod UID → (namespace, name).
    by_uid: HashMap<String, (String, String)>,
}

impl PodIndex {
    /// Host-order IPv4 matching `sin_addr.s_addr` / SockMeta. Pod IP wins on collision.
    pub fn lookup_ip(&self, daddr_raw: u32) -> Option<(String, String)> {
        self.by_pod_ip
            .get(&daddr_raw)
            .or_else(|| self.by_cluster_ip.get(&daddr_raw))
            .cloned()
    }

    pub fn lookup_uid(&self, uid: &str) -> Option<(String, String)> {
        self.by_uid.get(uid).cloned()
    }

    pub fn pod_count(&self) -> usize {
        self.by_uid.len()
    }

    pub fn pod_ip_count(&self) -> usize {
        self.by_pod_ip.len()
    }

    pub fn service_ip_count(&self) -> usize {
        self.by_cluster_ip.len()
    }

    #[allow(dead_code)] // headless diagnostics
    pub fn is_empty(&self) -> bool {
        self.by_uid.is_empty() && self.by_pod_ip.is_empty() && self.by_cluster_ip.is_empty()
    }

    fn absorb_pods(&mut self, other: PodIndex) {
        self.by_uid.extend(other.by_uid);
        self.by_pod_ip.extend(other.by_pod_ip);
    }
}

fn lock_index(m: &Mutex<PodIndex>) -> MutexGuard<'_, PodIndex> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Empty index when not in-cluster; otherwise spawns a refresh task.
pub fn spawn_pod_index_if_configured() -> Arc<Mutex<PodIndex>> {
    let index = Arc::new(Mutex::new(PodIndex::default()));
    if std::env::var_os("KUBERNETES_SERVICE_HOST").is_none() {
        debug!("k8s index: KUBERNETES_SERVICE_HOST unset; peer enrichment by IP disabled");
        return index;
    }
    let idx = Arc::clone(&index);
    tokio::task::spawn(async move {
        tokio::time::sleep(poll_jitter()).await;
        let mut tick = tokio::time::interval(Duration::from_secs(POLL_SECS));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut client: Option<reqwest::Client> = None;
        loop {
            tick.tick().await;
            if client.is_none() {
                match build_k8s_client() {
                    Ok(c) => client = Some(c),
                    Err(e) => {
                        warn!("k8s HTTP client: {e:#}; keeping last snapshot");
                        continue;
                    }
                }
            }
            let Some(http) = client.as_ref() else {
                continue;
            };
            let prev_cluster = lock_index(&idx).by_cluster_ip.clone();
            match refresh_once(http, prev_cluster).await {
                Ok(next) => {
                    info!(
                        "k8s index: pods={} pod_ips={} services={}",
                        next.pod_count(),
                        next.pod_ip_count(),
                        next.service_ip_count()
                    );
                    *lock_index(&idx) = next;
                }
                Err(e) => warn!("k8s index refresh failed; keeping last snapshot: {e:#}"),
            }
        }
    });
    index
}

fn poll_jitter() -> Duration {
    let pid = std::process::id() as u64;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    Duration::from_millis(250 + ((pid.wrapping_mul(2_654_435_761) ^ nanos) % 4750))
}

fn build_k8s_client() -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(10));
    let ca = fs::read("/var/run/secrets/kubernetes.io/serviceaccount/ca.crt")
        .map_err(|e| anyhow::anyhow!("k8s CA missing: {e}"))?;
    builder = builder.add_root_certificate(reqwest::Certificate::from_pem(&ca)?);
    Ok(builder.build()?)
}

fn api_base(host: &str, port: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("https://[{host}]:{port}")
    } else {
        format!("https://{host}:{port}")
    }
}

async fn refresh_once(
    client: &reqwest::Client,
    prev_cluster: HashMap<u32, (String, String)>,
) -> anyhow::Result<PodIndex> {
    let host = std::env::var("KUBERNETES_SERVICE_HOST")?;
    let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
    let token = fs::read_to_string("/var/run/secrets/kubernetes.io/serviceaccount/token")
        .map_err(|e| anyhow::anyhow!("read SA token: {e}"))?;
    let token = token.trim();
    let base = api_base(&host, &port);

    let mut next = list_all_pods(client, &format!("{base}/api/v1/pods"), token).await?;
    match list_all_services(client, &format!("{base}/api/v1/services"), token).await {
        Ok(m) => next.by_cluster_ip = m,
        Err(e) => {
            warn!("k8s services list failed; keeping previous ClusterIP map: {e:#}");
            next.by_cluster_ip = prev_cluster;
        }
    }
    Ok(next)
}

async fn list_all_pods(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> anyhow::Result<PodIndex> {
    let mut acc = PodIndex::default();
    let mut continue_token: Option<String> = None;
    for page in 0..MAX_LIST_PAGES {
        let body = get_list_page(client, url, token, continue_token.as_deref(), page == 0).await?;
        let (page_idx, next) = parse_pod_list_page(&body)?;
        acc.absorb_pods(page_idx);
        match next {
            Some(c) if !c.is_empty() => continue_token = Some(c),
            _ => return Ok(acc),
        }
    }
    anyhow::bail!("{url}: list pagination exceeded {MAX_LIST_PAGES} pages")
}

async fn list_all_services(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> anyhow::Result<HashMap<u32, (String, String)>> {
    let mut acc = HashMap::new();
    let mut continue_token: Option<String> = None;
    for page in 0..MAX_LIST_PAGES {
        let body = get_list_page(client, url, token, continue_token.as_deref(), page == 0).await?;
        let (page_map, next) = parse_service_list_page(&body)?;
        acc.extend(page_map);
        match next {
            Some(c) if !c.is_empty() => continue_token = Some(c),
            _ => return Ok(acc),
        }
    }
    anyhow::bail!("{url}: list pagination exceeded {MAX_LIST_PAGES} pages")
}

async fn get_list_page(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    continue_token: Option<&str>,
    first_page: bool,
) -> anyhow::Result<String> {
    let mut req = client
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/json")
        .query(&[("limit", LIST_PAGE)]);
    req = if let Some(c) = continue_token {
        req.query(&[("continue", c)])
    } else if first_page {
        req.query(&[("resourceVersion", "0")])
    } else {
        req
    };
    let resp = req
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("{url}: {e}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("{url} HTTP {}", resp.status());
    }
    resp.text()
        .await
        .map_err(|e| anyhow::anyhow!("{url} body: {e}"))
}

/// Combine list bodies with the previous snapshot.
///
/// - `pods_json = None` → `Ok(None)`: keep the entire previous index.
/// - Pods body that is not a list → `Err`: keep the entire previous index.
/// - `services_json = None` or unparsable services → update pods; keep previous ClusterIP map.
pub fn merge_lists(
    prev: &PodIndex,
    pods_json: Option<&str>,
    services_json: Option<&str>,
) -> anyhow::Result<Option<PodIndex>> {
    let Some(pods_json) = pods_json else {
        return Ok(None);
    };
    let (mut next, _) = parse_pod_list_page(pods_json)?;
    next.by_cluster_ip = match services_json {
        Some(body) => match parse_service_list_page(body) {
            Ok((m, _)) => m,
            Err(_) => prev.by_cluster_ip.clone(),
        },
        None => prev.by_cluster_ip.clone(),
    };
    Ok(Some(next))
}

fn insert_v4_ip(map: &mut HashMap<u32, (String, String)>, ip: &str, ns: &str, name: &str) {
    if ip.is_empty() || ip.eq_ignore_ascii_case("none") {
        return;
    }
    let Ok(v) = ip.parse::<Ipv4Addr>() else {
        return;
    };
    let key = u32::from_ne_bytes(v.octets());
    map.insert(key, (ns.to_string(), name.to_string()));
}

#[derive(Deserialize)]
struct ListMeta {
    #[serde(default, rename = "continue")]
    continue_token: Option<String>,
}

#[derive(Deserialize)]
struct ObjectMeta {
    name: Option<String>,
    namespace: Option<String>,
    uid: Option<String>,
    #[serde(default, rename = "deletionTimestamp")]
    deletion_timestamp: Option<String>,
}

#[derive(Deserialize)]
struct PodSpec {
    #[serde(default, rename = "hostNetwork")]
    host_network: bool,
}

#[derive(Deserialize)]
struct PodIp {
    ip: Option<String>,
}

#[derive(Deserialize)]
struct PodStatus {
    phase: Option<String>,
    #[serde(rename = "podIP")]
    pod_ip: Option<String>,
    #[serde(rename = "podIPs")]
    pod_ips: Option<Vec<PodIp>>,
}

#[derive(Deserialize)]
struct PodItem {
    metadata: Option<ObjectMeta>,
    spec: Option<PodSpec>,
    status: Option<PodStatus>,
}

#[derive(Deserialize)]
struct ServiceSpec {
    #[serde(rename = "clusterIP")]
    cluster_ip: Option<String>,
    #[serde(rename = "clusterIPs")]
    cluster_ips: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ServiceItem {
    metadata: Option<ObjectMeta>,
    spec: Option<ServiceSpec>,
}

#[derive(Deserialize)]
struct KubeList<T> {
    #[serde(default)]
    metadata: Option<ListMeta>,
    items: Option<Vec<T>>,
}

fn continue_token(meta: &Option<ListMeta>) -> Option<String> {
    meta.as_ref()
        .and_then(|m| m.continue_token.clone())
        .filter(|c| !c.is_empty())
}

fn list_missing_items() -> anyhow::Error {
    anyhow::anyhow!("kube list JSON missing items array")
}

/// Decode one PodList page. Missing `items` or invalid JSON is `Err`.
pub fn parse_pod_list_page(body: &str) -> anyhow::Result<(PodIndex, Option<String>)> {
    let list: KubeList<PodItem> =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("pod list JSON: {e}"))?;
    let Some(items) = list.items else {
        return Err(list_missing_items());
    };
    let mut index = PodIndex::default();
    for item in items {
        let Some(meta) = item.metadata else {
            continue;
        };
        let name = meta.name.as_deref().unwrap_or("");
        let ns = meta.namespace.as_deref().unwrap_or("");
        let uid = meta.uid.as_deref().unwrap_or("");
        if name.is_empty() || ns.is_empty() {
            continue;
        }
        if !uid.is_empty() {
            index
                .by_uid
                .insert(uid.to_string(), (ns.to_string(), name.to_string()));
        }
        let deleting = meta.deletion_timestamp.as_deref().is_some_and(|s| !s.is_empty());
        let host_network = item.spec.as_ref().is_some_and(|s| s.host_network);
        let running = item
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            == Some("Running");
        if !running || host_network || deleting {
            continue;
        }
        if let Some(status) = item.status.as_ref() {
            if let Some(ip) = status.pod_ip.as_deref() {
                insert_v4_ip(&mut index.by_pod_ip, ip, ns, name);
            }
            if let Some(ips) = status.pod_ips.as_ref() {
                for entry in ips {
                    if let Some(ip) = entry.ip.as_deref() {
                        insert_v4_ip(&mut index.by_pod_ip, ip, ns, name);
                    }
                }
            }
        }
    }
    Ok((index, continue_token(&list.metadata)))
}

/// Decode one ServiceList page. Missing `items` or invalid JSON is `Err`.
pub fn parse_service_list_page(
    body: &str,
) -> anyhow::Result<(HashMap<u32, (String, String)>, Option<String>)> {
    let list: KubeList<ServiceItem> =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("service list JSON: {e}"))?;
    let Some(items) = list.items else {
        return Err(list_missing_items());
    };
    let mut map = HashMap::new();
    for item in items {
        let Some(meta) = item.metadata else {
            continue;
        };
        let name = meta.name.as_deref().unwrap_or("");
        let ns = meta.namespace.as_deref().unwrap_or("");
        if name.is_empty() || ns.is_empty() {
            continue;
        }
        let Some(spec) = item.spec.as_ref() else {
            continue;
        };
        if let Some(ip) = spec.cluster_ip.as_deref() {
            insert_v4_ip(&mut map, ip, ns, name);
        }
        if let Some(ips) = spec.cluster_ips.as_ref() {
            for ip in ips {
                insert_v4_ip(&mut map, ip, ns, name);
            }
        }
    }
    Ok((map, continue_token(&list.metadata)))
}

/// Convenience for tests: one page, ignore continue.
pub fn parse_pod_list_json(body: &str) -> anyhow::Result<PodIndex> {
    parse_pod_list_page(body).map(|(idx, _)| idx)
}

/// Convenience for tests: one page, ignore continue.
pub fn parse_service_cluster_ips(body: &str) -> anyhow::Result<HashMap<u32, (String, String)>> {
    parse_service_list_page(body).map(|(m, _)| m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(a: u8, b: u8, c: u8, d: u8) -> u32 {
        u32::from_ne_bytes(Ipv4Addr::new(a, b, c, d).octets())
    }

    #[test]
    fn parses_pod_list() {
        let body = r#"{
          "items": [
            {
              "metadata": {"name": "frontend", "namespace": "demo", "uid": "u1"},
              "status": {"phase": "Running", "podIP": "10.0.0.5"}
            }
          ]
        }"#;
        let idx = parse_pod_list_json(body).unwrap();
        assert_eq!(
            idx.lookup_ip(v4(10, 0, 0, 5)),
            Some(("demo".into(), "frontend".into()))
        );
        assert_eq!(
            idx.lookup_uid("u1"),
            Some(("demo".into(), "frontend".into()))
        );
    }

    #[test]
    fn parses_pod_ips_array_v4_skips_v6() {
        let body = r#"{
          "items": [
            {
              "metadata": {"name": "dual", "namespace": "demo", "uid": "u2"},
              "status": {
                "phase": "Running",
                "podIP": "10.0.0.9",
                "podIPs": [{"ip": "10.0.0.9"}, {"ip": "2001:db8::1"}, {"ip": "10.0.0.10"}]
              }
            }
          ]
        }"#;
        let idx = parse_pod_list_json(body).unwrap();
        assert_eq!(
            idx.lookup_ip(v4(10, 0, 0, 9)),
            Some(("demo".into(), "dual".into()))
        );
        assert_eq!(
            idx.lookup_ip(v4(10, 0, 0, 10)),
            Some(("demo".into(), "dual".into()))
        );
        assert_eq!(idx.pod_ip_count(), 2);
    }

    #[test]
    fn parse_service_list_cluster_ip_is_service_name() {
        let body = r#"{
          "items": [
            {
              "metadata": {"name": "api", "namespace": "demo"},
              "spec": {"clusterIP": "10.43.0.1", "clusterIPs": ["10.43.0.1", "10.43.0.2"]}
            },
            {
              "metadata": {"name": "headless", "namespace": "demo"},
              "spec": {"clusterIP": "None"}
            },
            {
              "metadata": {"name": "empty", "namespace": "demo"},
              "spec": {"clusterIP": ""}
            }
          ]
        }"#;
        let map = parse_service_cluster_ips(body).unwrap();
        assert_eq!(
            map.get(&v4(10, 43, 0, 1)).cloned(),
            Some(("demo".into(), "api".into()))
        );
        assert_eq!(
            map.get(&v4(10, 43, 0, 2)).cloned(),
            Some(("demo".into(), "api".into()))
        );
        assert!(!map.values().any(|(_, n)| n == "headless"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn lookup_pod_ip_then_cluster_ip_pod_wins_collision() {
        let pods = r#"{
          "items": [
            {
              "metadata": {"name": "frontend", "namespace": "demo", "uid": "u1"},
              "status": {"phase": "Running", "podIP": "10.0.0.5"}
            }
          ]
        }"#;
        let services = r#"{
          "items": [
            {
              "metadata": {"name": "api", "namespace": "demo"},
              "spec": {"clusterIP": "10.43.0.1"}
            },
            {
              "metadata": {"name": "should-not-win", "namespace": "demo"},
              "spec": {"clusterIP": "10.0.0.5"}
            }
          ]
        }"#;
        let idx = merge_lists(&PodIndex::default(), Some(pods), Some(services))
            .unwrap()
            .unwrap();
        assert_eq!(
            idx.lookup_ip(v4(10, 0, 0, 5)),
            Some(("demo".into(), "frontend".into()))
        );
        assert_eq!(
            idx.lookup_ip(v4(10, 43, 0, 1)),
            Some(("demo".into(), "api".into()))
        );
    }

    #[test]
    fn merge_keeps_previous_cluster_ips_when_services_list_fails() {
        let prev_services = r#"{
          "items": [
            {
              "metadata": {"name": "api", "namespace": "demo"},
              "spec": {"clusterIP": "10.43.0.1"}
            }
          ]
        }"#;
        let prev = merge_lists(
            &PodIndex::default(),
            Some(r#"{"items":[]}"#),
            Some(prev_services),
        )
        .unwrap()
        .unwrap();
        let new_pods = r#"{
          "items": [
            {
              "metadata": {"name": "frontend", "namespace": "demo", "uid": "u1"},
              "status": {"phase": "Running", "podIP": "10.0.0.5"}
            }
          ]
        }"#;
        let next = merge_lists(&prev, Some(new_pods), None).unwrap().unwrap();
        assert_eq!(
            next.lookup_ip(v4(10, 0, 0, 5)),
            Some(("demo".into(), "frontend".into()))
        );
        assert_eq!(
            next.lookup_ip(v4(10, 43, 0, 1)),
            Some(("demo".into(), "api".into()))
        );
    }

    #[test]
    fn merge_pods_fail_returns_none() {
        assert!(
            merge_lists(&PodIndex::default(), None, Some(r#"{"items":[]}"#))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn skips_non_v4_cluster_ip() {
        let body = r#"{
          "items": [
            {
              "metadata": {"name": "v6", "namespace": "demo"},
              "spec": {"clusterIP": "2001:db8::1"}
            }
          ]
        }"#;
        assert!(parse_service_cluster_ips(body).unwrap().is_empty());
    }

    #[test]
    fn invalid_json_is_err_not_empty_ok() {
        assert!(parse_pod_list_json("not-json").is_err());
        assert!(parse_pod_list_json("{}").is_err());
        assert!(parse_pod_list_json(r#"{"kind":"Status","status":"Failure"}"#).is_err());
        assert!(parse_service_cluster_ips("not-json").is_err());
        assert!(parse_service_cluster_ips(r#"{"kind":"Status"}"#).is_err());
    }

    #[test]
    fn empty_items_is_ok_empty() {
        let idx = parse_pod_list_json(r#"{"items":[]}"#).unwrap();
        assert!(idx.is_empty());
        assert!(parse_service_cluster_ips(r#"{"items":[]}"#).unwrap().is_empty());
    }

    #[test]
    fn invalid_pods_json_merge_is_err() {
        assert!(merge_lists(&PodIndex::default(), Some("{}"), Some(r#"{"items":[]}"#)).is_err());
    }

    #[test]
    fn invalid_services_json_keeps_cluster_ips() {
        let prev_services = r#"{
          "items": [
            {
              "metadata": {"name": "api", "namespace": "demo"},
              "spec": {"clusterIP": "10.43.0.1"}
            }
          ]
        }"#;
        let prev = merge_lists(
            &PodIndex::default(),
            Some(r#"{"items":[]}"#),
            Some(prev_services),
        )
        .unwrap()
        .unwrap();
        let next = merge_lists(&prev, Some(r#"{"items":[]}"#), Some("{not json}"))
            .unwrap()
            .unwrap();
        assert_eq!(
            next.lookup_ip(v4(10, 43, 0, 1)),
            Some(("demo".into(), "api".into()))
        );
    }

    #[test]
    fn host_network_ip_skipped_uid_kept() {
        let body = r#"{
          "items": [
            {
              "metadata": {"name": "kube-proxy", "namespace": "kube-system", "uid": "u3"},
              "spec": {"hostNetwork": true},
              "status": {"phase": "Running", "podIP": "10.0.0.1"}
            }
          ]
        }"#;
        let idx = parse_pod_list_json(body).unwrap();
        assert_eq!(idx.lookup_ip(v4(10, 0, 0, 1)), None);
        assert_eq!(
            idx.lookup_uid("u3"),
            Some(("kube-system".into(), "kube-proxy".into()))
        );
    }

    #[test]
    fn succeeded_and_deleting_ips_skipped() {
        let body = r#"{
          "items": [
            {
              "metadata": {"name": "done", "namespace": "demo", "uid": "ud"},
              "status": {"phase": "Succeeded", "podIP": "10.0.0.5"}
            },
            {
              "metadata": {
                "name": "dying",
                "namespace": "demo",
                "uid": "ux",
                "deletionTimestamp": "2026-01-01T00:00:00Z"
              },
              "status": {"phase": "Running", "podIP": "10.0.0.6"}
            }
          ]
        }"#;
        let idx = parse_pod_list_json(body).unwrap();
        assert_eq!(idx.lookup_ip(v4(10, 0, 0, 5)), None);
        assert_eq!(idx.lookup_ip(v4(10, 0, 0, 6)), None);
        assert_eq!(idx.lookup_uid("ud"), Some(("demo".into(), "done".into())));
        assert_eq!(idx.lookup_uid("ux"), Some(("demo".into(), "dying".into())));
    }

    #[test]
    fn continue_token_on_empty_page() {
        let body = r#"{"metadata":{"continue":"abc"},"items":[]}"#;
        let (idx, cont) = parse_pod_list_page(body).unwrap();
        assert!(idx.is_empty());
        assert_eq!(cont.as_deref(), Some("abc"));
    }

    #[test]
    fn absorb_pages_joins_continue_chunks() {
        let p1 = parse_pod_list_json(
            r#"{"items":[{"metadata":{"name":"a","namespace":"demo","uid":"ua"},"status":{"phase":"Running","podIP":"10.0.0.1"}}]}"#,
        )
        .unwrap();
        let p2 = parse_pod_list_json(
            r#"{"items":[{"metadata":{"name":"b","namespace":"demo","uid":"ub"},"status":{"phase":"Running","podIP":"10.0.0.2"}}]}"#,
        )
        .unwrap();
        let mut acc = p1;
        acc.absorb_pods(p2);
        assert_eq!(acc.lookup_uid("ua"), Some(("demo".into(), "a".into())));
        assert_eq!(acc.lookup_uid("ub"), Some(("demo".into(), "b".into())));
        assert_eq!(acc.lookup_ip(v4(10, 0, 0, 2)), Some(("demo".into(), "b".into())));
    }
}
