//! Userspace observability agent.
//!
//! Phase 0 will load a trivial eBPF program. Later phases add the RingBuf
//! consumer, correlation, HTTP/TLS parsing, dashboard, and OTLP export.

fn main() {
    eprintln!(
        "ebpf-obs-agent: scaffold only — run Phase 0 to load the hello-world kprobe.\n\
         See README.md and docs/ROADMAP.md."
    );
}
