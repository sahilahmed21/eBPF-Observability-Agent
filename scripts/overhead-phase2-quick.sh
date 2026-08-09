#!/usr/bin/env bash
set -euo pipefail
export CARGO_TARGET_DIR=/home/sahil/.cache/obsagent-target
BIN="$CARGO_TARGET_DIR/release/obsagent"
SRV="$CARGO_TARGET_DIR/release/latency-server"
PR="$CARGO_TARGET_DIR/release/http-probe"
PORT=18081
LOG=/tmp/oha.log
PSLOG=/tmp/ohps.log
: >"$PSLOG"

pkill -9 -f '/release/obsagent' 2>/dev/null || true
pkill -9 -f latency-server 2>/dev/null || true
sleep 1

PORT=$PORT "$SRV" >/tmp/ohs.log 2>&1 &
spid=$!
sleep 0.5
OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
apid=$!
sleep 2
echo "agent_pid=$apid"

for i in 1 2 3 4 5 6 7 8; do
  "$PR" --port "$PORT" --repeat 2 --path /fast --path /users/1 --path '/slow?delay_ms=5' || true
  ps -o pcpu=,rss= -p "$apid" >>"$PSLOG" 2>/dev/null || true
done

kill -INT "$apid" 2>/dev/null || true
wait "$apid" 2>/dev/null || true
kill "$spid" 2>/dev/null || true

echo "=== overhead-phase2 summary ==="
echo "load: http-probe 8 rounds × 2 repeats × 3 paths vs :$PORT"
python3 - "$PSLOG" <<'PY'
import sys
rows=[]
for line in open(sys.argv[1]):
    p=line.split()
    if len(p)>=2: rows.append((float(p[0]), int(p[1])))
if not rows:
    print("no ps samples"); raise SystemExit(1)
cpu=sum(c for c,_ in rows)/len(rows)
rss=max(r for _,r in rows)
print(f"avg_pcpu={cpu:.2f}%  max_rss_kib={rss}  samples={len(rows)}")
PY
grep -E 'drops=|http_60s=' "$LOG" | tail -n 5 || true
