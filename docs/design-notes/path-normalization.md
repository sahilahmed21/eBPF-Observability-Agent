# Path normalization

## Why

Per-URL histograms fragment (`/user/1`, `/user/2`, …). We normalize to a low-cardinality route key for aggregation.

## Heuristics (Phase 2)

Apply segment-wise on the URL path:

| Segment shape | Replacement |
|---------------|-------------|
| All digits | `:id` |
| UUID (8-4-4-4-12 hex) | `:uuid` |
| Long hex (≥16) | `:hex` |
| Otherwise | keep literal |

Examples:

- `/user/123` → `/user/:id`
- `/orders/a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11/items` → `/orders/:uuid/items`

## Non-goals (MVP)

- Full framework route tables (Axum/Actix/Gin) introspection
- Query-string normalization beyond dropping the query for the route key
- Language-specific router dumping

Document false merges (e.g. `/v2` → might stay literal; `/2` alone becomes `:id`) and refine only when demos demand it.
