//! Cleartext h2c gRPC server (tonic). OpenSSL is not used — rustls is not either;
//! tonic's default h2c transport is for Phase 7 gRPC correctness.

use std::net::SocketAddr;
use std::time::Duration;

use grpc_slow::pb::slow_server::{Slow, SlowServer};
use grpc_slow::pb::{SleepReply, SleepRequest};
use tonic::{Request, Response, Status};

#[derive(Default)]
struct Svc;

#[tonic::async_trait]
impl Slow for Svc {
    async fn sleep(&self, req: Request<SleepRequest>) -> Result<Response<SleepReply>, Status> {
        let ms = req.into_inner().delay_ms.max(1);
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Ok(Response::new(SleepReply {}))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(18097);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    eprintln!("grpc-slow listening on http://{addr}");
    tonic::transport::Server::builder()
        .add_service(SlowServer::new(Svc))
        .serve(addr)
        .await?;
    Ok(())
}
