#!/usr/bin/env bash
# Phase 2 overhead under Q12 load (Axum + concurrent http-probe ~30s).
set -euo pipefail

BIN="${CARGO_TARGET_DIR:-target}/release/obsagent"
SERVER_BIN="${CARGO_TARGET_DIR:-target}/release/latency-server"
PROBE="${CARGO_TARGET_DIR:-target}/release/http-probe"
PORT=18081
DURATION=30
WORKERS=32
LOG=$(mktemp)
PSLOG=$(mktemp)

if [ "$(id -u)" -eq 0 ]; then PRIV=(env); else PRIV=(sudo -E); fi

[ -x "$BIN" ] || { echo "missing $BIN"; exit 1; }
[ -x "$SERVER_BIN" ] || { echo "missing $SERVER_BIN"; exit 1; }
[ -x "$PROBE" ] || { echo "missing $PROBE"; exit 1; }

PORT=$PORT "$SERVER_BIN" >/tmp/obs-overhead-server.log 2>&1 &
server_pid=$!
cleanup() {
  kill "$server_pid" 2>/dev/null || true
  kill "$agent_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true
  wait "$agent_pid" 2>/dev/null || true
}
trap cleanup EXIT
sleep 0.5

OBSAGENT_HEADLESS=1 RUST_LOG=warn "${PRIV[@]}" env OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
agent_pid=$!
sleep 2

(
  end=$((SECONDS + DURATION))
  while [ "$SECONDS" -lt "$end" ]; do
    ps -o pcpu=,rss= -p "$agent_pid" >>"$PSLOG" 2>/dev/null || true
    sleep 1
  done
) &
sampler=$!

end=$((SECONDS + DURATION))
while [ "$SECONDS" -lt "$end" ]; do
  for _ in $(seq 1 "$WORKERS"); do
    "$PROBE" --port "$PORT" --path /fast --path /users/1 --path '/slow?delay_ms=5' &
  done
  wait || true
done

wait "$sampler" 2>/dev/null || true
sleep 2
kill -INT "$agent_pid" 2>/dev/null || true
wait "$agent_pid" 2>/dev/null || true

echo "=== overhead-phase2 summary ==="
echo "load: http-probe ${WORKERS} workers × ${DURATION}s vs latency-server :${PORT}"
if [ -s "$PSLOG" ]; then
  python3 - "$PSLOG" <<'PY'
import sys
rows = []
for line in open(sys.argv[1]):
    parts = line.split()
    if len(parts) >= 2:
        rows.append((float(parts[0]), int(parts[1])))
if not rows:
    print("no ps samples")
else:
    cpu = sum(c for c, _ in rows) / len(rows)
    rss = max(r for _, r in rows)
    print(f"avg_pcpu={cpu:.2f}%  max_rss_kib={rss}  samples={len(rows)}")
PY
fi
grep -E 'drops=|http_60s=' "$LOG" | tail -n 5 || true
