# Phase plans

North star (95% of the original brief): [VISION-95.md](VISION-95.md)  
Roadmap checkboxes: [../ROADMAP.md](../ROADMAP.md)

Same layout as Phases 0–5: **checklist** (`phase-N.md`) + **how** (`phase-N-implementation-plan.md`) + **TDD** (`../testing/phase-N.tdd.md`).

| N | Goal | Start when |
|---|---|---|
| [0](phase-0.md) | Toolchain + hello kprobe | — |
| [1](phase-1.md) | connect/accept latency | M0 |
| [2](phase-2.md) | HTTP/1.1 | M1 |
| [3](phase-3.md) | OpenSSL TLS-only | M2 |
| [4](phase-4.md) | DaemonSet + metrics | M3 |
| [5](phase-5.md) | OTLP histograms | M4 |
| [6](phase-6.md) | writev + reassembly + deny + IPv6 | M5 |
| [7](phase-7.md) | HTTP/2 + gRPC | **M6** |
| [8](phase-8.md) | Dual-plane TLS + handshake | M7 |
| [9](phase-9.md) | OTLP traces + Grafana | M8 |
| [10](phase-10.md) | Real-node named service map | M9 (VM may start at 6) |
| [11](phase-11.md) | `perf stat` + sampling | M10 |
| [12](phase-12.md) | CPU profiles + **claim lock** | M11 |

First implementation step: [phase-6-implementation-plan.md](phase-6-implementation-plan.md) §3 subphase **6.0**.

Do not implement Phase 7 until Milestone 6 is ticked. Do not use the original resume sentence until Milestone 12.
