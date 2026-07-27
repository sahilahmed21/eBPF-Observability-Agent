//! Shared types between eBPF and userspace.
//!
//! Phase 0+: define `#[repr(C)]` event layouts and map keys here.
//! Keep this crate `no_std`-friendly for the eBPF side.

#![no_std]

// pub mod events;
// pub mod keys;

/// Schema version for RingBuf payloads. Bump when layouts change.
pub const EVENT_SCHEMA_VERSION: u32 = 0;
