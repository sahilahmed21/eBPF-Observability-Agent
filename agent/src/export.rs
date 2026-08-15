//! Cumulative metrics export (Phase 5).
//!
//! Records into [`MetricsRegistry`] on the drain path (sync, cheap).
//! A background task periodically snapshots and POSTs cumulative OTLP/HTTP JSON.
//! Never blocks RingBuf drain on collector I/O (Phase 5 Q7).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::{debug, warn};

use crate::metrics_registry::{HttpObs, MetricsRegistry};
use crate::service_map::DstId;

pub struct ExportHub {
    registry: Arc<Mutex<MetricsRegistry>>,
    pub dropped: Arc<AtomicU64>,
    pub exported: Arc<AtomicU64>,
    pub http_total: Arc<AtomicU64>,
    pub http_errors: Arc<AtomicU64>,
    enabled: bool,
}

impl ExportHub {
    pub fn spawn() -> Self {
        let registry = Arc::new(Mutex::new(MetricsRegistry::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let exported = Arc::new(AtomicU64::new(0));
        let http_total = Arc::new(AtomicU64::new(0));
        let http_errors = Arc::new(AtomicU64::new(0));

        let endpoint = otlp_endpoint();
        let enabled = endpoint.is_some();
        if let Some(endpoint) = endpoint {
            let reg = Arc::clone(&registry);
            let dropped_t = Arc::clone(&dropped);
            let exported_t = Arc::clone(&exported);
            tokio::task::spawn(async move {
                let client = match reqwest::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()
                {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("OTLP client build failed: {e:#}");
                        return;
                    }
                };
                let mut tick = tokio::time::interval(Duration::from_secs(10));
                loop {
                    tick.tick().await;
                    let body = {
                        let g = reg.lock().unwrap_or_else(|p| p.into_inner());
                        if g.total_requests() == 0 && g.events_dropped == 0 {
                            continue;
                        }
                        g.to_otlp_json()
                    };
                    match client
                        .post(&endpoint)
                        .header("Content-Type", "application/json")
                        .body(body)
                        .send()
                        .await
                    {
                        Ok(resp) if resp.status().is_success() => {
                            exported_t.fetch_add(1, Ordering::Relaxed);
                            if let Ok(mut g) = reg.lock() {
                                g.otlp_flushes_ok += 1;
                            }
                        }
                        Ok(resp) => {
                            let status = resp.status();
                            let body = resp.text().await.unwrap_or_default();
                            let snippet: String = body.chars().take(240).collect();
                            warn!("OTLP export HTTP {status}: {snippet}");
                            dropped_t.fetch_add(1, Ordering::Relaxed);
                            if let Ok(mut g) = reg.lock() {
                                g.otlp_dropped += 1;
                            }
                        }
                        Err(e) => {
                            warn!("OTLP export failed: {e:#}");
                            dropped_t.fetch_add(1, Ordering::Relaxed);
                            if let Ok(mut g) = reg.lock() {
                                g.otlp_dropped += 1;
                            }
                        }
                    }
                }
            });
        } else {
            debug!("OTLP export disabled (set OTEL_EXPORTER_OTLP_ENDPOINT or OBSAGENT_OTLP=1)");
        }

        Self {
            registry,
            dropped,
            exported,
            http_total,
            http_errors,
            enabled,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn registry(&self) -> Arc<Mutex<MetricsRegistry>> {
        Arc::clone(&self.registry)
    }

    pub fn set_events_dropped(&self, n: u64) {
        if let Ok(mut g) = self.registry.lock() {
            g.events_dropped = n;
        }
    }

    pub fn set_edge_count(&self, n: u64) {
        if let Ok(mut g) = self.registry.lock() {
            g.edge_count = n;
        }
    }

    pub fn record_exchange(
        &self,
        src: &str,
        dst: &DstId,
        method: &str,
        route: &str,
        latency_ns: u64,
        status: u16,
    ) {
        self.http_total.fetch_add(1, Ordering::Relaxed);
        if status >= 500 {
            self.http_errors.fetch_add(1, Ordering::Relaxed);
        }
        let obs = HttpObs {
            src: src.to_string(),
            dst: dst.label(),
            method: method.to_string(),
            route: route.to_string(),
            latency_ns,
            status,
        };
        // Sync aggregate — never wait on network (Q7).
        if let Ok(mut g) = self.registry.lock() {
            g.record(&obs);
        }
    }
}

fn otlp_endpoint() -> Option<String> {
    if let Ok(ep) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        if !ep.is_empty() {
            return Some(normalize_metrics_url(&ep));
        }
    }
    if std::env::var_os("OBSAGENT_OTLP").is_some() {
        return Some(normalize_metrics_url("http://127.0.0.1:4318"));
    }
    None
}

fn normalize_metrics_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1/metrics") {
        base.to_string()
    } else {
        format!("{base}/v1/metrics")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_appends_metrics_path() {
        assert_eq!(
            normalize_metrics_url("http://collector:4318"),
            "http://collector:4318/v1/metrics"
        );
        assert_eq!(
            normalize_metrics_url("http://collector:4318/v1/metrics"),
            "http://collector:4318/v1/metrics"
        );
    }

    #[test]
    fn record_updates_registry_without_endpoint() {
        // No OTEL env in unit tests → disabled flush, but local registry still records.
        let hub = ExportHub {
            registry: Arc::new(Mutex::new(MetricsRegistry::new())),
            dropped: Arc::new(AtomicU64::new(0)),
            exported: Arc::new(AtomicU64::new(0)),
            http_total: Arc::new(AtomicU64::new(0)),
            http_errors: Arc::new(AtomicU64::new(0)),
            enabled: false,
        };
        hub.record_exchange(
            "src",
            &DstId::IpPort {
                addr: "10.0.0.1:80".into(),
            },
            "GET",
            "/x",
            5_000_000,
            200,
        );
        assert_eq!(hub.http_total.load(Ordering::Relaxed), 1);
        assert_eq!(hub.registry.lock().unwrap().total_requests(), 1);
        let body = hub.registry.lock().unwrap().to_otlp_json();
        assert!(body.contains("histogram"));
        assert!(body.contains("http.client.duration"));
    }
}



