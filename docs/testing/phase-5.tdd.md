# Phase 5 TDD evidence

## Locked Qs

See [phase-5-implementation-plan.md](../phases/phase-5-implementation-plan.md).

## Unit / regression (run on WSL)

```bash
wsl-run.sh test-agent   # includes metrics_registry + peer_cache + export
wsl-run.sh build
wsl-run.sh smoke3
wsl-run.sh correctness3
wsl-run.sh smoke4
# optional:
KIND_E2E=1 wsl-run.sh smoke4
```

## Results (2026-08-13, WSL)

| Command | Result |
|---|---|
| `wsl-run.sh test-agent` | **38 passed** |
| `wsl-run.sh build` | PASS |
| `wsl-run.sh smoke3` | PASS |
| `wsl-run.sh correctness3` | PASS `/slow count=5` p50≈50.82ms; `edges=1` |
| `wsl-run.sh smoke4` | PASS (kind SKIP unless `KIND_E2E=1`) |

OTLP body from registry contains `histogram` + `aggregationTemporality":2` (unit-tested).

## Kind e2e (2026-08-15)

Cluster `obsagent` (kind v0.29, k8s v1.33.1, WSL kernel `6.6.114.1-microsoft-standard-WSL2`).

| Check | Result |
|---|---|
| `KIND_E2E=1 wsl-run.sh smoke4` | **PASS** DS + otel-collector rollout |
| Demo `api` / `frontend` | Running |
| OTLP JSON | First payload 400 (unclosed root + Prometheus-style `bucketCounts`); fixed; `POST /v1/metrics` **2xx** |
| Collector scrape `:8889` | `http_client_duration_milliseconds` histogram; `obsagent_events_dropped_total 0`; `obsagent_otlp_dropped_total 0` |
| Demo series | `GET /` → `10.96.28.98:8080` count=99; dst `demo/frontend-…` count=99 |

**Kind-in-Docker limits (not a real-node DaemonSet proof):** `src=proc:unknown:…` (BPF PIDs are WSL host; pod `/host/proc` is the kind node). Scrape is dominated by host Docker API + kube-proxy `/readyz` because `hostPID: true`.

