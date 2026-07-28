#![no_std]

// Kernel <-> userspace ABI. Empty until Phase 1 defines the first event.
//
// Event structs land here as `#[repr(C)]` with `unsafe impl aya::Pod` behind the `user` feature.
// Phase 0 only proves this crate compiles for both the host and bpfel-unknown-none.
