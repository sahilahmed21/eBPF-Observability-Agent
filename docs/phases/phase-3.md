# Phase 3 — TLS interception

## Checklist

- [ ] Resolve + attach `SSL_write` / `SSL_read` (OpenSSL 1.1 and 3.x paths)
- [ ] Plaintext prefix via `bpf_probe_read_user`
- [ ] Correlate TLS ↔ syscall planes
- [ ] Redaction pass (`Authorization`, `Cookie`, secret patterns)
- [ ] HTTPS test service (self-signed OK)
- [ ] Security section reviewed (`docs/security.md`)
- [ ] Overhead row for Phase 3

## Milestone 3

Same per-endpoint breakdown as Phase 2 over TLS, with documented redaction.
