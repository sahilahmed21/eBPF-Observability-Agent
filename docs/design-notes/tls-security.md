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

1. **Metrics-only UI** in Milestone 3 — never log raw prefixes. **Redaction** before any off-node
   export / payload sink: strip `Authorization`, `Cookie`, and common secret patterns.
2. **Prefix-only capture** (256 B) — reduces body leakage, not a complete control.
3. **Scoped deployment**: node-local agent; restrict who can read exported telemetry (Phase 4).
4. **Audit**: log attach targets (lib paths) and config changes — not payloads.
5. Prefer **capability set** (`CAP_BPF`, `CAP_PERFMON`, `CAP_SYS_PTRACE`) over blanket `privileged: true` when the kernel allows.

## What we explicitly do not claim

- We do not make multi-tenant “untrusted workload on same host” safe by default.
- We do not support every TLS stack (Go crypto/tls, rustls, BoringSSL variants) in Phase 3.
- We do not implement dual-plane TLS↔syscall wire-timing correlation in Milestone 3.
- Redaction is best-effort pattern matching — not a compliance certification.
