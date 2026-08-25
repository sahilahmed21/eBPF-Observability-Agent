//! HTTP/2 + gRPC ingest (Phase 7).
//!
//! Demux happens **before** HTTP/1.1 reassembly. Stream_id is the correlator
//! key; leftover bytes are still keyed `(tgid, fd, dir)`.

mod conn;
mod frame;
mod hpack;

pub use conn::{H2Exchange, H2Registry};
pub use frame::{looks_like_h2, looks_like_http11};
