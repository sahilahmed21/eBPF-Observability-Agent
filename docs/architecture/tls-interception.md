# TLS interception

Interview: *How do you intercept TLS without changing the app? Security implications?*

## Why syscall-only fails

After `SSL_write`, kernel TCP sees **ciphertext**. Phase 2 byte capture on `read`/`write` is useless for HTTPS apps that encrypt in userspace first.

## Mechanism

Hook the **TLS library boundary** (plaintext):

| Library | Symbols | Notes |
|---|---|---|
| OpenSSL 1.1 / 3.x | `SSL_write`, `SSL_read` | Primary target |
| GnuTLS | `gnutls_record_send` / `recv` | Later |
| Go `crypto/tls` | runtime internals | No OpenSSL; separate stretch |

Uprobe attaches to **binary/library path + symbol offset**, not a syscall. Resolve `libssl.so.1.1` vs `libssl.so.3` at attach time (version skew is a production headache).

```c
// Conceptual — plaintext already available
int SSL_write(SSL *ssl, const void *buf, int num);
// bpf_probe_read_user(buf, min(num, PREFIX_LEN), ...)
```

No decryption on our side.

## Correlation with syscall timeline

See [correlation.md](correlation.md): TLS = content, syscall = timing; key `(pid, tid)` + tight ts window.

## Security implications

1. Agent must be **privileged** (`CAP_SYS_ADMIN` legacy, or `CAP_BPF` + `CAP_PERFMON` + `CAP_SYS_PTRACE` on modern kernels).
2. Agent sees **all hooked TLS plaintext** on the node — high-value target.
3. Scope tightly: DaemonSet node affinity, RBAC, audit what is captured.
4. **Redact before export:** strip `Authorization`, `Cookie`, common secret patterns (Pixie/Tetragon-style).

Operational policy: [`docs/security.md`](../security.md).
