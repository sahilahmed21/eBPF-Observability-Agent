#!/usr/bin/env bash
# Sample Phase 1 overhead under benches/connect-load.sh (must run as root).
set -euo pipefail
source /home/sahil/.cargo/env 2>/dev/null || true
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}"
ROOT=/mnt/c/projects/eBPF-Observability-Agent
BIN="$CARGO_TARGET_DIR/release/obsagent"
python3 -c "open('/tmp/load.sh','wb').write(open('$ROOT/benches/connect-load.sh','rb').read().replace(b'\\r',b''))"
OBSAGENT_HEADLESS=1 RUST_LOG=warn "$BIN" >/tmp/obs-oh.log 2>&1 &
APID=$!
sleep 2
START=$(date +%s%N)
bash /tmp/load.sh 50 >/dev/null || true
END=$(date +%s%N)
sleep 1
ps -p "$APID" -o %cpu=,rss= --no-headers | awk '{print "cpu_pct="$1" rss_kb="$2}'
echo "load_ns=$((END-START))"
kill -INT "$APID" 2>/dev/null || true
wait "$APID" 2>/dev/null || true
grep -E 'events_60s=' /tmp/obs-oh.log | tail -3 || true
