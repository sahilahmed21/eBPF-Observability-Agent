# Phase 4 TDD evidence

## Locked Qs

See [phase-4-implementation-plan.md](../phases/phase-4-implementation-plan.md).

## Unit tests

| Suite | Result (2026-08-12) |
|---|---|
| `wsl-run.sh test-common` | PASS (incl. `sock_meta_layout`) |
| `wsl-run.sh test-agent` | **33 passed** (identity, service_map, export, k8s_index, prior suites) |

## Regression gates (2026-08-12, WSL root, after release build)

| Gate | Result |
|---|---|
| smoke2 | PASS |
| smoke3 | PASS (`tlsio`, HTTP rows, edges in headless) |
| correctness3 | PASS `GET /slow count=5` p50≈50.86ms; edges `-> 127.0.0.1:…` |

Headless signature excerpt:

```text
http_60s=15 sockio=0 tlsio=60 drops=0 edges=2 otlp_ok=0 otlp_drop=0
GET /slow count=5 ... p50=50.86ms
[edge] proc:… -> 127.0.0.1:18444 count=14 …
```

## Phase 4 manual / kind

```bash
# Image + cluster (operator)
docker build -f deploy/Dockerfile -t ebpf-obs-agent:latest .
kubectl apply -f deploy/k8s/rbac.yaml
kubectl apply -f deploy/k8s/otel-collector.yaml
kubectl apply -f deploy/k8s/daemonset.yaml
kubectl apply -f demos/microservices/k8s.yaml
```

Headless agent should show `edges=` and `[edge]` rows; collector logs OTLP metrics when endpoint set.
