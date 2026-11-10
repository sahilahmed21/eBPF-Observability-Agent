#!/usr/bin/env bash
# Cross-check obsagent's gen_ai spans against the Atlas gateway's own per-step records (SQLite).
# Run as root inside WSL2 (BPF). Stand-in provider: testdata/mock-llm-server.py over TLS (OpenAI-style SSE).
#
# Prereqs (once):
#   sudo apt-get install -y python3-venv python3-pip
#   python3 -m venv ~/atlas-venv && ~/atlas-venv/bin/pip install fastapi "uvicorn[standard]" httpx pyyaml \
#     prometheus-client opentelemetry-api opentelemetry-sdk opentelemetry-exporter-otlp-proto-http python-multipart
#   ./scripts/wsl-run.sh build            # release obsagent
# Run:  sudo ATLAS=/mnt/c/projects/Atlas ./scripts/atlas-crosscheck.sh [steps] [first_token_ms]
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
ATLAS="${ATLAS:-/mnt/c/projects/Atlas}"
VENV="${VENV:-/home/sahil/atlas-venv}"
BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
STEPS="${1:-30}"
FIRST_TOKEN_MS="${2:-0}"
UP_TLS="${UP_TLS:-0}"   # 0 = cleartext upstream (Atlas default); 1 = TLS upstream (see docs/overhead.md: not captured)
W=/tmp/atlas-xc
UP_PORT=18450; GW_PORT=18080; SINK_PORT=14318

pkill -x obsagent 2>/dev/null || true
rm -rf "$W"; mkdir -p "$W"
cd "$W"

openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem -days 1 \
  -subj "/CN=127.0.0.1" -addext "subjectAltName=IP:127.0.0.1" 2>/dev/null
cat > tenants.yaml <<EOF
tenants:
  - id: xc
    api_key: sk-xc
    rpm_limit: 100000
    record_allowed: true
    allowed_models: [mock-model]
EOF
cat > workers.yaml <<EOF
workers:
  - id: w1
    model: mock-model
    base_url: $([ "$UP_TLS" = 1 ] && echo https || echo http)://127.0.0.1:$UP_PORT/v1
EOF

# OTLP sink: stores "<path> <body>" lines for atlas-join.py
cat > sink.py <<'PY'
import http.server
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        b = self.rfile.read(int(self.headers.get('content-length', 0)))
        open('/tmp/atlas-xc/sink.log', 'ab').write(self.path.encode() + b' ' + b + b'\n')
        self.send_response(200); self.send_header('content-length', '2'); self.end_headers(); self.wfile.write(b'{}')
    def log_message(self, *a): pass
http.server.HTTPServer(('127.0.0.1', 14318), H).serve_forever()
PY
python3 sink.py & sink_pid=$!

TLS=$UP_TLS PORT=$UP_PORT TLS_CERT=cert.pem TLS_KEY=key.pem OAI_FIRST_TOKEN_MS="$FIRST_TOKEN_MS" \
  python3 "$ROOT/testdata/mock-llm-server.py" >up.log 2>&1 & up_pid=$!

# SAMPLE_N=1: the gateway keeps one upstream connection alive, and sticky sampling is per fd, so an
# auto-raised sample_n would drop every call at once. Real ring drops are still reported below.
OBSAGENT_SAMPLE_N=1 OBSAGENT_HEADLESS=1 OBSAGENT_K8S=0 RUST_LOG=info OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:$SINK_PORT \
  OBSAGENT_COMM_DENY=dockerd,containerd,kubelet,kube-proxy,otelcol,otelcol-contrib,k3s-server \
  "$BIN" >agent.log 2>&1 & agent_pid=$!
for _ in $(seq 1 120); do grep -q "obsagent ready" agent.log && break; sleep 1; done
grep -q "obsagent ready" agent.log || { echo "FAIL: agent never ready"; exit 1; }

PP="$ATLAS/platform/gateway:$ATLAS/platform/tenant:$ATLAS/platform/registry:$ATLAS/platform/router:$ATLAS/platform/observability:$ATLAS/platform/recorder:$ATLAS/workers"
# SSL_CERT_FILE: httpx (stdlib ssl -> system libssl.so.3) must trust the stand-in provider's cert.
SSL_CERT_FILE=$W/cert.pem PYTHONPATH=$PP ATLAS_TENANTS=$W/tenants.yaml ATLAS_WORKERS=$W/workers.yaml \
  ATLAS_RECORDER_DB=$W/atlas.sqlite "$VENV/bin/uvicorn" app:create_app_from_env --factory \
  --app-dir "$ATLAS/platform/gateway" --port $GW_PORT >gw.log 2>&1 & gw_pid=$!
for _ in $(seq 1 60); do curl -s -o /dev/null "http://127.0.0.1:$GW_PORT/docs" && break; sleep 1; done
sleep 2

# Client: sequential streaming steps across 3 runs; include_usage so the upstream reports tokens.
"$VENV/bin/python" - "$GW_PORT" "$STEPS" <<'PY'
import sys, time, httpx
port, n = sys.argv[1], int(sys.argv[2])
c = httpx.Client(base_url=f"http://127.0.0.1:{port}", timeout=30)
for i in range(n):
    run, step = f"run{i % 3}", i // 3
    body = {"model": "mock-model", "stream": True, "stream_options": {"include_usage": True},
            "messages": [{"role": "user", "content": "hello " * (3 + i)}]}
    with c.stream("POST", "/v1/chat/completions", json=body, headers={
        "Authorization": "Bearer sk-xc", "x-atlas-mode": "record",
        "x-atlas-run-id": run, "x-atlas-step": str(step)}) as r:
        for _ in r.iter_lines():
            pass
        if r.status_code != 200:
            print("step", i, "status", r.status_code)
    time.sleep(0.3)
PY
sleep 12   # agent exports spans every 10 s
kill -INT "$agent_pid" 2>/dev/null || true; wait "$agent_pid" 2>/dev/null || true
kill "$gw_pid" "$up_pid" "$sink_pid" 2>/dev/null || true
echo "agent status: $(grep -E '^events_60s' agent.log | tail -1 | grep -oE 'drops=[0-9]+ sample_n=[0-9]+')"
python3 "$ROOT/scripts/atlas-join.py" "$W/atlas.sqlite" "$W/sink.log" --server-contains ":$UP_PORT"
