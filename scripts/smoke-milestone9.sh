#!/usr/bin/env bash
# Milestone 9: sampled OTLP /v1/traces 2xx + Grafana stems + headless counters.
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
LOG=/tmp/obs-smoke9.log
SINK_LOG=/tmp/obs-otlp-sink9.log
SINK_PORT=14318
HTTP_PORT=18097

if [ "$(id -u)" -eq 0 ]; then PRIV=(env); else PRIV=(sudo -E); fi

[ -x "$BIN" ] || { echo "FAIL: $BIN not built"; exit 1; }

DASH="$ROOT/deploy/grafana/dashboards/obsagent.json"
python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$DASH"
python3 - "$DASH" <<'PY'
import sys
dash = open(sys.argv[1]).read()
for name in (
    "http_client_duration_milliseconds_bucket",
    "http_client_requests_total",
    "tls_handshake_duration_milliseconds_bucket",
    "obsagent_events_dropped_total",
    "obsagent_traces_sampled_total",
    "obsagent_traces_not_sampled_total",
    "obsagent_traces_dropped_total",
    "obsagent_traces_export_failed_total",
    "http_protocol",
):
    if name not in dash:
        raise SystemExit(f"FAIL: dashboard missing {name}")
print("PASS: Grafana scrape names present")
PY

pkill -f "obsotlpsink9" 2>/dev/null || true
pkill -f "/release/obsagent" 2>/dev/null || true
rm -f "$SINK_LOG" "$LOG" /tmp/obs-otlp-traces9.ok /tmp/obs-otlp-metrics9.ok

# Distinct comm so deny-list can drop collector I/O without a port denylist
# and without hiding the python3 /slow probe.
SINK_BIN=/tmp/obsotlpsink9
cp "$(command -v python3)" "$SINK_BIN"
chmod +x "$SINK_BIN"

"$SINK_BIN" - "$SINK_PORT" "$SINK_LOG" <<'PY' &
import os, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

port = int(sys.argv[1])
log_path = sys.argv[2]

class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(n)
        with open(log_path, "ab") as f:
            f.write(f"{self.path} {n}\n".encode())
        if b"resourceSpans" in body or self.path.endswith("/v1/traces"):
            open("/tmp/obs-otlp-traces9.ok", "w").write("ok")
        if b"resourceMetrics" in body or self.path.endswith("/v1/metrics"):
            open("/tmp/obs-otlp-metrics9.ok", "w").write("ok")
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"{}")

    def log_message(self, *_args):
        pass

print("obs-otlp-sink9 listening", flush=True)
HTTPServer(("127.0.0.1", port), H).serve_forever()
PY
sink_pid=$!

python3 - "$HTTP_PORT" <<'PY' &
from http.server import BaseHTTPRequestHandler, HTTPServer
import sys

port = int(sys.argv[1])

class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")

    def log_message(self, *_args):
        pass

HTTPServer(("127.0.0.1", port), H).serve_forever()
PY
http_pid=$!

cleanup() {
  if [ -n "${runner:-}" ]; then
    kill -INT "$runner" 2>/dev/null || true
    wait "$runner" 2>/dev/null || true
    runner=""
  fi
  kill "$sink_pid" "$http_pid" 2>/dev/null || true
  wait "$sink_pid" 2>/dev/null || true
  wait "$http_pid" 2>/dev/null || true
}
trap cleanup EXIT
sleep 0.4

OBSAGENT_HEADLESS=1 RUST_LOG=warn timeout --signal=INT 25 \
  "${PRIV[@]}" env \
  OBSAGENT_HEADLESS=1 \
  RUST_LOG=warn \
  OBSAGENT_OTLP=1 \
  OBSAGENT_TRACE_SAMPLE=1 \
  OBSAGENT_COMM_DENY=obsotlpsink9 \
  OTEL_EXPORTER_OTLP_ENDPOINT="http://127.0.0.1:${SINK_PORT}" \
  "$BIN" >"$LOG" 2>&1 &
runner=$!

for i in $(seq 1 40); do
  grep -q "obsagent ready" "$LOG" 2>/dev/null && break
  sleep 0.25
done
grep -q "obsagent ready" "$LOG" || { echo "FAIL: agent did not become ready"; cat "$LOG"; exit 1; }

python3 - "$HTTP_PORT" <<'PY'
import sys, urllib.request
port = int(sys.argv[1])
for _ in range(5):
    urllib.request.urlopen(f"http://127.0.0.1:{port}/slow", timeout=2).read()
print("probe ok")
PY

# Export tick is 10s (first tokio interval fires immediately, empty).
sleep 12
kill -INT "$runner" 2>/dev/null || true
wait "$runner" 2>/dev/null || true
runner=""

echo "=== tail agent ==="
tail -n 30 "$LOG"
echo "=== sink ==="
cat "$SINK_LOG" 2>/dev/null || true

grep -q "trace_s=" "$LOG" || { echo "FAIL: no trace_s in headless"; exit 1; }
python3 - "$LOG" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
hits = [int(m.group(1)) for m in re.finditer(r"trace_s=(\d+)", text)]
if not hits or max(hits) < 1:
    raise SystemExit("FAIL: trace_s never > 0")
print(f"PASS: trace_s={max(hits)}")
body = "\n".join(
    l for l in text.splitlines() if l.startswith("  ")
)
m = re.search(r"obsagent ready pid=(\d+)", text)
if not m:
    raise SystemExit("FAIL: ready line missing pid=")
pid = m.group(1)
if re.search(rf"proc:obsagent:{pid}\b", body) or re.search(
    rf"\[edge\] proc:[^ ]+:{pid} ", body
):
    raise SystemExit(f"FAIL: agent tgid {pid} appeared as HTTP/map src")
if re.search(r"POST /v1/(traces|metrics) ", body):
    raise SystemExit("FAIL: OTLP export HTTP ingested as application rows")
if "127.0.0.1:14318" in body:
    raise SystemExit("FAIL: collector socket ingested as TCP/HTTP")
print(f"PASS: self tgid {pid} and OTLP export rows excluded")
PY

[ -f /tmp/obs-otlp-traces9.ok ] || {
  echo "FAIL: sink did not receive /v1/traces"
  exit 1
}
echo "PASS: /v1/traces 2xx"
echo "PASS: milestone 9 traces export"
