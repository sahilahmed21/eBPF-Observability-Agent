//! Build helpers for the Aya workspace (compile eBPF with nightly, then agent).
//!
//! Phase 0 will implement roughly:
//!   cargo xtask build-ebpf
//!   cargo xtask run

fn main() {
    eprintln!(
        "xtask: scaffold only.\n\
         Phase 0 will add build-ebpf / run commands. See docs/ROADMAP.md."
    );
}
