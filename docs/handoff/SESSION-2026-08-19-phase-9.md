# Handoff — Phase 9 traces + Grafana (2026-08-19)

**Paste into a new chat:** *Read `docs/handoff/SESSION-2026-08-19-phase-9.md` and continue from §Next. Do not re-litigate locked Qs. Do not claim VISION-95 resume sentence. Do not commit unless asked. Do not lift Q8 (first iovec, 256 B). Handshake on spans is an **attribute**, not a child.*

**Checklist:** [`docs/phases/phase-9.md`](../phases/phase-9.md)  
**Plan + §10:** [`docs/phases/phase-9-implementation-plan.md`](../phases/phase-9-implementation-plan.md)  
**TDD:** [`docs/testing/phase-9.tdd.md`](../testing/phase-9.tdd.md)  
**Prior:** Phase 8 dual-plane (M8 green; do not reverse content-primary join).

**Repo:** `c:\projects\eBPF-Observability-Agent` (Windows checkout; **build/run only on WSL2 Ubuntu**)  
**Working tree:** **UNCOMMITTED.** Do **not** commit unless the user asks.

---

## 0. Environment

```bash
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh test-agent
wsl -d Ubuntu -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh build
wsl -d Ubuntu -u root -- bash /mnt/c/projects/eBPF-Observability-Agent/scripts/wsl-run.sh smoke9
```

Always `wsl -d Ubuntu` (not `docker-desktop`). Root gates: `wsl -d Ubuntu -u root`.

---

## 1. What shipped

Sampled OTLP/HTTP JSON **traces** from completed HTTP/1.1 and h2/gRPC exchanges. One **root** span per exchange/stream. Handshake is `obsagent.tls.handshake_ns` on the first sampled span for that `(tgid, fd)` (consumed once). Same `ExportHub` 10 s tick; drain never awaits the collector. Grafana dashboard PromQL uses **exported** stems.

**P9-Q4 amendment (locked):** not a child span — handshake ends before HTTP; parent/child times would be invalid.

**P9-Q2:** splitmix64, no blake3/sha256 crate.

---

## 2. Gates

| Gate | Result | Date |
|---|---|---|
| `test-agent` | **123 passed** | 2026-08-19 |
| `smoke9` | **PASS** `trace_s=10`; sink `/v1/traces` 2xx; no OTLP HTTP/TCP rows | 2026-08-19 |

---

## 3. Next (Phase 10)

Real-node named service map. Kind-in-Docker identity remains Phase 10. Do not start Phase 10 BPF/ABI work as part of a Grafana screenshot.

Optional handoff: Grafana screenshot against a real Prometheus scrape (VISION-95 §7 Milestone 9 “Grafana live”). JSON + stem grep already gated.

---

## 4. Residuals

- RingBuf drain drops this process: userspace tgid/tids, BPF-learned tgid (localhost calib connect — `getpid()` can disagree with BPF on WSL), and comm `obsagent`. k8s-default `otelcol`. POST `/v1/traces`/`/v1/metrics`/`/v1/logs` are not published as app HTTP (route, not :4318). `smoke9` uses a uniquely named sink comm as extra deny.
- Handshake ticket residual is Phase 8 (unchanged).
- No Tempo/Jaeger required; collector logging exporter is the visibility path in `deploy/k8s/otel-collector.yaml`.
