# TLS interception

Interview: *How do you intercept TLS without changing the app? Security implications?*

## Why syscall-only fails

After `SSL_write`, kernel TCP sees **ciphertext**. Phase 2 byte capture on `read`/`write` is useless for HTTPS apps that encrypt in userspace first.

## Mechanism (Milestone 3)

Hook the **OpenSSL library boundary** (plaintext):

| Library | Symbols | M3 status |
|---|---|---|
| OpenSSL 1.1 / 3.x | `SSL_set_fd` / `SSL_set_rfd` / `SSL_set_wfd`, `SSL_write`, `SSL_read` | **In scope** |
| GnuTLS | `gnutls_record_send` / `recv` | Out of Phase 3 |
| Go `crypto/tls`, rustls, BoringSSL-first-class | — | Out of Phase 3 (document unsupported) |

Uprobe attaches to **library path + symbol**, not a syscall. M3 try-attaches host
`libssl.so.3` and `libssl.so.1.1`; soft-fail if missing (cleartext HTTP must keep working).

```c
// Conceptual — plaintext already available
int SSL_set_fd(SSL *ssl, int fd);   // map SSL* → fd
int SSL_write(SSL *ssl, const void *buf, int num);
int SSL_read(SSL *ssl, void *buf, int num);
// bpf_probe_read_user(buf, min(ret, PREFIX_LEN), ...) on exit
```

No decryption on our side. Prefix length = **256 B** (same as Phase 2).

## Correlation (M3)

See [correlation.md](correlation.md) and [phase-3-implementation-plan.md](../phases/phase-3-implementation-plan.md):

```text
SSL_set_fd → SSL*→fd → SSL_read/write → TlsIo → existing (tgid,fd) correlator
```

**TLS-only latency** for Milestone 3. Dual-plane “TLS content + syscall wire timing” is explicitly
out of scope (would become a general TLS/network correlation engine).

Skip **prefix copy** on fds marked via `SSL_set_fd` (ciphertext is useless). Phase 8 still emits
timing-only `SockIoTimes` on those fds.

## Security implications

1. Agent must be **privileged** (`CAP_SYS_ADMIN` legacy, or `CAP_BPF` + `CAP_PERFMON` + `CAP_SYS_PTRACE` on modern kernels).
2. Agent sees **all hooked TLS plaintext** on the node — high-value target.
3. Scope tightly: DaemonSet node affinity, RBAC, audit what is captured (Phase 4 deploy).
4. **Metrics-only UI** in M3; never log raw prefixes. **Redact before export:** strip
   `Authorization`, `Cookie`, common secret patterns (Pixie/Tetragon-style policy).

Operational policy: [`docs/security.md`](../security.md).
