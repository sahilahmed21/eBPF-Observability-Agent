//! Injectable-latency HTTP server for Phase 2 correctness (Q13).

use std::net::SocketAddr;
use std::time::Duration;

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
                tokio::time::sleep(Duration::from_millis(q.delay_ms)).await;
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
