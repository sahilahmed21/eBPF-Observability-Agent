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

**Phase 8:** sock probes on TLS fds emit **timestamps only** (`SockIoTimes`: fd, dir, ret, ts).
No ciphertext prefix is copied. The agent still sees OpenSSL plaintext via `TlsIo`.

Before any export off-node (Phase 4 OTLP and any future payload sinks):

1. Strip `Authorization`, `Cookie`, `Set-Cookie` (`redact_headers` in agent).
2. Redact common secret patterns (API keys, bearer tokens, card-like numbers — best-effort regex).
3. Prefer metrics + sanitized span attributes over raw payloads in production configs.

**Phase 9:** sampled OTLP spans carry identity + method/route/status/protocol and optional
wire/handshake durations. Prefix bytes and headers are **not** span attributes. Query strings
are stripped from `http.route`. Collector logs may still show those attributes — do not add
payloads later without the redaction list above.

RingBuf events whose **tgid is this process** (userspace ids plus a BPF-learned tgid),
or whose comm is `obsagent`, are dropped before TCP/HTTP ingest. POST `/v1/traces`,
`/v1/metrics`, and `/v1/logs` are not published as application HTTP. The k8s
default deny list includes `otelcol` / `otelcol-contrib`. This is process identity
and export-route skip, not an OTLP port denylist.

**Phase 11:** `DENIED_TGID` (deny-list) or `ALLOWED_TGID` + `ALLOW_ONLY` (exclusive `OBSAGENT_COMM_ALLOW`) is the same policy in the kernel so non-captured processes do
not pay prefix copy. It is **not** a cluster ACL. `OBSAGENT_SAMPLE_N` is a load
valve (connection keep), not a privacy control — latency events still emit for
kept, captured processes. `bpf_get_prandom_u32` is not cryptographic. Prefixes
are still never logged.

## Deployment scope

- DaemonSet per node, not on every developer laptop by default.
- RBAC: `get/list` on **pods** (src via cgroup UID) and **services** (dest ClusterIP → Service `ns/name`). No `watch` until informers exist. No EndpointSlice in Milestone 10 — ClusterIP is the syscall dest. Token is never a span attribute.
- Audit: log attach targets (PIDs/libs) at info level; no payload logging in prod.

## Parallel

Pixie / Tetragon ship RBAC + redaction for the same reason — we mirror the *policy*, not their codebase.
