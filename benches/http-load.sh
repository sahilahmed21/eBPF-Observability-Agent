#!/usr/bin/env bash
# Q12 load: concurrent write/read HTTP probes against latency-server.
set -euo pipefail
PORT="${1:-18080}"
WORKERS="${2:-32}"
ROUNDS="${3:-20}"
PROBE="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/http-probe"
for _ in $(seq 1 "$ROUNDS"); do
  for _w in $(seq 1 "$WORKERS"); do
    "$PROBE" --port "$PORT" --path /fast --path /users/123 --path '/slow?delay_ms=5' &
  done
  wait || true
done
