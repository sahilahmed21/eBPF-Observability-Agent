# TLS interception — security implications

## What we do

For HTTPS workloads, ciphertext at the TCP layer is useless for HTTP parsing.
We attach **uprobes** to OpenSSL (`SSL_write` / `SSL_read`) so we observe **plaintext** at the library boundary — no application code changes, no private key theft, no MITM cert injection.

## Why this is a trust boundary

| Fact | Implication |
|------|-------------|
| Agent must run privileged (BPF + ptrace-class caps) | Compromise of the agent ≈ compromise of host observability plane |
| Plaintext includes headers and bodies (we only keep prefixes) | Secrets (`Authorization`, cookies, tokens) may appear in captured bytes |
| DaemonSet runs on every node | Blast radius is cluster-wide if RBAC/network export is loose |

This is the same reason Pixie / Tetragon emphasize RBAC, least privilege, and **redaction** before data leaves the node.

## Mitigations we commit to

1. **Redaction** before CLI display / OTLP export: strip `Authorization`, `Cookie`, and common secret patterns.
2. **Prefix-only capture** (bounded N bytes) — reduces body leakage, not a complete control.
3. **Scoped deployment**: node-local agent; restrict who can read exported telemetry.
4. **Audit**: log attach targets (PIDs/libs) and config changes.
5. Prefer **capability set** (`CAP_BPF`, `CAP_PERFMON`, `CAP_SYS_PTRACE`) over blanket `privileged: true` when the kernel allows.

## What we explicitly do not claim

- We do not make multi-tenant “untrusted workload on same host” safe by default.
- We do not support every TLS stack (Go crypto/tls, BoringSSL variants) without additional work.
- Redaction is best-effort pattern matching — not a compliance certification.
