# Grafana dashboards (Phase 5)

Import [`obsagent.json`](obsagent.json) into Grafana.

Datasource: Prometheus scraping the collector (`otel-collector:8889` from
`deploy/k8s/otel-collector.yaml`). Metric names may be normalized by the
collector's Prometheus exporter (underscores / `_bucket` / `_total` suffixes).

Panels: latency quantiles, request rate by edge, agent drops, edge count.
