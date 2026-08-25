#!/usr/bin/env bash
# Vision-95 overload (Phase 11): 5k RPS HTTP/1.1 for 15 s.
# PASS: RingBuf drops rise OR sample_n >= 2, and HTTP series still increment.
# RSS must stay bounded (no userspace queue growth).
set -euo pipefail

HTTP_URL="${HTTP_URL:-http://127.0.0.1:8080/}"
DURATION_SECS="${DURATION_SECS:-15}"
HTTP_RATE="${HTTP_RATE:-5000}"
WORKERS="${WORKERS:-128}"
MAX_WORKERS="${MAX_WORKERS:-512}"
AGENT_PID="${AGENT_PID:-}"

command -v vegeta >/dev/null 2>&1 || {
  echo "missing vegeta (pin: v12.13.0)" >&2
  exit 1
}

echo "overload-vision95 HTTP_URL=$HTTP_URL rate=${HTTP_RATE}/s ${DURATION_SECS}s workers=$WORKERS max-workers=$MAX_WORKERS"

rss_before=""
if [[ -n "$AGENT_PID" ]]; then
  # Prefer /proc: kill -0 fails for a root DaemonSet agent when we are not root.
  if [[ ! -d "/proc/$AGENT_PID" ]]; then
    echo "AGENT_PID=$AGENT_PID is not running" >&2
    exit 1
  fi
  rss_before=$(awk '/VmRSS:/ {print $2}' "/proc/$AGENT_PID/status" 2>/dev/null || true)
  echo "agent_pid=$AGENT_PID rss_kib_before=${rss_before:-unknown}"
fi

echo "GET $HTTP_URL" | vegeta attack \
  -http2=false \
  -rate="${HTTP_RATE}/s" \
  -duration="${DURATION_SECS}s" \
  -workers="$WORKERS" \
  -max-workers="$MAX_WORKERS" \
  | vegeta report

if [[ -n "$AGENT_PID" ]]; then
  rss_after=$(awk '/VmRSS:/ {print $2}' "/proc/$AGENT_PID/status" 2>/dev/null || true)
  echo "rss_kib_after=${rss_after:-unknown}"
  echo "check agent log: drops= or sample_n>=2; http_60s still moving"
fi
