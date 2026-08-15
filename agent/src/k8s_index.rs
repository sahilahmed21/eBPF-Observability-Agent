//! Best-effort Kubernetes pod IP → name/namespace index (Phase 4 Q2).
//!
//! Polls the in-cluster API with the service-account token. Soft-fails when
//! not running in a cluster (`KUBERNETES_SERVICE_HOST` unset) or on HTTP errors.

use std::collections::HashMap;
use std::fs;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::{debug, warn};

/// Snapshot shared with the drain/export path.
#[derive(Default, Clone)]
pub struct PodIndex {
    /// Host-order IPv4 → (namespace, name).
    by_ip: HashMap<u32, (String, String)>,
    /// Pod UID → (namespace, name).
    by_uid: HashMap<String, (String, String)>,
}

impl PodIndex {
    pub fn lookup_ip(&self, daddr_raw: u32) -> Option<(String, String)> {
        // Same raw `sin_addr.s_addr` representation as SockMeta / SockLatencyEvent.
        self.by_ip.get(&daddr_raw).cloned()
    }

    pub fn lookup_uid(&self, uid: &str) -> Option<(String, String)> {
        self.by_uid.get(uid).cloned()
    }

    pub fn len(&self) -> usize {
        self.by_ip.len()
    }

    #[allow(dead_code)] // used by future headless diagnostics
    pub fn is_empty(&self) -> bool {
        self.by_ip.is_empty()
    }
}

/// Returns `None` when not in-cluster; otherwise spawns a refresh task.
pub fn spawn_pod_index_if_configured() -> Arc<Mutex<PodIndex>> {
    let index = Arc::new(Mutex::new(PodIndex::default()));
    if std::env::var_os("KUBERNETES_SERVICE_HOST").is_none() {
        debug!("k8s index: KUBERNETES_SERVICE_HOST unset; peer enrichment by IP disabled");
        return index;
    }
    let idx = Arc::clone(&index);
    tokio::task::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tick.tick().await;
            match refresh_once().await {
                Ok(next) => {
                    if let Ok(mut g) = idx.lock() {
                        *g = next;
                    }
                }
                Err(e) => warn!("k8s pod index refresh failed: {e:#}"),
            }
        }
    });
    index
}

async fn refresh_once() -> anyhow::Result<PodIndex> {
    let host = std::env::var("KUBERNETES_SERVICE_HOST")?;
    let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
    let token = fs::read_to_string("/var/run/secrets/kubernetes.io/serviceaccount/token")
        .map_err(|e| anyhow::anyhow!("read SA token: {e}"))?;
    let ns_path = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";
    let _default_ns = fs::read_to_string(ns_path).unwrap_or_else(|_| "default".into());

    let url = format!("https://{host}:{port}/api/v1/pods");
    // Blocking HTTPS without extra crates: use std via spawn_blocking + ureq-less.
    // Prefer raw TCP is wrong for TLS. Use `reqwest` if available; else skip.
    refresh_with_reqwest(&url, token.trim()).await
}

async fn refresh_with_reqwest(url: &str, token: &str) -> anyhow::Result<PodIndex> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(10));
    let ca = fs::read("/var/run/secrets/kubernetes.io/serviceaccount/ca.crt");
    builder = match ca {
        Ok(pem) => builder.add_root_certificate(reqwest::Certificate::from_pem(&pem)?),
        Err(_) => {
            warn!("k8s CA missing; skipping pod index refresh");
            return Ok(PodIndex::default());
        }
    };
    let client = builder.build()?;
    let resp = client
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("pods list: {e}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("pods list HTTP {}", resp.status());
    }
    let body = resp.text().await?;
    Ok(parse_pod_list_json(&body))
}

/// Minimal JSON scrape without a full k8s client (avoids kube crate weight).
pub fn parse_pod_list_json(body: &str) -> PodIndex {
    let mut index = PodIndex::default();
    // Walk items coarsely: look for "metadata" blocks with name/namespace/uid and status.podIP.
    // Prefer serde_json if present.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
            for item in items {
                let meta = item.get("metadata");
                let name = meta
                    .and_then(|m| m.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let ns = meta
                    .and_then(|m| m.get("namespace"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let uid = meta
                    .and_then(|m| m.get("uid"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                if name.is_empty() || ns.is_empty() {
                    continue;
                }
                if !uid.is_empty() {
                    index
                        .by_uid
                        .insert(uid.to_string(), (ns.to_string(), name.to_string()));
                }
                if let Some(ip) = item
                    .pointer("/status/podIP")
                    .and_then(|p| p.as_str())
                    .and_then(|s| s.parse::<Ipv4Addr>().ok())
                {
                    let key = u32::from_ne_bytes(ip.octets());
                    index
                        .by_ip
                        .insert(key, (ns.to_string(), name.to_string()));
                }
            }
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pod_list() {
        let body = r#"{
          "items": [
            {
              "metadata": {"name": "frontend", "namespace": "demo", "uid": "u1"},
              "status": {"podIP": "10.0.0.5"}
            }
          ]
        }"#;
        let idx = parse_pod_list_json(body);
        let ip = u32::from_ne_bytes(Ipv4Addr::new(10, 0, 0, 5).octets());
        assert_eq!(
            idx.by_ip.get(&ip).cloned(),
            Some(("demo".into(), "frontend".into()))
        );
        assert_eq!(
            idx.lookup_uid("u1"),
            Some(("demo".into(), "frontend".into()))
        );
    }
}
