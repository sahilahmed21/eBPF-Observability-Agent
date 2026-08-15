# `deploy/`

Container image + Kubernetes manifests (DaemonSet, RBAC, ServiceAccount, OTLP collector).

## Apply (kind / cluster)

```bash
docker build -f deploy/Dockerfile -t ebpf-obs-agent:latest .
kubectl apply -f deploy/k8s/rbac.yaml
kubectl apply -f deploy/k8s/otel-collector.yaml
kubectl apply -f deploy/k8s/configmap.yaml
kubectl apply -f deploy/k8s/daemonset.yaml
kubectl apply -f demos/microservices/k8s.yaml
```

Caps: prefer `CAP_BPF` / `CAP_PERFMON` / `CAP_SYS_PTRACE` / `CAP_SYS_RESOURCE`. Set `privileged: true` only if the cluster rejects distinct caps.

Env:

| Var | Meaning |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Collector base URL (agent appends `/v1/metrics`) |
| `OBSAGENT_OTLP=1` | Default endpoint `http://127.0.0.1:4318` |
| `OBSAGENT_PROC_ROOT` | Override procfs root (default `/proc` or `/host/proc`) |
| `OBSAGENT_HEADLESS=1` | Metrics print loop |
