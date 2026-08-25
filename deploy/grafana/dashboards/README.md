# Grafana dashboards (Phase 5 / Phase 9)

Import [`obsagent.json`](obsagent.json) into Grafana.

Datasource: Prometheus scraping the collector (`otel-collector:8889` from
`deploy/k8s/otel-collector.yaml`, image `otel/opentelemetry-collector:0.96.0`).

OTLP names use dots (`http.client.duration`, unit `ms`). The collector
Prometheus exporter (0.96 **defaults**: unit suffixes on, type suffixes on)
rewrites them to the **pinned** scrape series in the dashboard JSON:

| OTLP name | Scrape series used in queries |
|---|---|
| `http.client.duration` (ms histogram) | `http_client_duration_milliseconds_bucket` |
| `http.client.requests` | `http_client_requests_total` |
| `tls.handshake.duration` (ms histogram) | `tls_handshake_duration_milliseconds_bucket` |
| `obsagent.events_dropped` | `obsagent_events_dropped_total` |
| `obsagent.sample_n` (gauge) | `obsagent_sample_n` |
| `obsagent.traces.sampled` | `obsagent_traces_sampled_total` |
| `obsagent.traces.not_sampled` | `obsagent_traces_not_sampled_total` |
| `obsagent.traces.dropped` | `obsagent_traces_dropped_total` |
| `obsagent.traces.export_failed` | `obsagent_traces_export_failed_total` |

Do not set `without_units: true` or `without_type_suffix: true` on that
exporter — the dashboard is the contract with those defaults.

Histogram **label** `http.protocol` stays `http` / `h2` / `grpc` (not `http/1.1`).
Span `service.name` is the reconstructed `src`; `telemetry.sdk.name` is `obsagent`.

Screenshot rule: allow-list on; do not use Docker API as the hero series.
The agent does not ingest its own tgid (OTLP POSTs are not HTTP rows).
