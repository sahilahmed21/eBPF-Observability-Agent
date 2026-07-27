# Demo workloads

Local / kind targets used to validate milestones. **Not** instrumented — the agent observes them from the outside.

| Directory | Milestone | Purpose |
|-----------|-----------|---------|
| `http-server/` | 1–2 | Plain HTTP with injectable latency |
| `https-server/` | 3 | TLS via OpenSSL-linked stack |
| `microservices/` | 4 | Multi-service graph for service map |

Scaffolding only until the matching phase starts.
