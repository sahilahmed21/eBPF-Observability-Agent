# Handoff — Kind e2e + OTLP ingest (2026-08-15)

**Cluster:** `kind` name `obsagent`  
**Image:** `ebpf-obs-agent:latest` (Dockerfile installs prebuilt `bpf-linker` musl tarball)

## Proven

- Local: `test-agent` 38; smoke3; correctness3; smoke4 without kind
- `KIND_E2E=1` smoke4: DaemonSet + otel-collector rolled out; demo ns Running
- OTLP/HTTP JSON accepted by collector 0.96 (`MetricsExporter` every ~10s)
- Prometheus scrape on collector `:8889`: cumulative histograms + `obsagent_*` self-metrics
- Demo path visible: `GET /` to api ClusterIP `10.96.28.98:8080`

## Fixes this session

- OTLP JSON root object was missing a closing `}` → HTTP 400
- `bucketCounts` must be **per-bucket** (sum == `count`), not Prometheus cumulative
- Warn log now includes collector response body on non-2xx

## Known gaps (do not over-claim)

- Kind-in-Docker on WSL: local identity stays `proc:unknown:tgid`
- Node-wide HTTP (Docker API, kube health) floods cardinality; Grafana will look noisy until a drop/allow list exists
- Stretch S1 HTTP/2 and S2 CPU profiles **not started**

## Next

1. Commit Phase 4+5 + OTLP JSON fix when asked
2. Optional: route allow-list / drop Docker+kube probes before Grafana screenshots
3. Stretch S1 only after that (or skip if interview pack is the goal)
