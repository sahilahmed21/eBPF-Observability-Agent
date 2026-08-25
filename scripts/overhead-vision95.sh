#!/usr/bin/env bash
# Vision-95 headline load (Phase 11). Not a 2% proof on WSL kind.
#
# Pins: vegeta v12.13.0, ghz v0.121.0 — see benches/vision95-load.md.
#
# Usage (on the measurement node):
#   HTTP_URL=http://127.0.0.1:8080/ GRPC_TARGET=127.0.0.1:9000 ./scripts/overhead-vision95.sh
#   AGENT_PID=<obsagent pid> ./scripts/overhead-vision95.sh   # also runs perf stat -p
set -euo pipefail

HTTP_URL="${HTTP_URL:-http://127.0.0.1:8080/}"
GRPC_TARGET="${GRPC_TARGET:-127.0.0.1:9000}"
GRPC_CALL="${GRPC_CALL:-hello.HelloService/SayHello}"
DURATION_SECS="${DURATION_SECS:-60}"
HTTP_RATE="${HTTP_RATE:-500}"
GRPC_RPS="${GRPC_RPS:-200}"
WORKERS="${WORKERS:-64}"
MAX_WORKERS="${MAX_WORKERS:-256}"
GRPC_CONNS="${GRPC_CONNS:-50}"
AGENT_PID="${AGENT_PID:-}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing $1 (pin: $2)" >&2
    exit 1
  }
}
need vegeta "vegeta v12.13.0"
need ghz "ghz v0.121.0"

echo "overhead-vision95 HTTP_URL=$HTTP_URL rate=${HTTP_RATE}/s ${DURATION_SECS}s workers=$WORKERS max-workers=$MAX_WORKERS"
echo "overhead-vision95 GRPC_TARGET=$GRPC_TARGET rps=$GRPC_RPS conns=$GRPC_CONNS call=$GRPC_CALL"

http_out=$(mktemp)
grpc_out=$(mktemp)
perf_out=""
perf_pid=""
cleanup() {
  rm -f "$http_out" "$grpc_out"
  if [[ -n "${perf_out}" ]]; then rm -f "$perf_out"; fi
}
trap cleanup EXIT

if [[ -n "$AGENT_PID" ]]; then
  need perf "linux-perf (perf stat -p)"
  # Prefer /proc: kill -0 fails for a root DaemonSet agent when we are not root.
  if [[ ! -d "/proc/$AGENT_PID" ]]; then
    echo "AGENT_PID=$AGENT_PID is not running" >&2
    exit 1
  fi
  perf_out=$(mktemp)
  echo "perf stat -p $AGENT_PID sleep ${DURATION_SECS}s (task-clock; % of one core ≈ CPUs utilized × 100)"
  echo "footnote: eBPF probe time is billed to syscall CPUs, not this process"
  # Root agent → need root perf; fall back to sudo -n when not already root.
  if [[ "$(id -u)" -eq 0 ]]; then
    perf stat -p "$AGENT_PID" sleep "$DURATION_SECS" >"$perf_out" 2>&1 &
  else
    sudo -n perf stat -p "$AGENT_PID" sleep "$DURATION_SECS" >"$perf_out" 2>&1 &
  fi
  perf_pid=$!
fi

echo "GET $HTTP_URL" | vegeta attack \
  -http2=false \
  -rate="${HTTP_RATE}/s" \
  -duration="${DURATION_SECS}s" \
  -workers="$WORKERS" \
  -max-workers="$MAX_WORKERS" \
  >"$http_out" &
http_pid=$!

ghz --insecure \
  --call "$GRPC_CALL" \
  -d '{"greeting":"obs"}' \
  --connections="$GRPC_CONNS" \
  -c "$GRPC_CONNS" \
  --rps "$GRPC_RPS" \
  --duration="${DURATION_SECS}s" \
  "$GRPC_TARGET" >"$grpc_out" 2>&1 &
grpc_pid=$!

http_rc=0
grpc_rc=0
wait "$http_pid" || http_rc=$?
wait "$grpc_pid" || grpc_rc=$?
if [[ -n "$perf_pid" ]]; then
  wait "$perf_pid" || true
fi

echo "=== vegeta report (HTTP/1.1) ==="
vegeta report <"$http_out"
echo "=== ghz (gRPC) ==="
cat "$grpc_out"

if [[ -n "$perf_out" && -f "$perf_out" ]]; then
  echo "=== perf stat -p $AGENT_PID ==="
  cat "$perf_out"
fi

if [[ "$http_rc" -ne 0 ]]; then
  echo "vegeta failed rc=$http_rc" >&2
  exit "$http_rc"
fi
if [[ "$grpc_rc" -ne 0 ]]; then
  echo "ghz failed rc=$grpc_rc" >&2
  exit "$grpc_rc"
fi
