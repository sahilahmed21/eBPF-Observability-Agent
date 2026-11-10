#!/usr/bin/env bash
# Observer-accuracy gate: eBPF-observed streamed LLM call (TTFB, total, tokens) vs client-side truth.
# Mock LLM over TLS (stdlib ssl -> libssl), no application changes. Needs root + BPF (WSL2 ok).
# Usage: sudo ./scripts/correctness-llm.sh   (N=30 calls, TOL_PCT=5, MIN_MS=3 absolute slack)
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
SERVER="$ROOT/testdata/mock-llm-server.py"
PROBE="$ROOT/testdata/llm-probe.py"
N="${N:-30}"
TOL_PCT="${TOL_PCT:-5}"
MIN_MS="${MIN_MS:-3}"
PORT=18450
CERT=/tmp/obsagent-llm.crt
KEY=/tmp/obsagent-llm.key
AGENT_LOG=/tmp/obs-llm-agent.log
TRUTH=/tmp/obs-llm-truth.jsonl

[ -x "$BIN" ] && [ -f "$SERVER" ] && [ -f "$PROBE" ]
pkill -f mock-llm-server.py 2>/dev/null || true
pkill -f release/obsagent 2>/dev/null || true
sleep 0.5

openssl req -x509 -newkey rsa:2048 -keyout "$KEY" -out "$CERT" -days 1 -nodes -subj "/CN=localhost" >/dev/null 2>&1
PORT=$PORT TLS_CERT=$CERT TLS_KEY=$KEY python3 "$SERVER" >/tmp/obs-llm-server.log 2>&1 &
server_pid=$!
sleep 1

OBSAGENT_HEADLESS=1 OBSAGENT_K8S=0 RUST_LOG=info \
  OBSAGENT_COMM_DENY=dockerd,containerd,kubelet,kube-proxy,otelcol,otelcol-contrib \
  "$BIN" >"$AGENT_LOG" 2>&1 &
agent_pid=$!
for _ in $(seq 1 120); do grep -q "obsagent ready" "$AGENT_LOG" && break; sleep 1; done
grep -q "obsagent ready" "$AGENT_LOG" || { echo "FAIL: agent never ready"; exit 1; }
sleep 1

python3 "$PROBE" --port "$PORT" --repeat "$N" >"$TRUTH"
sleep 2
kill -INT "$agent_pid" 2>/dev/null || true
wait "$agent_pid" 2>/dev/null || true
kill "$server_pid" 2>/dev/null || true

python3 - "$TRUTH" "$AGENT_LOG" "$TOL_PCT" "$MIN_MS" <<'PY'
import json, re, sys
truth = [json.loads(l) for l in open(sys.argv[1])]
tol, slack = float(sys.argv[3]) / 100, float(sys.argv[4])
obs = []
for l in open(sys.argv[2]):
    if "llm_call" in l and "path=/v1/messages" in l:
        obs.append(dict(re.findall(r"(\w+)=(\S+)", l)))
print(f"truth={len(truth)} observed={len(obs)}")
if len(obs) != len(truth):
    sys.exit(f"FAIL: observed {len(obs)} calls, expected {len(truth)}")
worst = {"ttfb": 0.0, "total": 0.0}
bad = 0
for t, o in zip(truth, obs):
    for k in ("ttfb", "total"):
        tv, ov = t[f"{k}_ms"], float(o[f"{k}_ms"])
        err = abs(ov - tv)
        worst[k] = max(worst[k], err / tv)
        if err > max(tv * tol, slack):
            bad += 1
            print(f"  {k}: truth={tv:.1f}ms observed={ov:.1f}ms")
    if o["in_tokens"] != f"Some({t['in']})" or o["out_tokens"] != f"Some({t['out']})" or o["complete"] != "true":
        bad += 1
        print(f"  tokens/complete mismatch: truth in={t['in']} out={t['out']} obs={o['in_tokens']} {o['out_tokens']} {o['complete']}")
print(f"worst relative error: ttfb={worst['ttfb']*100:.2f}% total={worst['total']*100:.2f}%")
if bad:
    sys.exit(f"FAIL: {bad} mismatches")
print("PASS: observer within tolerance, tokens exact")
PY
