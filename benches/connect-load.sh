#!/usr/bin/env bash
# Fixed Phase 1 load: N sequential HTTPS GETs (IPv4). Used for docs/overhead.md.
set -euo pipefail
N="${1:-50}"
for i in $(seq 1 "$N"); do
  curl -s --max-time 3 -o /dev/null "http://1.1.1.1/" || true
done
