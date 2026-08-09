# Probe map

Inventory of attachments the agent will use. Prefer **tracepoints** when a stable ABI exists; use **kprobes** for socket-layer CO-RE; use **uprobes** for TLS plaintext.

## Phase 1 — connection timing

| Kind | Attach point | On entry | On exit / fire |
|------|--------------|----------|----------------|
| Tracepoint | `syscalls:sys_enter_connect` | Store `(tgid,pid) → ts, fd, sockaddr ptr` | — |
| Tracepoint | `syscalls:sys_exit_connect` | — | Lookup, latency, emit RingBuf |
| Tracepoint | `syscalls:sys_enter_accept4` | Store pending accept | — |
| Tracepoint | `syscalls:sys_exit_accept4` | — | Latency + new fd |
| Kprobe (optional, preferred for addrs) | `tcp_v4_connect` / related | Read `struct sock` via CO-RE | Enrich saddr/daddr/sport/dport |

**Notes**

- Modern servers usually call `accept4`, not `accept` — probe both if needed, prioritize `accept4`.
- Tracepoints are more portable across kernel builds than kprobes on internal symbol names.

## Phase 2 — HTTP byte prefixes

| Kind | Attach point | Capture |
|------|--------------|---------|
| Tracepoint / kprobe | `sys_enter_write` / `sys_enter_sendto` (filtered) | First N bytes of userspace buffer if fd is tracked TCP socket |
| Tracepoint / kprobe | `sys_enter_read` / `sys_enter_recvfrom` (filtered) | Same for reads |

**Filters (critical for overhead)**

- Only fds previously seen in connect/accept (or classified as TCP via sock lookup).
- Cap `N` (256–512) to respect map value / stack limits.
- Optional: target TGID allowlist for MVP (single process) before going system-wide.

## Phase 3 — TLS (OpenSSL / Milestone 3)

| Kind | Attach point | Capture |
|------|--------------|---------|
| Uprobe | `SSL_set_fd` / `SSL_set_rfd` / `SSL_set_wfd` | `SSL*` → fd map |
| Uprobe + uretprobe | `SSL_write` / `SSL_write_ex` | enter stash / exit plaintext prefix |
| Uprobe + uretprobe | `SSL_read` / `SSL_read_ex` | enter stash / exit decrypted plaintext |

**Note:** OpenSSL 3 / CPython call `SSL_*_ex`; classic `SSL_read`/`SSL_write` remain attached for older callers.

**Attach (M3 locked)**

1. Try-attach host `libssl.so.3` and `libssl.so.1.1` (candidate paths).
2. Soft-fail if missing — cleartext Phase 2 must keep working.
3. Handle BoringSSL / LibreSSL / static OpenSSL / Go `crypto/tls` / rustls as **unsupported** with a clear log — document, don’t pretend.
4. Skip Phase 2 sock I/O on TLS-marked fds. **No** dual-plane TLS↔syscall timing merge in M3.

## Stretch — profiling

| Kind | Attach point | Capture |
|------|--------------|---------|
| Perf event | CPU cycles @ ~99Hz | Kernel + userspace stack IDs |

## What we intentionally do not attach (MVP)

- Unfiltered `tcp_sendmsg` system-wide without fd filters (too hot).
- Full packet capture / AF_PACKET sniffers for HTTP body reconstruction.
- Every SSL library variant on day one.
