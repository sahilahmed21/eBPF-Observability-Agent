//! Cumulative metrics + sampled traces export (Phase 5 / Phase 9).
//!
//! Records into [`MetricsRegistry`] and optionally [`SpanQueue`] on the drain
//! path (sync, cheap). A background task periodically snapshots and POSTs
//! OTLP/HTTP JSON. Never blocks RingBuf drain on collector I/O (Phase 5 Q7).
//! Trace POSTs are not retried; a full span queue drops incoming.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use log::{debug, warn};

use crate::metrics_registry::{HttpObs, MetricsRegistry};
use crate::profile::ProfileStore;
use crate::service_map::DstId;
use crate::trace_export::{
    build_span, sample_n_from_env, sampled, to_otlp_traces_json, HandshakeIndex, SpanQueue,
    TraceMeta, SPAN_QUEUE_CAP,
};

pub struct ExportHub {
    registry: Arc<Mutex<MetricsRegistry>>,
    spans: Arc<Mutex<SpanQueue>>,
    handshake: Arc<Mutex<HandshakeIndex>>,
    sample_n: u64,
    pub dropped: Arc<AtomicU64>,
    pub exported: Arc<AtomicU64>,
    pub http_total: Arc<AtomicU64>,
    pub http_errors: Arc<AtomicU64>,
    pub traces_sampled: Arc<AtomicU64>,
    pub traces_not_sampled: Arc<AtomicU64>,
    pub traces_dropped: Arc<AtomicU64>,
    pub traces_export_failed: Arc<AtomicU64>,
    bpf_sample_n: AtomicU32,
    enabled: bool,
    profile: Option<Arc<Mutex<ProfileStore>>>,
    pub profile_join_hits: Arc<AtomicU64>,
    pub profile_join_miss: Arc<AtomicU64>,
}

impl ExportHub {
    pub fn spawn(profile: Option<Arc<Mutex<ProfileStore>>>) -> Self {
        let registry = Arc::new(Mutex::new(MetricsRegistry::new()));
        let spans = Arc::new(Mutex::new(SpanQueue::new(SPAN_QUEUE_CAP)));
        let handshake = Arc::new(Mutex::new(HandshakeIndex::default()));
        let dropped = Arc::new(AtomicU64::new(0));
        let exported = Arc::new(AtomicU64::new(0));
        let http_total = Arc::new(AtomicU64::new(0));
        let http_errors = Arc::new(AtomicU64::new(0));
        let traces_sampled = Arc::new(AtomicU64::new(0));
        let traces_not_sampled = Arc::new(AtomicU64::new(0));
        let traces_dropped = Arc::new(AtomicU64::new(0));
        let traces_export_failed = Arc::new(AtomicU64::new(0));
        let profile_join_hits = Arc::new(AtomicU64::new(0));
        let profile_join_miss = Arc::new(AtomicU64::new(0));
        let sample_n = sample_n_from_env();

        let metrics_url = otlp_metrics_url();
        let traces_url = otlp_traces_url();
        let want_otlp = metrics_url.is_some() || traces_url.is_some();
        let client = if want_otlp {
            match reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
            {
                Ok(c) => Some(c),
                Err(e) => {
                    warn!("OTLP client build failed: {e:#}");
                    None
                }
            }
        } else {
            None
        };
        let enabled = client.is_some();
        if let Some(client) = client {
            let reg = Arc::clone(&registry);
            let span_q = Arc::clone(&spans);
            let dropped_t = Arc::clone(&dropped);
            let exported_t = Arc::clone(&exported);
            let traces_export_failed_t = Arc::clone(&traces_export_failed);
            tokio::task::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(10));
                loop {
                    tick.tick().await;
                    let metrics_body = if metrics_url.is_some() {
                        let snap = {
                            let g = reg.lock().unwrap_or_else(|p| p.into_inner());
                            if g.has_otlp_payload() {
                                Some(g.clone())
                            } else {
                                None
                            }
                        };
                        snap.map(|g| g.to_otlp_json())
                    } else {
                        None
                    };
                    let batch = {
                        let mut q = span_q.lock().unwrap_or_else(|p| p.into_inner());
                        q.take_all()
                    };
                    if let (Some(url), Some(body)) = (metrics_url.as_ref(), metrics_body) {
                        post_otlp(
                            &client,
                            url,
                            body,
                            &exported_t,
                            &dropped_t,
                            &reg,
                            None,
                        )
                        .await;
                    }
                    if let Some(url) = traces_url.as_ref() {
                        if !batch.is_empty() {
                            let n = batch.len() as u64;
                            let body = to_otlp_traces_json(&batch);
                            post_otlp(
                                &client,
                                url,
                                body,
                                &exported_t,
                                &dropped_t,
                                &reg,
                                Some((&traces_export_failed_t, n)),
                            )
                            .await;
                        }
                    }
                }
            });
        } else if want_otlp {
            debug!("OTLP export disabled (HTTP client build failed)");
        } else {
            debug!("OTLP export disabled (set OTEL_EXPORTER_OTLP_ENDPOINT or OBSAGENT_OTLP=1)");
        }

        Self {
            registry,
            spans,
            handshake,
            sample_n,
            dropped,
            exported,
            http_total,
            http_errors,
            traces_sampled,
            traces_not_sampled,
            traces_dropped,
            traces_export_failed,
            bpf_sample_n: AtomicU32::new(1),
            enabled,
            profile,
            profile_join_hits,
            profile_join_miss,
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

    pub fn set_tls_unmapped(&self, n: u64) {
        if let Ok(mut g) = self.registry.lock() {
            g.tls_unmapped = n;
        }
    }

    pub fn set_bpf_sample_n(&self, n: u32) {
        self.bpf_sample_n.store(n, Ordering::Relaxed);
        if let Ok(mut g) = self.registry.lock() {
            g.sample_n = n;
        }
    }

    pub fn bpf_sample_n(&self) -> u32 {
        self.bpf_sample_n.load(Ordering::Relaxed)
    }

    pub fn record_handshake(&self, latency_ns: u64) {
        if let Ok(mut g) = self.registry.lock() {
            g.record_handshake(latency_ns);
        }
    }

    pub fn note_handshake(&self, tgid: u32, fd: i32, latency_ns: u64, ts_ns: u64, now: Instant) {
        self.record_handshake(latency_ns);
        if let Ok(mut h) = self.handshake.lock() {
            h.note(tgid, fd, latency_ns, ts_ns, now);
        }
    }

    pub fn evict_handshake(&self, now: Instant, still_open: impl Fn(u32, i32) -> bool) {
        if let Ok(mut h) = self.handshake.lock() {
            h.evict_stale(now);
            h.evict_closed(still_open);
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
        protocol: &str,
        wire_ns: Option<u64>,
        meta: TraceMeta,
        now: Instant,
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
            protocol: protocol.to_string(),
            wire_ns,
        };
        // Sync aggregate — never wait on network (Q7).
        if let Ok(mut g) = self.registry.lock() {
            g.record(&obs);
        }
        let t_end_ns = meta.t_start_ns.saturating_add(latency_ns);
        let profile_frames = self.try_profile_join(meta.tgid, meta.t_start_ns, t_end_ns, latency_ns);
        if !self.enabled {
            return;
        }
        if !sampled(self.sample_n, meta) {
            self.traces_not_sampled.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut g) = self.registry.lock() {
                g.traces_not_sampled += 1;
            }
            return;
        }
        let handshake_ns = self
            .handshake
            .lock()
            .map(|mut h| h.peek_preceding(meta.tgid, meta.fd, meta.t_start_ns, now))
            .unwrap_or(None);
        let span = build_span(
            meta,
            src,
            &dst.label(),
            method,
            route,
            protocol,
            status,
            latency_ns,
            wire_ns,
            handshake_ns,
            profile_frames,
        );
        let queued = self
            .spans
            .lock()
            .map(|mut q| q.push(span))
            .unwrap_or(false);
        if queued {
            if handshake_ns.is_some() {
                if let Ok(mut h) = self.handshake.lock() {
                    h.consume(meta.tgid, meta.fd);
                }
            }
            self.traces_sampled.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut g) = self.registry.lock() {
                g.traces_sampled += 1;
            }
        } else {
            debug!("span queue full; dropping incoming span");
            self.traces_dropped.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut g) = self.registry.lock() {
                g.traces_dropped += 1;
            }
        }
    }

    /// Join profile samples for a completed span (independent of OTLP export and trace sampling).
    fn try_profile_join(
        &self,
        tgid: u32,
        t_start_ns: u64,
        t_end_ns: u64,
        latency_ns: u64,
    ) -> Option<[String; 5]> {
        let store = self.profile.as_ref()?;
        let mut g = store.lock().unwrap_or_else(|p| p.into_inner());
        let min = g.config().min_latency_ns;
        match g.join(tgid, t_start_ns, t_end_ns) {
            Some(frames) => {
                self.profile_join_hits.fetch_add(1, Ordering::Relaxed);
                debug!("profile join hit tgid={tgid} frames={frames:?}");
                Some(frames)
            }
            None => {
                if latency_ns >= min {
                    self.profile_join_miss.fetch_add(1, Ordering::Relaxed);
                }
                None
            }
        }
    }
}

async fn post_otlp(
    client: &reqwest::Client,
    url: &str,
    body: String,
    exported: &AtomicU64,
    dropped: &AtomicU64,
    reg: &Mutex<MetricsRegistry>,
    traces_fail: Option<(&AtomicU64, u64)>,
) {
    match client
        .post(url)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            exported.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut g) = reg.lock() {
                g.otlp_flushes_ok += 1;
            }
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let snippet: String = body.chars().take(240).collect();
            warn!("OTLP export HTTP {status}: {snippet}");
            dropped.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut g) = reg.lock() {
                g.otlp_dropped += 1;
                if let Some((_, n)) = traces_fail {
                    g.traces_export_failed = g.traces_export_failed.saturating_add(n);
                }
            }
            if let Some((ctr, n)) = traces_fail {
                ctr.fetch_add(n, Ordering::Relaxed);
            }
        }
        Err(e) => {
            warn!("OTLP export failed: {e:#}");
            dropped.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut g) = reg.lock() {
                g.otlp_dropped += 1;
                if let Some((_, n)) = traces_fail {
                    g.traces_export_failed = g.traces_export_failed.saturating_add(n);
                }
            }
            if let Some((ctr, n)) = traces_fail {
                ctr.fetch_add(n, Ordering::Relaxed);
            }
        }
    }
}

fn otlp_metrics_url() -> Option<String> {
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

fn otlp_traces_url() -> Option<String> {
    if let Ok(ep) = std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT") {
        if !ep.is_empty() {
            return Some(normalize_traces_url(&ep));
        }
    }
    otlp_metrics_url().map(|m| m.replacen("/v1/metrics", "/v1/traces", 1))
}

fn normalize_metrics_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1/metrics") {
        base.to_string()
    } else {
        format!("{base}/v1/metrics")
    }
}

fn normalize_traces_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1/traces") {
        base.to_string()
    } else {
        format!("{base}/v1/traces")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disabled_hub(sample_n: u64, enabled: bool) -> ExportHub {
        ExportHub {
            registry: Arc::new(Mutex::new(MetricsRegistry::new())),
            spans: Arc::new(Mutex::new(SpanQueue::new(SPAN_QUEUE_CAP))),
            handshake: Arc::new(Mutex::new(HandshakeIndex::default())),
            sample_n,
            dropped: Arc::new(AtomicU64::new(0)),
            exported: Arc::new(AtomicU64::new(0)),
            http_total: Arc::new(AtomicU64::new(0)),
            http_errors: Arc::new(AtomicU64::new(0)),
            traces_sampled: Arc::new(AtomicU64::new(0)),
            traces_not_sampled: Arc::new(AtomicU64::new(0)),
            traces_dropped: Arc::new(AtomicU64::new(0)),
            traces_export_failed: Arc::new(AtomicU64::new(0)),
            bpf_sample_n: AtomicU32::new(1),
            enabled,
            profile: None,
            profile_join_hits: Arc::new(AtomicU64::new(0)),
            profile_join_miss: Arc::new(AtomicU64::new(0)),
        }
    }

    fn meta() -> TraceMeta {
        TraceMeta {
            tgid: 9,
            fd: 4,
            stream_id: 0,
            t_start_ns: 100,
            is_client: true,
        }
    }

    fn hub_with_profile(enabled_otlp: bool, sample_n: u64) -> (ExportHub, Arc<Mutex<ProfileStore>>) {
        use crate::profile::{ProfileConfig, ProfileStore};
        use std::time::Duration;

        let cfg = ProfileConfig {
            enabled: true,
            min_latency_ns: 1,
            store_cap: 128,
            store_ttl: Duration::from_secs(2),
            freq_hz: 99,
        };
        let store = Arc::new(Mutex::new(ProfileStore::new(cfg)));
        let hub = ExportHub {
            profile: Some(Arc::clone(&store)),
            sample_n,
            enabled: enabled_otlp,
            ..disabled_hub(sample_n, enabled_otlp)
        };
        (hub, store)
    }

    #[test]
    fn profile_join_runs_without_otlp_export() {
        let (hub, store) = hub_with_profile(false, 1);
        store
            .lock()
            .unwrap()
            .push_symbolized(9, 150, vec!["handler_fn".into()]);
        hub.record_exchange(
            "src",
            &DstId::IpPort {
                addr: "10.0.0.1:80".into(),
            },
            "GET",
            "/slow",
            50_000_000,
            200,
            "http",
            None,
            meta(),
            Instant::now(),
        );
        assert_eq!(hub.profile_join_hits.load(Ordering::Relaxed), 1);
        assert_eq!(hub.traces_sampled.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn profile_join_runs_when_trace_not_sampled() {
        let (hub, store) = hub_with_profile(true, 10);
        let meta = (0..1_000)
            .map(|i| TraceMeta {
                tgid: 9,
                fd: 4,
                stream_id: 0,
                t_start_ns: i,
                is_client: true,
            })
            .find(|m| !sampled(10, *m))
            .expect("a non-sampled meta");
        store
            .lock()
            .unwrap()
            .push_symbolized(9, meta.t_start_ns + 1_000, vec!["handler_fn".into()]);
        hub.record_exchange(
            "src",
            &DstId::IpPort {
                addr: "10.0.0.1:80".into(),
            },
            "GET",
            "/slow",
            50_000_000,
            200,
            "http",
            None,
            meta,
            Instant::now(),
        );
        assert_eq!(hub.profile_join_hits.load(Ordering::Relaxed), 1);
        assert_eq!(hub.traces_sampled.load(Ordering::Relaxed), 0);
        assert_eq!(hub.traces_not_sampled.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn profile_join_attaches_frames_to_exported_span() {
        let (hub, store) = hub_with_profile(true, 1);
        store
            .lock()
            .unwrap()
            .push_symbolized(9, 150, vec!["hot_frame".into()]);
        hub.record_exchange(
            "src",
            &DstId::IpPort {
                addr: "10.0.0.1:80".into(),
            },
            "GET",
            "/slow",
            50_000_000,
            200,
            "http",
            None,
            meta(),
            Instant::now(),
        );
        let batch = hub.spans.lock().unwrap().take_all();
        assert_eq!(batch.len(), 1);
        let frames = batch[0].profile_frames.as_ref().expect("profile frames");
        assert_eq!(frames[0], "hot_frame");
    }

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
    fn normalize_appends_traces_path() {
        assert_eq!(
            normalize_traces_url("http://collector:4318"),
            "http://collector:4318/v1/traces"
        );
        assert_eq!(
            normalize_traces_url("http://collector:4318/v1/traces"),
            "http://collector:4318/v1/traces"
        );
        assert_eq!(
            "http://collector:4318/v1/metrics".replacen("/v1/metrics", "/v1/traces", 1),
            "http://collector:4318/v1/traces"
        );
    }

    #[test]
    fn record_updates_registry_without_endpoint() {
        // No OTEL env in unit tests → disabled flush, but local registry still records.
        let hub = disabled_hub(1, false);
        hub.record_exchange(
            "src",
            &DstId::IpPort {
                addr: "10.0.0.1:80".into(),
            },
            "GET",
            "/x",
            5_000_000,
            200,
            "http",
            None,
            meta(),
            Instant::now(),
        );
        assert_eq!(hub.http_total.load(Ordering::Relaxed), 1);
        assert_eq!(hub.registry.lock().unwrap().total_requests(), 1);
        assert_eq!(hub.traces_sampled.load(Ordering::Relaxed), 0);
        let body = hub.registry.lock().unwrap().to_otlp_json();
        assert!(body.contains("histogram"));
        assert!(body.contains("http.client.duration"));
        assert!(body.contains("obsagent.traces.sampled"));
    }

    #[test]
    fn sampled_exchange_enqueues_span() {
        let hub = disabled_hub(1, true);
        let now = Instant::now();
        hub.note_handshake(9, 4, 3_000_000, 50, now);
        hub.record_exchange(
            "src",
            &DstId::IpPort {
                addr: "10.0.0.1:80".into(),
            },
            "GET",
            "/slow?x=1",
            50_000_000,
            200,
            "http",
            Some(49_000_000),
            meta(),
            now,
        );
        assert_eq!(hub.traces_sampled.load(Ordering::Relaxed), 1);
        assert_eq!(hub.traces_not_sampled.load(Ordering::Relaxed), 0);
        let batch = hub.spans.lock().unwrap().take_all();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].name, "GET /slow");
        assert_eq!(batch[0].protocol, "http/1.1");
        assert_eq!(batch[0].handshake_ns, Some(3_000_000));
        assert_eq!(batch[0].wire_ns, Some(49_000_000));
        hub.record_exchange(
            "src",
            &DstId::IpPort {
                addr: "10.0.0.1:80".into(),
            },
            "GET",
            "/slow",
            50_000_000,
            200,
            "http",
            None,
            TraceMeta {
                t_start_ns: 200,
                ..meta()
            },
            now,
        );
        let batch2 = hub.spans.lock().unwrap().take_all();
        assert_eq!(batch2[0].handshake_ns, None);
    }

    #[test]
    fn not_sampled_skips_queue() {
        let hub = disabled_hub(10, true);
        let meta = (0..1_000)
            .map(|i| TraceMeta {
                tgid: 9,
                fd: 4,
                stream_id: 0,
                t_start_ns: i,
                is_client: true,
            })
            .find(|m| !sampled(10, *m))
            .expect("a non-sampled meta");
        hub.record_exchange(
            "src",
            &DstId::IpPort {
                addr: "10.0.0.1:80".into(),
            },
            "GET",
            "/x",
            1_000,
            200,
            "http",
            None,
            meta,
            Instant::now(),
        );
        assert_eq!(hub.traces_sampled.load(Ordering::Relaxed), 0);
        assert_eq!(hub.traces_not_sampled.load(Ordering::Relaxed), 1);
        assert_eq!(hub.spans.lock().unwrap().len(), 0);
    }

    #[test]
    fn full_queue_counts_dropped() {
        let hub = ExportHub {
            spans: Arc::new(Mutex::new(SpanQueue::new(1))),
            ..disabled_hub(1, true)
        };
        let dst = DstId::IpPort {
            addr: "10.0.0.1:80".into(),
        };
        let now = Instant::now();
        hub.record_exchange(
            "src", &dst, "GET", "/a", 1, 200, "http", None, meta(), now,
        );
        hub.record_exchange(
            "src",
            &dst,
            "GET",
            "/b",
            1,
            200,
            "http",
            None,
            TraceMeta {
                t_start_ns: 101,
                ..meta()
            },
            now,
        );
        assert_eq!(hub.traces_sampled.load(Ordering::Relaxed), 1);
        assert_eq!(hub.traces_dropped.load(Ordering::Relaxed), 1);
        assert_eq!(hub.spans.lock().unwrap().len(), 1);
    }

    #[test]
    fn queue_full_does_not_consume_handshake() {
        let hub = ExportHub {
            spans: Arc::new(Mutex::new(SpanQueue::new(1))),
            ..disabled_hub(1, true)
        };
        let dst = DstId::IpPort {
            addr: "10.0.0.1:80".into(),
        };
        let now = Instant::now();
        hub.record_exchange(
            "src", &dst, "GET", "/fill", 1, 200, "http", None, meta(), now,
        );
        hub.note_handshake(9, 4, 3_000_000, 50, now);
        hub.record_exchange(
            "src",
            &dst,
            "GET",
            "/dropped",
            1,
            200,
            "http",
            None,
            TraceMeta {
                t_start_ns: 101,
                ..meta()
            },
            now,
        );
        assert_eq!(hub.traces_dropped.load(Ordering::Relaxed), 1);
        let _ = hub.spans.lock().unwrap().take_all();
        hub.record_exchange(
            "src",
            &dst,
            "GET",
            "/kept",
            1,
            200,
            "http",
            None,
            TraceMeta {
                t_start_ns: 102,
                ..meta()
            },
            now,
        );
        let batch = hub.spans.lock().unwrap().take_all();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].handshake_ns, Some(3_000_000));
        assert_eq!(batch[0].name, "GET /kept");
    }

    #[test]
    fn bpf_sample_n_gauge_is_independent_of_trace_sample() {
        let hub = disabled_hub(10, false);
        assert_eq!(hub.bpf_sample_n(), 1);
        hub.set_bpf_sample_n(8);
        assert_eq!(hub.bpf_sample_n(), 8);
        assert_eq!(hub.registry.lock().unwrap().sample_n, 8);
        let body = hub.registry.lock().unwrap().to_otlp_json();
        assert!(body.contains("obsagent.sample_n"));
        assert!(body.contains("\"gauge\""));
    }
}
