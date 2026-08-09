# Security

## Trust boundary

This agent can observe **plaintext application traffic** (especially with TLS uprobes). It is not a casual sidecar. Treat it like a privileged security agent.

## Privileges

Prefer modern capability set over blanket `privileged: true` when the kernel supports it:

- `CAP_BPF`
- `CAP_PERFMON`
- `CAP_SYS_PTRACE` (uprobes / process attach)
- Mounts: `/sys/kernel/btf` (ro), `/sys/kernel/debug` as required

Document actual caps used in `deploy/k8s/` when Phase 4 lands.

## Data handling

**Phase 3 / Milestone 3 (locked):** metrics-only UI; **never log raw prefixes**. Secrets may still
exist briefly in RingBuf/correlator RAM — treat the agent process as the trust boundary.

Before any export off-node (Phase 4 OTLP and any future payload sinks):

1. Strip `Authorization`, `Cookie`, `Set-Cookie` (`redact_headers` in agent).
2. Redact common secret patterns (API keys, bearer tokens, card-like numbers — best-effort regex).
3. Prefer metrics + sanitized span attributes over raw payloads in production configs.

## Deployment scope

- DaemonSet per node, not on every developer laptop by default.
- RBAC: minimal K8s API access for pod identity only.
- Audit: log attach targets (PIDs/libs) at info level; no payload logging in prod.

## Parallel

Pixie / Tetragon ship RBAC + redaction for the same reason — we mirror the *policy*, not their codebase.
