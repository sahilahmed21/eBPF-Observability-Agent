#![no_std]
#![no_main]

use aya_ebpf::{
    EbpfContext,
    helpers::{
        bpf_get_current_cgroup_id, bpf_get_current_pid_tgid, bpf_get_prandom_u32, bpf_ktime_get_ns,
        bpf_probe_read_user, bpf_probe_read_user_buf,
    },
    macros::{kprobe, map, perf_event, tracepoint, uprobe, uretprobe},
    maps::{Array, HashMap, PerCpuArray, RingBuf},
    programs::{PerfEventContext, ProbeContext, RetProbeContext, TracePointContext},
};
use aya_ebpf::bindings::BPF_F_USER_STACK;
use aya_ebpf::helpers::bpf_get_stack;
use aya_log_ebpf::info;
use obsagent_common::{
    AF_INET, AF_INET6, EVENTS_RINGBUF_BYTES, EventKind, IO_BUF_LEN_UNBOUNDED, IoDir,
    PENDING_MAP_ENTRIES, PendingEnter, PendingIo, PendingTls, SOCK_IO_PREFIX_LEN, SockIoEvent,
    SockIoTimesEvent, SockLatencyEvent, SockMeta, SockMetaKeep, StackSampleEvent,
    STACKS_RINGBUF_BYTES, STACK_SAMPLE_MAX_FRAMES, TlsHandshakeEvent,
    DENIED_TGID_ENTRIES, ALLOWED_TGID_ENTRIES, http_magic, io_copy_len,
};

// Tracepoint field offsets from this kernel's format files (WSL2 6.6).
const ENTER_FD_OFF: usize = 16;
const ENTER_SOCKADDR_OFF: usize = 24;
const ENTER_BUF_OFF: usize = 24;
const EXIT_RET_OFF: usize = 16;
// writev/readv: arg1 is `struct iovec *` at ENTER_BUF_OFF.
// sendmsg/recvmsg: arg1 is `struct user_msghdr *` at ENTER_BUF_OFF.
// linux/socket.h user_msghdr: msg_iov at offset 16 on x86_64 (msg_name + msg_namelen + pad).

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

/// HTTP-magic emit sets this; continuation prefixes skip magic. Cleared on
/// close and by userspace after a header half-flush.
#[map]
static INFLIGHT: HashMap<u64, u8> =
    HashMap::<u64, u8>::with_max_entries(PENDING_MAP_ENTRIES, 0);

#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(EVENTS_RINGBUF_BYTES, 0);

/// Phase 12: perf_event stack samples (separate from HTTP `EVENTS`).
#[map]
static STACKS: RingBuf = RingBuf::with_byte_size(STACKS_RINGBUF_BYTES, 0);

#[map]
static DROPS: Array<u64> = Array::with_max_entries(1, 0);

#[map]
static STACK_DROPS: Array<u64> = Array::with_max_entries(1, 0);

/// I/O keep denominator. Userspace writes 1,2,4,8,16 or a pin. `n <= 1` keep all.
#[map]
static SAMPLE_N: Array<u32> = Array::with_max_entries(1, 0);

/// Processes that must never be captured (deny-list mode). Presence = skip.
#[map]
static DENIED_TGID: HashMap<u32, u8> =
    HashMap::<u32, u8>::with_max_entries(DENIED_TGID_ENTRIES, 0);

/// Exclusive allow-list mode: capture only if present. Used when ALLOW_ONLY[0] != 0.
#[map]
static ALLOWED_TGID: HashMap<u32, u8> =
    HashMap::<u32, u8>::with_max_entries(ALLOWED_TGID_ENTRIES, 0);

/// 0 = deny-list (`DENIED_TGID`). Non-zero = allow-only (`ALLOWED_TGID`).
#[map]
static ALLOW_ONLY: Array<u32> = Array::with_max_entries(1, 0);

/// SSL* seen without `SSL_FD` (custom BIO residual).
#[map]
static TLS_UNMAPPED: Array<u64> = Array::with_max_entries(1, 0);

/// SSL* already counted in `tls_unmapped` (once per object, not per SSL_read).
#[map]
static UNMAPPED_SEEN: HashMap<u64, u8> =
    HashMap::<u64, u8>::with_max_entries(PENDING_MAP_ENTRIES, 0);

/// First `SSL_do_handshake` enter ts keyed by SSL*.
#[map]
static HANDSHAKE_START: HashMap<u64, u64> =
    HashMap::<u64, u64>::with_max_entries(PENDING_MAP_ENTRIES, 0);

/// tid → SSL* so uretprobe can recover the pointer.
#[map]
static PENDING_HS: HashMap<u32, u64> =
    HashMap::<u32, u64>::with_max_entries(PENDING_MAP_ENTRIES, 0);

/// Scratch for SockIoEvent (296B) — avoid BPF stack overflow.
#[map]
static IO_SCRATCH: PerCpuArray<SockIoEvent> = PerCpuArray::with_max_entries(1, 0);

#[repr(C)]
struct SockAddrIn {
    sin_family: u16,
    sin_port: u16,
    sin_addr: u32,
}

/// Userspace `sockaddr_in6` (uapi linux/in6.h). Do not use kernel `msghdr`.
#[repr(C)]
struct SockAddrIn6 {
    sin6_family: u16,
    sin6_port: u16,
    sin6_flowinfo: u32,
    sin6_addr: [u8; 16],
    sin6_scope_id: u32,
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

fn mark_sock_meta(fd: i64, mut meta: SockMeta) {
    if fd < 0 || fd > u32::MAX as i64 {
        return;
    }
    let (tgid, _) = pid_tgid();
    if !tgid_captured(tgid) {
        return;
    }
    let key = sock_fd_key(tgid, fd as u32);
    let old = unsafe { SOCK_META.get(&key) }.copied();
    meta = meta.merge_peer_preserve_sample(old);
    if old.and_then(SockMeta::io_sampled_keep).is_none() {
        apply_keep_io_flags(&mut meta);
    }
    let _ = SOCK_META.insert(&key, &meta, 0);
}

fn unmark_sock_fd(fd: u32) {
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd);
    let _ = SOCK_META.remove(&key);
    let _ = TLS_FDS.remove(&key);
    let _ = INFLIGHT.remove(&key);
    clear_ssl_for_fd_key(key, fd as i32);
}

fn clear_ssl_for_fd_key(key: u64, fd: i32) {
    let Some(ssl) = (unsafe { FD_SSL.get(&key) }) else {
        return;
    };
    let ssl = *ssl;
    let _ = FD_SSL.remove(&key);
    let _ = HANDSHAKE_START.remove(&ssl);
    let _ = UNMAPPED_SEEN.remove(&ssl);
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
    let _ = HANDSHAKE_START.remove(&ssl);
    let _ = UNMAPPED_SEEN.remove(&ssl);
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

fn is_inflight(fd: u32) -> bool {
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd);
    unsafe { INFLIGHT.get(&key).is_some() }
}

fn mark_inflight(fd: i32) {
    if fd < 0 {
        return;
    }
    let (tgid, _) = pid_tgid();
    let key = sock_fd_key(tgid, fd as u32);
    let one: u8 = 1;
    let _ = INFLIGHT.insert(&key, &one, 0);
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

fn bump_stack_drop() {
    if let Some(ptr) = STACK_DROPS.get_ptr_mut(0) {
        unsafe {
            *ptr += 1;
        }
    }
}

fn is_denied(tgid: u32) -> bool {
    unsafe { DENIED_TGID.get(&tgid).is_some() }
}

fn allow_only_mode() -> bool {
    matches!(ALLOW_ONLY.get(0), Some(n) if *n != 0)
}

/// Whether this tgid may emit at all (deny-list vs allow-only).
fn tgid_captured(tgid: u32) -> bool {
    if allow_only_mode() {
        unsafe { ALLOWED_TGID.get(&tgid).is_some() }
    } else {
        !is_denied(tgid)
    }
}

fn read_sample_n() -> u32 {
    match SAMPLE_N.get(0) {
        Some(n) => *n,
        None => 1,
    }
}

fn draw_keep_io() -> bool {
    let n = read_sample_n();
    if n <= 1 {
        return true;
    }
    (unsafe { bpf_get_prandom_u32() }) % n == 0
}

fn apply_keep_io_flags(meta: &mut SockMeta) {
    meta.set_io_keep(draw_keep_io());
}

/// Sticky I/O keep for a **marked** fd. Does not create `SOCK_META`.
fn existing_fd_keep_io(fd: u32) -> bool {
    let (tgid, _) = pid_tgid();
    if !tgid_captured(tgid) {
        return false;
    }
    let key = sock_fd_key(tgid, fd);
    let meta = unsafe { SOCK_META.get(&key) }.copied();
    match SockMeta::keep_from_lookup(meta) {
        SockMetaKeep::Unmarked => false,
        SockMetaKeep::Decided(keep) => keep,
        SockMetaKeep::Undecided => {
            let Some(mut m) = meta else {
                return false;
            };
            apply_keep_io_flags(&mut m);
            let keep = m.flags & SockMeta::FLAG_KEEP_IO != 0;
            // BPF_EXIST: never create a SOCK_META that connect/accept did not mark.
            let _ = SOCK_META.insert(&key, &m, 2);
            keep
        }
    }
}

fn bump_unmapped() {
    if let Some(ptr) = TLS_UNMAPPED.get_ptr_mut(0) {
        unsafe {
            *ptr += 1;
        }
    }
}

fn bump_unmapped_ssl(ssl: u64) {
    if ssl == 0 {
        return;
    }
    if unsafe { UNMAPPED_SEEN.get(&ssl) }.is_some() {
        return;
    }
    let one: u8 = 1;
    if UNMAPPED_SEEN.insert(&ssl, &one, 0).is_err() {
        return;
    }
    bump_unmapped();
}

fn read_sockaddr(ptr: u64) -> Result<SockMeta, i64> {
    let fam: u16 = unsafe { bpf_probe_read_user(ptr as *const u16)? };
    if fam == AF_INET {
        let sa: SockAddrIn = unsafe { bpf_probe_read_user(ptr as *const SockAddrIn)? };
        Ok(SockMeta::with_peer_v4(sa.sin_addr, sa.sin_port))
    } else if fam == AF_INET6 {
        let sa: SockAddrIn6 = unsafe { bpf_probe_read_user(ptr as *const SockAddrIn6)? };
        Ok(SockMeta::with_peer_v6(sa.sin6_addr, sa.sin6_port))
    } else {
        Err(1)
    }
}

fn emit(kind: EventKind, ret: i64, latency_ns: u64, ts_ns: u64, daddr_be: u32, dport_be: u16) {
    let (tgid, pid) = pid_tgid();
    if !tgid_captured(tgid) {
        return;
    }
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
    http_magic(prefix, len)
}

fn emit_io(pending: &PendingIo, ret: i64) {
    emit_io_kind(EventKind::SockIo, pending, ret);
}

fn emit_io_kind(kind: EventKind, pending: &PendingIo, ret: i64) {
    let now = unsafe { bpf_ktime_get_ns() };
    let (tgid, pid) = pid_tgid();
    let buf_ptr = pending.buf_ptr;
    let buf_len = pending.buf_len;
    let fd = pending.fd;
    let dir = pending.dir;

    if fd < 0 || !existing_fd_keep_io(fd as u32) {
        return;
    }

    let prefix_len = io_copy_len(ret, buf_len);
    if prefix_len == 0 {
        return;
    }

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
    ev.cgroup_id = unsafe { bpf_get_current_cgroup_id() };
    ev.prefix = [0; SOCK_IO_PREFIX_LEN];

    if unsafe { bpf_probe_read_user_buf(buf_ptr as *const u8, &mut ev.prefix) }.is_err() {
        return;
    }
    // Verifier needs a fixed dest size for the probe-read. Zero the tail so
    // adjacent userspace bytes never leave the scratch slot.
    let n = prefix_len as usize;
    let mut i = 0usize;
    while i < SOCK_IO_PREFIX_LEN {
        if i >= n {
            ev.prefix[i] = 0;
        }
        i += 1;
    }

    let magic = looks_like_http_fixed(&ev.prefix, prefix_len);
    if !magic && !is_inflight(fd as u32) {
        return;
    }

    let Some(mut slot) = EVENTS.reserve::<SockIoEvent>(0) else {
        bump_drop();
        return;
    };
    slot.write(*ev);
    slot.submit(0);
    if magic {
        mark_inflight(fd);
    }
}

fn try_enter_io(ctx: &TracePointContext, dir: IoDir) -> Result<(), i64> {
    let fd: u64 = unsafe { ctx.read_at(ENTER_FD_OFF)? };
    let buf_ptr: u64 = unsafe { ctx.read_at(ENTER_BUF_OFF)? };
    stash_pending_io(fd as u32, buf_ptr, IO_BUF_LEN_UNBOUNDED, dir)
}

#[repr(C)]
struct IoVec {
    base: u64,
    len: u64,
}

fn stash_pending_io(fd: u32, buf_ptr: u64, buf_len: u32, dir: IoDir) -> Result<(), i64> {
    let (tgid, _) = pid_tgid();
    if !tgid_captured(tgid) {
        return Ok(());
    }
    if is_tls_fd(fd) {
        if !existing_fd_keep_io(fd) {
            return Ok(());
        }
        let pending = PendingIo {
            buf_ptr: 0,
            buf_len: 0,
            fd: fd as i32,
            dir: dir as u8,
            timing_only: 1,
            _pad: [0; 2],
        };
        PENDING_IO.insert(&tid(), &pending, 0)?;
        return Ok(());
    }
    if !is_marked_sock_fd(fd) || buf_ptr == 0 {
        return Ok(());
    }
    if !existing_fd_keep_io(fd) {
        return Ok(());
    }
    let pending = PendingIo {
        buf_ptr,
        buf_len,
        fd: fd as i32,
        dir: dir as u8,
        timing_only: 0,
        _pad: [0; 2],
    };
    PENDING_IO.insert(&tid(), &pending, 0)?;
    Ok(())
}

fn iov_len_u32(len: u64) -> u32 {
    if len > u32::MAX as u64 {
        u32::MAX
    } else {
        len as u32
    }
}

fn try_enter_iov(ctx: &TracePointContext, dir: IoDir) -> Result<(), i64> {
    let fd: u64 = unsafe { ctx.read_at(ENTER_FD_OFF)? };
    let iov_ptr: u64 = unsafe { ctx.read_at(ENTER_BUF_OFF)? };
    if iov_ptr == 0 {
        return Ok(());
    }
    let iov: IoVec = unsafe { bpf_probe_read_user(iov_ptr as *const IoVec)? };
    stash_pending_io(fd as u32, iov.base, iov_len_u32(iov.len), dir)
}

#[repr(C)]
struct UserMsgHdrIov {
    _name: u64,
    _namelen: u32,
    _pad: u32,
    msg_iov: u64,
}

fn try_enter_msghdr(ctx: &TracePointContext, dir: IoDir) -> Result<(), i64> {
    let fd: u64 = unsafe { ctx.read_at(ENTER_FD_OFF)? };
    let msg: u64 = unsafe { ctx.read_at(ENTER_BUF_OFF)? };
    if msg == 0 {
        return Ok(());
    }
    let hdr: UserMsgHdrIov = unsafe { bpf_probe_read_user(msg as *const UserMsgHdrIov)? };
    if hdr.msg_iov == 0 {
        return Ok(());
    }
    let iov: IoVec = unsafe { bpf_probe_read_user(hdr.msg_iov as *const IoVec)? };
    stash_pending_io(fd as u32, iov.base, iov_len_u32(iov.len), dir)
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
    let (tgid, _) = pid_tgid();
    if !tgid_captured(tgid) {
        return Ok(());
    }
    let fd: u64 = unsafe { ctx.read_at(ENTER_FD_OFF)? };

    let sockaddr: u64 = unsafe { ctx.read_at(ENTER_SOCKADDR_OFF)? };
    let meta = match read_sockaddr(sockaddr) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    mark_sock_meta(fd as i64, meta);
    let pending = PendingEnter {
        ts_ns: unsafe { bpf_ktime_get_ns() },
        daddr_be: meta.v4_addr().unwrap_or(0),
        dport_be: meta.dport_be,
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
    let (tgid, _) = pid_tgid();
    if !tgid_captured(tgid) {
        return Ok(());
    }
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
    let (daddr_be, dport_be, meta) = peer_addr(&pending, ret);
    mark_sock_meta(ret, meta);
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
    let (daddr_be, dport_be, _) = peer_addr(&pending, ret);
    emit(kind, ret, latency_ns, now, daddr_be, dport_be);
    Ok(())
}

fn peer_addr(pending: &PendingEnter, ret: i64) -> (u32, u16, SockMeta) {
    if pending.has_addr == 1 {
        let meta = SockMeta::with_peer_v4(pending.daddr_be, pending.dport_be);
        (pending.daddr_be, pending.dport_be, meta)
    } else if pending.sockaddr_ptr != 0 && ret >= 0 {
        match read_sockaddr(pending.sockaddr_ptr) {
            Ok(meta) => (
                meta.v4_addr().unwrap_or(0),
                meta.dport_be,
                meta,
            ),
            Err(_) => (0, 0, SockMeta::with_peer_v4(0, 0)),
        }
    } else {
        (0, 0, SockMeta::with_peer_v4(0, 0))
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

#[tracepoint]
pub fn enter_readv(ctx: TracePointContext) -> u32 {
    match try_enter_iov(&ctx, IoDir::Read) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn exit_readv(ctx: TracePointContext) -> u32 {
    match try_exit_io(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn enter_writev(ctx: TracePointContext) -> u32 {
    match try_enter_iov(&ctx, IoDir::Write) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn exit_writev(ctx: TracePointContext) -> u32 {
    match try_exit_io(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn enter_recvmsg(ctx: TracePointContext) -> u32 {
    match try_enter_msghdr(&ctx, IoDir::Read) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn exit_recvmsg(ctx: TracePointContext) -> u32 {
    match try_exit_io(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn enter_sendmsg(ctx: TracePointContext) -> u32 {
    match try_enter_msghdr(&ctx, IoDir::Write) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn exit_sendmsg(ctx: TracePointContext) -> u32 {
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
    if pending.timing_only != 0 {
        emit_times(&pending, ret);
    } else {
        emit_io(&pending, ret);
    }
    Ok(())
}

fn emit_times(pending: &PendingIo, ret: i64) {
    if ret <= 0 {
        return;
    }
    if pending.fd < 0 || !existing_fd_keep_io(pending.fd as u32) {
        return;
    }
    let now = unsafe { bpf_ktime_get_ns() };
    let (tgid, pid) = pid_tgid();
    let Some(mut slot) = EVENTS.reserve::<SockIoTimesEvent>(0) else {
        bump_drop();
        return;
    };
    slot.write(SockIoTimesEvent {
        kind: EventKind::SockIoTimes as u8,
        dir: pending.dir,
        _pad: 0,
        fd: pending.fd,
        pid,
        tgid,
        ret,
        ts_ns: now,
    });
    slot.submit(0);
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
    let fd = match unsafe { SSL_FD.get(&ssl) } {
        Some(fd) if *fd >= 0 => *fd,
        _ => {
            bump_unmapped_ssl(ssl);
            return Ok(());
        }
    };
    if !existing_fd_keep_io(fd as u32) {
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
    let fd = match unsafe { SSL_FD.get(&ssl) } {
        Some(fd) if *fd >= 0 => *fd,
        _ => {
            bump_unmapped_ssl(ssl);
            return Ok(());
        }
    };
    if !existing_fd_keep_io(fd as u32) {
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

    let io = PendingIo {
        buf_ptr: pending.buf_ptr,
        buf_len: IO_BUF_LEN_UNBOUNDED,
        fd,
        dir: pending.dir,
        timing_only: 0,
        _pad: [0; 2],
    };
    emit_io_kind(EventKind::TlsIo, &io, ret);
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

#[uprobe]
pub fn enter_ssl_do_handshake(ctx: ProbeContext) -> u32 {
    match try_enter_ssl_do_handshake(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[uretprobe]
pub fn exit_ssl_do_handshake(ctx: RetProbeContext) -> u32 {
    match try_exit_ssl_do_handshake(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_enter_ssl_do_handshake(ctx: &ProbeContext) -> Result<(), u32> {
    let (tgid, _) = pid_tgid();
    if !tgid_captured(tgid) {
        return Ok(());
    }
    let ssl: u64 = ctx.arg(0).ok_or(1u32)?;
    if ssl == 0 {
        return Ok(());
    }
    let _ = PENDING_HS.insert(&tid(), &ssl, 0);
    if unsafe { HANDSHAKE_START.get(&ssl) }.is_none() {
        let ts = unsafe { bpf_ktime_get_ns() };
        let _ = HANDSHAKE_START.insert(&ssl, &ts, 0);
    }
    Ok(())
}

fn try_exit_ssl_do_handshake(ctx: &RetProbeContext) -> Result<(), u32> {
    let tid = tid();
    let Some(ssl) = (unsafe { PENDING_HS.get(&tid) }) else {
        return Ok(());
    };
    let ssl = *ssl;
    let _ = PENDING_HS.remove(&tid);

    let ret: i32 = ctx.ret();
    if ret != 1 {
        return Ok(());
    }
    let Some(start) = (unsafe { HANDSHAKE_START.get(&ssl) }) else {
        return Ok(());
    };
    let start = *start;
    let _ = HANDSHAKE_START.remove(&ssl);

    let fd = match unsafe { SSL_FD.get(&ssl) } {
        Some(fd) => *fd,
        None => {
            bump_unmapped_ssl(ssl);
            return Ok(());
        }
    };

    let now = unsafe { bpf_ktime_get_ns() };
    let (tgid, pid) = pid_tgid();
    if !tgid_captured(tgid) {
        return Ok(());
    }
    let Some(mut slot) = EVENTS.reserve::<TlsHandshakeEvent>(0) else {
        bump_drop();
        return Ok(());
    };
    slot.write(TlsHandshakeEvent {
        kind: EventKind::TlsHandshake as u8,
        _pad0: [0; 7],
        pid,
        tgid,
        fd,
        _pad1: 0,
        ret: ret as i64,
        latency_ns: now.saturating_sub(start),
        ts_ns: now,
    });
    slot.submit(0);
    Ok(())
}

// --- Phase 12: perf_event stack samples (Q16) ---

const STACK_BUF_BYTES: u32 = (STACK_SAMPLE_MAX_FRAMES * core::mem::size_of::<u64>()) as u32;

#[perf_event]
pub fn profile_sample(ctx: PerfEventContext) -> u32 {
    match try_profile_sample(&ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_profile_sample(ctx: &PerfEventContext) -> Result<(), i32> {
    let (tgid, pid) = pid_tgid();
    let ts_ns = unsafe { bpf_ktime_get_ns() };
    let Some(mut slot) = STACKS.reserve::<StackSampleEvent>(0) else {
        bump_stack_drop();
        return Ok(());
    };
    let mut stack = [0u8; STACK_BUF_BYTES as usize];
    let len = unsafe {
        bpf_get_stack(
            ctx.as_ptr(),
            stack.as_mut_ptr() as *mut core::ffi::c_void,
            STACK_BUF_BYTES,
            BPF_F_USER_STACK as u64,
        )
    };
    if len <= 0 {
        slot.discard(0);
        return Ok(());
    }
    let n = (len as usize / core::mem::size_of::<u64>()).min(STACK_SAMPLE_MAX_FRAMES);
    let mut ips = [0u64; STACK_SAMPLE_MAX_FRAMES];
    let mut i = 0usize;
    while i < n {
        let off = i * core::mem::size_of::<u64>();
        ips[i] = u64::from_ne_bytes([
            stack[off],
            stack[off + 1],
            stack[off + 2],
            stack[off + 3],
            stack[off + 4],
            stack[off + 5],
            stack[off + 6],
            stack[off + 7],
        ]);
        i += 1;
    }
    slot.write(StackSampleEvent {
        kind: EventKind::StackSample as u8,
        _pad0: [0; 7],
        tgid,
        pid,
        ts_ns,
        frame_count: n as u8,
        _pad1: [0; 3],
        ips,
    });
    slot.submit(0);
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
