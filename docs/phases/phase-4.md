# Phase 4 — Production-grade

## Checklist

- [ ] Service map: nodes = process/container; edges = rate/p99/errors
- [ ] PID → cgroup → container → pod/namespace (K8s API or CRI)
- [ ] OTLP traces + metrics (`opentelemetry` / `opentelemetry-otlp`)
- [ ] Local Collector → Prometheus + Tempo/Jaeger → Grafana dashboard
- [ ] Multi-stage Dockerfile (musl / distroless or scratch)
- [ ] DaemonSet: least-privilege caps, BTF + debugfs mounts
- [ ] kind/minikube demo with multi-service app

## Milestone 4

`kubectl apply` DaemonSet; service map + Grafana populate with zero app instrumentation.
