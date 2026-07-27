# `deploy/`

Container image + Kubernetes manifests (DaemonSet, RBAC, ServiceAccount).

Filled in Phase 4. Caps: prefer `CAP_BPF` / `CAP_PERFMON` / `CAP_SYS_PTRACE` over full privileged when possible.
