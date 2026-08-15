#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{
        bpf_get_current_pid_tgid, bpf_ktime_get_ns, bpf_probe_read_user,
        bpf_probe_read_user_buf,
    },
    macros::{kprobe, map, tracepoint, uprobe, uretprobe},
    maps::{Array, HashMap, PerCpuArray, RingBuf},
    programs::{ProbeContext, RetProbeContext, TracePointContext},
};
use aya_log_ebpf::info;
use obsagent_common::{
    AF_INET, EVENTS_RINGBUF_BYTES, EventKind, IoDir, PENDING_MAP_ENTRIES, PendingEnter,
    PendingIo, PendingTls, SOCK_IO_PREFIX_LEN, SockIoEvent, SockLatencyEvent, SockMeta,
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

#[map]
static PENDING_TLS: HashMap<u32, PendingTls> =
    HashMap::<u32, PendingTls>::with_max_entries(PENDING_MAP_ENTRIES, 0);

/// Separate from `PENDING_TLS` so classic `SSL_read`/`SSL_write` and `_ex`
/// on the same tid cannot clobber each other.
#[map]
static PENDING_TLS_EX: HashMap<u32, PendingTls> =
    HashMap::<u32, PendingTls>::with_max_entries(PENDING_MAP_ENTRIES, 0);

/// Phase 4 Q1: fds observed via connect/accept with peer metadata.
/// Key = (tgid, fd). Replaces presence-only `SOCK_FDS`.
#[map]
static SOCK_META: HashMap<u64, SockMeta> =
    HashMap::<u64, SockMeta>::with_max_entries(PENDING_MAP_ENTRIES, 0);

/// Phase 3 Q1: SSL* → fd (from SSL_set_fd / rfd / wfd).
#[map]
static SSL_FD: HashMap<u64, i32> =
    HashMap::<u64, i32>::with_max_entries(PENDING_MAP_ENTRIES, 0);

/// Reverse of `SSL_FD` for close/`SSL_free` cleanup. Key = (tgid, fd) → ssl*.
#[map]
static FD_SSL: HashMap<u64, u64> =
    HashMap::<u64, u64>::with_max_entries(PENDING_MAP_ENTRIES, 0);

/// Phase 3 Q8: fds known to be TLS — skip Phase 2 sock I/O. Key = (tgid, fd).
#[map]
static TLS_FDS: HashMap<u64, u8> =
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

fn mark_sock_meta(fd: i64, daddr_be: u32, dport_be: u16) {
    if fd < 0 || fd > u32::MAX as i64 {
        return;
    }
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd as u32);
    let meta = SockMeta::with_peer(daddr_be, dport_be);
    let _ = SOCK_META.insert(&key, &meta, 0);
}

fn unmark_sock_fd(fd: u32) {
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd);
    let _ = SOCK_META.remove(&key);
    let _ = TLS_FDS.remove(&key);
    clear_ssl_for_fd_key(key, fd as i32);
}

fn clear_ssl_for_fd_key(key: u64, fd: i32) {
    let Some(ssl) = (unsafe { FD_SSL.get(&key) }) else {
        return;
    };
    let ssl = *ssl;
    let _ = FD_SSL.remove(&key);
    if let Some(mapped) = unsafe { SSL_FD.get(&ssl) } {
        if *mapped == fd {
            let _ = SSL_FD.remove(&ssl);
        }
    }
}

fn clear_ssl_ptr(ssl: u64) {
    if ssl == 0 {
        return;
    }
    let Some(fd) = (unsafe { SSL_FD.get(&ssl) }) else {
        return;
    };
    let fd = *fd;
    let _ = SSL_FD.remove(&ssl);
    if fd < 0 {
        return;
    }
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd as u32);
    if let Some(mapped) = unsafe { FD_SSL.get(&key) } {
        if *mapped == ssl {
            let _ = FD_SSL.remove(&key);
            let _ = TLS_FDS.remove(&key);
        }
    }
}

fn is_marked_sock_fd(fd: u32) -> bool {
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd);
    unsafe { SOCK_META.get(&key).is_some() }
}

fn is_tls_fd(fd: u32) -> bool {
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd);
    unsafe { TLS_FDS.get(&key).is_some() }
}

fn mark_tls_fd(fd: i32) {
    if fd < 0 {
        return;
    }
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd as u32);
    let one: u8 = 1;
    let _ = TLS_FDS.insert(&key, &one, 0);
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
    emit_io_kind(EventKind::SockIo, pending.buf_ptr, pending.fd, pending.dir, ret);
}

fn emit_io_kind(kind: EventKind, buf_ptr: u64, fd: i32, dir: u8, ret: i64) {
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
    ev.kind = kind as u8;
    ev.dir = dir;
    ev.prefix_len = prefix_len;
    ev.fd = fd;
    ev.pid = pid;
    ev.tgid = tgid;
    ev.ret = ret;
    ev.ts_ns = now;
    ev.prefix = [0; SOCK_IO_PREFIX_LEN];

    // Copy at most prefix_len bytes. Verifier needs a fixed dest size; we use the
    // full array but only treat prefix_len as valid (userspace caps too).
    if prefix_len > 0 {
        let _ = unsafe { bpf_probe_read_user_buf(buf_ptr as *const u8, &mut ev.prefix) };
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
    // Q8: TLS-marked fds skip Phase 2 sock I/O (ciphertext).
    if is_tls_fd(fd) {
        return Ok(());
    }
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

    let sockaddr: *const SockAddrIn = unsafe { ctx.read_at(ENTER_SOCKADDR_OFF)? };
    let (daddr_be, dport_be) = match read_sockaddr_v4(sockaddr) {
        Ok(v) => v,
        Err(_) => return Ok(()), // non-IPv4: ignore (P1 Q4)
    };
    // Phase 4 Q1: store peer with the fd mark (was presence-only SOCK_FDS).
    mark_sock_meta(fd as i64, daddr_be, dport_be);
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
    let now = unsafe { bpf_ktime_get_ns() };
    let latency_ns = now.saturating_sub(pending.ts_ns);
    let (daddr_be, dport_be) = peer_addr(&pending, ret);
    mark_sock_meta(ret, daddr_be, dport_be);
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

// --- Phase 3: OpenSSL uprobes (Q1/Q7) ---

#[uprobe]
pub fn enter_ssl_set_fd(ctx: ProbeContext) -> u32 {
    match try_enter_ssl_set_fd(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_enter_ssl_set_fd(ctx: &ProbeContext) -> Result<(), u32> {
    let ssl: u64 = ctx.arg(0).ok_or(1u32)?;
    let fd: i32 = ctx.arg(1).ok_or(1u32)?;
    if ssl == 0 || fd < 0 {
        return Ok(());
    }
    // Drop stale reverse map if this SSL* was rebound to another fd.
    if let Some(old_fd) = unsafe { SSL_FD.get(&ssl) } {
        let old_fd = *old_fd;
        if old_fd >= 0 && old_fd != fd {
            let (tgid, _) = pid_tgid();
            let old_key = sock_fd_key(tgid, old_fd as u32);
            if let Some(mapped) = unsafe { FD_SSL.get(&old_key) } {
                if *mapped == ssl {
                    let _ = FD_SSL.remove(&old_key);
                    let _ = TLS_FDS.remove(&old_key);
                }
            }
        }
    }
    let _ = SSL_FD.insert(&ssl, &fd, 0);
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd as u32);
    let _ = FD_SSL.insert(&key, &ssl, 0);
    mark_tls_fd(fd);
    Ok(())
}

#[uprobe]
pub fn enter_ssl_free(ctx: ProbeContext) -> u32 {
    match try_enter_ssl_free(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_enter_ssl_free(ctx: &ProbeContext) -> Result<(), u32> {
    let ssl: u64 = ctx.arg(0).ok_or(1u32)?;
    clear_ssl_ptr(ssl);
    Ok(())
}

#[uprobe]
pub fn enter_ssl_write(ctx: ProbeContext) -> u32 {
    match try_enter_ssl_io(&ctx, IoDir::Write) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[uretprobe]
pub fn exit_ssl_write(ctx: RetProbeContext) -> u32 {
    match try_exit_ssl_io(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[uprobe]
pub fn enter_ssl_read(ctx: ProbeContext) -> u32 {
    match try_enter_ssl_io(&ctx, IoDir::Read) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[uretprobe]
pub fn exit_ssl_read(ctx: RetProbeContext) -> u32 {
    match try_exit_ssl_io(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_enter_ssl_io(ctx: &ProbeContext, dir: IoDir) -> Result<(), u32> {
    let ssl: u64 = ctx.arg(0).ok_or(1u32)?;
    let buf: u64 = ctx.arg(1).ok_or(1u32)?;
    if ssl == 0 || buf == 0 {
        return Ok(());
    }
    // Q1: require SSL_set_fd mapping before we bother stashing.
    if unsafe { SSL_FD.get(&ssl) }.is_none() {
        return Ok(());
    }
    let pending = PendingTls {
        buf_ptr: buf,
        ssl_ptr: ssl,
        outlen_ptr: 0,
        dir: dir as u8,
        _pad: [0; 7],
    };
    PENDING_TLS.insert(&tid(), &pending, 0).map_err(|_| 1u32)?;
    Ok(())
}

/// `SSL_write_ex` / `SSL_read_ex`: byte count lives in `*arg3` on success (ret==1).
fn try_enter_ssl_io_ex(ctx: &ProbeContext, dir: IoDir) -> Result<(), u32> {
    let ssl: u64 = ctx.arg(0).ok_or(1u32)?;
    let buf: u64 = ctx.arg(1).ok_or(1u32)?;
    let outlen: u64 = ctx.arg(3).ok_or(1u32)?;
    if ssl == 0 || buf == 0 || outlen == 0 {
        return Ok(());
    }
    if unsafe { SSL_FD.get(&ssl) }.is_none() {
        return Ok(());
    }
    let pending = PendingTls {
        buf_ptr: buf,
        ssl_ptr: ssl,
        outlen_ptr: outlen,
        dir: dir as u8,
        _pad: [0; 7],
    };
    PENDING_TLS_EX.insert(&tid(), &pending, 0).map_err(|_| 1u32)?;
    Ok(())
}

fn try_exit_ssl_io(ctx: &RetProbeContext) -> Result<(), u32> {
    try_exit_ssl_pending(ctx, false)
}

fn try_exit_ssl_io_ex(ctx: &RetProbeContext) -> Result<(), u32> {
    try_exit_ssl_pending(ctx, true)
}

fn try_exit_ssl_pending(ctx: &RetProbeContext, is_ex: bool) -> Result<(), u32> {
    let tid = tid();
    let pending = if is_ex {
        let Some(pending) = (unsafe { PENDING_TLS_EX.get(&tid) }) else {
            return Ok(());
        };
        let pending = *pending;
        let _ = PENDING_TLS_EX.remove(&tid);
        pending
    } else {
        let Some(pending) = (unsafe { PENDING_TLS.get(&tid) }) else {
            return Ok(());
        };
        let pending = *pending;
        let _ = PENDING_TLS.remove(&tid);
        pending
    };

    let Some(fd) = (unsafe { SSL_FD.get(&pending.ssl_ptr) }) else {
        // Q1: drop if fd unknown.
        return Ok(());
    };
    let fd = *fd;

    let ret: i64 = if pending.outlen_ptr == 0 {
        ctx.ret()
    } else {
        let ok: i32 = ctx.ret();
        if ok != 1 {
            0
        } else {
            match unsafe { bpf_probe_read_user(pending.outlen_ptr as *const u64) } {
                Ok(n) => n as i64,
                Err(_) => 0,
            }
        }
    };

    emit_io_kind(
        EventKind::TlsIo,
        pending.buf_ptr,
        fd,
        pending.dir,
        ret,
    );
    Ok(())
}

#[uprobe]
pub fn enter_ssl_write_ex(ctx: ProbeContext) -> u32 {
    match try_enter_ssl_io_ex(&ctx, IoDir::Write) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[uretprobe]
pub fn exit_ssl_write_ex(ctx: RetProbeContext) -> u32 {
    match try_exit_ssl_io_ex(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[uprobe]
pub fn enter_ssl_read_ex(ctx: ProbeContext) -> u32 {
    match try_enter_ssl_io_ex(&ctx, IoDir::Read) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[uretprobe]
pub fn exit_ssl_read_ex(ctx: RetProbeContext) -> u32 {
    match try_exit_ssl_io_ex(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
