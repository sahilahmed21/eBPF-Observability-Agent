#![no_std]
#![no_main]

use aya_ebpf::{macros::kprobe, programs::ProbeContext};
use aya_log_ebpf::info;

// Milestone 0 smoke probe. Attached to `try_to_wake_up`, which fires constantly on any live
// system, so silence means broken rather than idle. Phase 1 replaces this with the connect
// tracepoints. Program name in the ELF is this fn name (kernel truncates at 15 chars).
#[kprobe]
pub fn smoke_probe(ctx: ProbeContext) -> u32 {
    match try_smoke_probe(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

fn try_smoke_probe(ctx: ProbeContext) -> Result<u32, u32> {
    info!(&ctx, "kprobe called");
    Ok(0)
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
