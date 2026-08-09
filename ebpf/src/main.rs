#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{
        bpf_get_current_pid_tgid, bpf_ktime_get_ns, bpf_probe_read_user,
        bpf_probe_read_user_buf,
    },
    macros::{kprobe, map, tracepoint},
    maps::{Array, HashMap, PerCpuArray, RingBuf},
    programs::{ProbeContext, TracePointContext},
};
use aya_log_ebpf::info;
use obsagent_common::{
    AF_INET, EVENTS_RINGBUF_BYTES, EventKind, IoDir, PENDING_MAP_ENTRIES, PendingEnter,
    PendingIo, SOCK_IO_PREFIX_LEN, SockIoEvent, SockLatencyEvent,
};

// Tracepoint field offsets from this kernel's format files (WSL2 6.6).
const ENTER_FD_OFF: usize = 16;
const ENTER_SOCKADDR_OFF: usize = 24;
const ENTER_BUF_OFF: usize = 24;
const EXIT_RET_OFF: usize = 16;

#[map]
static PENDING: HashMap<u32, PendingEnter> =
    HashMap::<u32, PendingEnter>::with_max_entries(PENDING_MAP_ENTRIES, 0);

#[map]
static PENDING_IO: HashMap<u32, PendingIo> =
    HashMap::<u32, PendingIo>::with_max_entries(PENDING_MAP_ENTRIES, 0);

/// Q4: fds observed via connect/accept (process fd table). Key = (tgid, fd).
#[map]
static SOCK_FDS: HashMap<u64, u8> =
    HashMap::<u64, u8>::with_max_entries(PENDING_MAP_ENTRIES, 0);

#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(EVENTS_RINGBUF_BYTES, 0);

#[map]
static DROPS: Array<u64> = Array::with_max_entries(1, 0);

/// Scratch for SockIoEvent (288B) — avoid BPF stack overflow.
#[map]
static IO_SCRATCH: PerCpuArray<SockIoEvent> = PerCpuArray::with_max_entries(1, 0);

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

fn sock_fd_key(tgid: u32, fd: u32) -> u64 {
    ((tgid as u64) << 32) | (fd as u64)
}

fn mark_sock_fd(fd: i64) {
    if fd < 0 || fd > u32::MAX as i64 {
        return;
    }
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd as u32);
    let one: u8 = 1;
    let _ = SOCK_FDS.insert(&key, &one, 0);
}

fn unmark_sock_fd(fd: u32) {
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd);
    let _ = SOCK_FDS.remove(&key);
}

fn is_marked_sock_fd(fd: u32) -> bool {
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd);
    unsafe { SOCK_FDS.get(&key).is_some() }
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

fn looks_like_http_fixed(prefix: &[u8; SOCK_IO_PREFIX_LEN], len: u16) -> bool {
    if len < 4 {
        return false;
    }
    let a = prefix[0];
    let b = prefix[1];
    let c = prefix[2];
    let d = prefix[3];
    (a == b'G' && b == b'E' && c == b'T' && d == b' ')
        || (a == b'P' && b == b'O' && c == b'S' && d == b'T')
        || (a == b'P' && b == b'U' && c == b'T' && d == b' ')
        || (a == b'H' && b == b'E' && c == b'A' && d == b'D')
        || (a == b'H' && b == b'T' && c == b'T' && d == b'P')
        || (a == b'D' && b == b'E' && c == b'L' && d == b'E')
        || (a == b'P' && b == b'A' && c == b'T' && d == b'C')
        || (a == b'O' && b == b'P' && c == b'T' && d == b'I')
}

fn emit_io(pending: &PendingIo, ret: i64) {
    let now = unsafe { bpf_ktime_get_ns() };
    let (tgid, pid) = pid_tgid();

    let prefix_len: u16 = if ret <= 0 {
        0
    } else if ret as usize > SOCK_IO_PREFIX_LEN {
        SOCK_IO_PREFIX_LEN as u16
    } else {
        ret as u16
    };

    let Some(scratch) = IO_SCRATCH.get_ptr_mut(0) else {
        return;
    };
    let ev = unsafe { &mut *scratch };
    ev.kind = EventKind::SockIo as u8;
    ev.dir = pending.dir;
    ev.prefix_len = prefix_len;
    ev.fd = pending.fd;
    ev.pid = pid;
    ev.tgid = tgid;
    ev.ret = ret;
    ev.ts_ns = now;
    ev.prefix = [0; SOCK_IO_PREFIX_LEN];

    // Copy at most prefix_len bytes. Verifier needs a fixed dest size; we use the
    // full array but only treat prefix_len as valid (userspace caps too).
    if prefix_len > 0 {
        let _ = unsafe {
            bpf_probe_read_user_buf(pending.buf_ptr as *const u8, &mut ev.prefix)
        };
    }

    // Second gate: skip non-HTTP on marked sockets (SSH, etc.).
    if !looks_like_http_fixed(&ev.prefix, prefix_len) {
        return;
    }

    let Some(mut slot) = EVENTS.reserve::<SockIoEvent>(0) else {
        bump_drop();
        return;
    };
    slot.write(*ev);
    slot.submit(0);
}

fn try_enter_io(ctx: &TracePointContext, dir: IoDir) -> Result<(), i64> {
    let fd: u64 = unsafe { ctx.read_at(ENTER_FD_OFF)? };
    let fd = fd as u32;
    // Q4: only fds marked via connect/accept (enter-side — cheap).
    if !is_marked_sock_fd(fd) {
        return Ok(());
    }
    let buf_ptr: u64 = unsafe { ctx.read_at(ENTER_BUF_OFF)? };
    if buf_ptr == 0 {
        return Ok(());
    }
    let pending = PendingIo {
        buf_ptr,
        fd: fd as i32,
        dir: dir as u8,
        _pad: [0; 3],
    };
    PENDING_IO.insert(&tid(), &pending, 0)?;
    Ok(())
}

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
    let fd: u64 = unsafe { ctx.read_at(ENTER_FD_OFF)? };
    mark_sock_fd(fd as i64);

    let sockaddr: *const SockAddrIn = unsafe { ctx.read_at(ENTER_SOCKADDR_OFF)? };
    let (daddr_be, dport_be) = match read_sockaddr_v4(sockaddr) {
        Ok(v) => v,
        Err(_) => return Ok(()), // non-IPv4: ignore (P1 Q4)
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
    match try_exit_accept4(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_exit_accept4(ctx: &TracePointContext) -> Result<(), i64> {
    let tid = tid();
    let Some(pending) = (unsafe { PENDING.get(&tid) }) else {
        return Ok(());
    };
    let pending = *pending;
    let _ = PENDING.remove(&tid);

    let ret: i64 = unsafe { ctx.read_at(EXIT_RET_OFF)? };
    mark_sock_fd(ret);

    let now = unsafe { bpf_ktime_get_ns() };
    let latency_ns = now.saturating_sub(pending.ts_ns);
    let (daddr_be, dport_be) = peer_addr(&pending, ret);
    emit(EventKind::Accept, ret, latency_ns, now, daddr_be, dport_be);
    Ok(())
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
    let (daddr_be, dport_be) = peer_addr(&pending, ret);
    emit(kind, ret, latency_ns, now, daddr_be, dport_be);
    Ok(())
}

fn peer_addr(pending: &PendingEnter, ret: i64) -> (u32, u16) {
    if pending.has_addr == 1 {
        (pending.daddr_be, pending.dport_be)
    } else if pending.sockaddr_ptr != 0 && ret >= 0 {
        match read_sockaddr_v4(pending.sockaddr_ptr as *const SockAddrIn) {
            Ok(v) => v,
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    }
}

// --- close (Q4 fd table hygiene) ---

#[tracepoint]
pub fn enter_close(ctx: TracePointContext) -> u32 {
    match try_enter_close(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_enter_close(ctx: &TracePointContext) -> Result<(), i64> {
    let fd: u64 = unsafe { ctx.read_at(ENTER_FD_OFF)? };
    unmark_sock_fd(fd as u32);
    Ok(())
}

// --- read / write (Phase 2 Q2/Q3) ---

#[tracepoint]
pub fn enter_read(ctx: TracePointContext) -> u32 {
    match try_enter_io(&ctx, IoDir::Read) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn exit_read(ctx: TracePointContext) -> u32 {
    match try_exit_io(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn enter_write(ctx: TracePointContext) -> u32 {
    match try_enter_io(&ctx, IoDir::Write) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn exit_write(ctx: TracePointContext) -> u32 {
    match try_exit_io(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn enter_recvfrom(ctx: TracePointContext) -> u32 {
    match try_enter_io(&ctx, IoDir::Read) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn exit_recvfrom(ctx: TracePointContext) -> u32 {
    match try_exit_io(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn enter_sendto(ctx: TracePointContext) -> u32 {
    match try_enter_io(&ctx, IoDir::Write) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn exit_sendto(ctx: TracePointContext) -> u32 {
    match try_exit_io(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_exit_io(ctx: &TracePointContext) -> Result<(), i64> {
    let tid = tid();
    let Some(pending) = (unsafe { PENDING_IO.get(&tid) }) else {
        return Ok(());
    };
    let pending = *pending;
    let _ = PENDING_IO.remove(&tid);

    let ret: i64 = unsafe { ctx.read_at(EXIT_RET_OFF)? };
    emit_io(&pending, ret);
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
