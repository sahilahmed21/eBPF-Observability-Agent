#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{bpf_get_current_pid_tgid, bpf_ktime_get_ns, bpf_probe_read_user},
    macros::{kprobe, map, tracepoint},
    maps::{Array, HashMap, RingBuf},
    programs::{ProbeContext, TracePointContext},
};
use aya_log_ebpf::info;
use obsagent_common::{
    AF_INET, EVENTS_RINGBUF_BYTES, EventKind, PENDING_MAP_ENTRIES, PendingEnter, SockLatencyEvent,
};

// Tracepoint field offsets from this kernel's format files (WSL2 6.6).
const ENTER_SOCKADDR_OFF: usize = 24;
const EXIT_RET_OFF: usize = 16;

#[map]
static PENDING: HashMap<u32, PendingEnter> =
    HashMap::<u32, PendingEnter>::with_max_entries(PENDING_MAP_ENTRIES, 0);

#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(EVENTS_RINGBUF_BYTES, 0);

#[map]
static DROPS: Array<u64> = Array::with_max_entries(1, 0);

#[repr(C)]
struct SockAddrIn {
    sin_family: u16,
    sin_port: u16,
    sin_addr: u32,
}

fn tid() -> u32 {
    bpf_get_current_pid_tgid() as u32
}

fn pid_tgid() -> (u32, u32) {
    let v = bpf_get_current_pid_tgid();
    ((v >> 32) as u32, v as u32)
}

fn bump_drop() {
    if let Some(ptr) = DROPS.get_ptr_mut(0) {
        unsafe {
            *ptr += 1;
        }
    }
}

fn read_sockaddr_v4(ptr: *const SockAddrIn) -> Result<(u32, u16), i64> {
    let sa: SockAddrIn = unsafe { bpf_probe_read_user(ptr)? };
    if sa.sin_family != AF_INET {
        return Err(1);
    }
    Ok((sa.sin_addr, sa.sin_port))
}

fn emit(kind: EventKind, ret: i64, latency_ns: u64, ts_ns: u64, daddr_be: u32, dport_be: u16) {
    let (tgid, pid) = pid_tgid();
    let Some(mut slot) = EVENTS.reserve::<SockLatencyEvent>(0) else {
        bump_drop();
        return;
    };
    let ev = SockLatencyEvent {
        kind: kind as u8,
        _pad0: [0; 7],
        pid,
        tgid,
        ret,
        latency_ns,
        ts_ns,
        daddr_be,
        dport_be,
        _pad1: 0,
    };
    slot.write(ev);
    slot.submit(0);
}

// --- Milestone 0 smoke probe (Q9: keep alongside) ---

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

// --- connect ---

#[tracepoint]
pub fn enter_connect(ctx: TracePointContext) -> u32 {
    match try_enter_connect(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_enter_connect(ctx: &TracePointContext) -> Result<(), i64> {
    let sockaddr: *const SockAddrIn = unsafe { ctx.read_at(ENTER_SOCKADDR_OFF)? };
    let (daddr_be, dport_be) = match read_sockaddr_v4(sockaddr) {
        Ok(v) => v,
        Err(_) => return Ok(()), // non-IPv4: ignore (Q4)
    };
    let pending = PendingEnter {
        ts_ns: unsafe { bpf_ktime_get_ns() },
        daddr_be,
        dport_be,
        has_addr: 1,
        _pad: 0,
        sockaddr_ptr: 0,
    };
    PENDING.insert(&tid(), &pending, 0)?;
    Ok(())
}

#[tracepoint]
pub fn exit_connect(ctx: TracePointContext) -> u32 {
    match try_exit_kind(&ctx, EventKind::Connect) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

// --- accept4 ---

#[tracepoint]
pub fn enter_accept4(ctx: TracePointContext) -> u32 {
    match try_enter_accept4(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_enter_accept4(ctx: &TracePointContext) -> Result<(), i64> {
    let sockaddr: u64 = unsafe { ctx.read_at(ENTER_SOCKADDR_OFF)? };
    let pending = PendingEnter {
        ts_ns: unsafe { bpf_ktime_get_ns() },
        daddr_be: 0,
        dport_be: 0,
        has_addr: 0,
        _pad: 0,
        sockaddr_ptr: sockaddr,
    };
    PENDING.insert(&tid(), &pending, 0)?;
    Ok(())
}

#[tracepoint]
pub fn exit_accept4(ctx: TracePointContext) -> u32 {
    match try_exit_kind(&ctx, EventKind::Accept) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_exit_kind(ctx: &TracePointContext, kind: EventKind) -> Result<(), i64> {
    let tid = tid();
    let Some(pending) = (unsafe { PENDING.get(&tid) }) else {
        return Ok(());
    };
    let pending = *pending;
    let _ = PENDING.remove(&tid);

    let ret: i64 = unsafe { ctx.read_at(EXIT_RET_OFF)? };
    let now = unsafe { bpf_ktime_get_ns() };
    let latency_ns = now.saturating_sub(pending.ts_ns);

    let (daddr_be, dport_be) = if pending.has_addr == 1 {
        (pending.daddr_be, pending.dport_be)
    } else if pending.sockaddr_ptr != 0 && ret >= 0 {
        match read_sockaddr_v4(pending.sockaddr_ptr as *const SockAddrIn) {
            Ok(v) => v,
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    };

    emit(kind, ret, latency_ns, now, daddr_be, dport_be);
    Ok(())
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
