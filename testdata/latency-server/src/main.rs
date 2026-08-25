//! Injectable-latency HTTP server for Phase 2 correctness (Q13).
//! Phase 12: `/slow` burns CPU in a named symbol for profile join gates.
//! (thread::sleep deschedules the task; CPU-clock sampling never hits it.)

use std::hint::black_box;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query};
use axum::routing::get;
use axum::{Router, http::StatusCode};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct SlowQuery {
    #[serde(default = "default_delay")]
    delay_ms: u64,
}

fn default_delay() -> u64 {
    50
}

/// Named on-CPU burn target for Phase 12 stack join (must not be inlined).
/// Busy-wait so perf CPU-clock samples land on this tgid during the span window.
#[inline(never)]
fn slow_handler_sleep(delay_ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(delay_ms);
    let mut x = 0u64;
    while Instant::now() < deadline {
        x = black_box(x.wrapping_add(1));
    }
    black_box(x);
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(18080);
    let app = Router::new()
        .route("/fast", get(|| async { StatusCode::OK }))
        .route(
            "/slow",
            get(|Query(q): Query<SlowQuery>| async move {
                let ms = q.delay_ms;
                let _ = tokio::task::spawn_blocking(move || slow_handler_sleep(ms)).await;
                StatusCode::OK
            }),
        )
        .route(
            "/users/{id}",
            get(|Path(id): Path<String>| async move {
                (StatusCode::OK, format!("user {id}"))
            }),
        )
        .route(
            "/err",
            get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
        );

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    eprintln!("latency-server listening on http://{addr}");
    axum::serve(listener, app).await.expect("serve");
}
