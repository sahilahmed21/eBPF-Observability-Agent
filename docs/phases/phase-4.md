# Phase 4 — Production-grade

Plan: [phase-4-implementation-plan.md](phase-4-implementation-plan.md) (Qs locked).

## Checklist

- [x] Service map: nodes = process/container/pod; edges = rate/p99/errors (in-memory)
- [x] PID → cgroup → container → pod UID; pod name via K8s API soft-fail index
- [x] OTLP metrics (`obsagent.http.client.duration_ms` via OTLP/HTTP JSON); traces deferred
- [x] Local Collector manifest → Prometheus exporter scrape port
- [x] Multi-stage Dockerfile
- [x] DaemonSet: caps + BTF/debug/proc mounts; privileged fallback documented
- [x] kind/minikube demo manifests (`demos/microservices/k8s.yaml`)

## Milestone 4

`kubectl apply` DaemonSet + demo; service map edges in headless output; OTLP reaches collector when configured.
