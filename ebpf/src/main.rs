//! eBPF programs for the observability agent (Aya).
//!
//! Phase 0: replace with a trivial kprobe that loads and logs.
//! Later modules: connect, socket, http_capture, tls.

#![no_std]
#![no_main]

// #[panic_handler] and Aya entrypoints land in Phase 0 scaffolding.

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
