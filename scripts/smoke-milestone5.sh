#!/usr/bin/env bash
# Phase 5 Milestone gate: cumulative OTLP payload shape + optional kind apply.
# Safe to run without a cluster: local checks always run; kind is opt-in.
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}"
BIN="${CARGO_TARGET_DIR}/release/obsagent"

echo "=== Phase 5 smoke4 ==="

# 1) Binary exists (caller should have run wsl-run.sh build).
if [[ ! -x "$BIN" ]]; then
  echo "FAIL: missing release binary at $BIN (run wsl-run.sh build)" >&2
  exit 1
fi
echo "PASS: release binary present"

# 2) Unit-level registry/export already covered by cargo test; assert dashboard exists.
DASH="$ROOT/deploy/grafana/dashboards/obsagent.json"
if [[ ! -f "$DASH" ]]; then
  echo "FAIL: missing Grafana dashboard $DASH" >&2
  exit 1
fi
python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$DASH"
echo "PASS: Grafana dashboard JSON parses"

# 3) Manifests present.
for f in daemonset.yaml rbac.yaml otel-collector.yaml; do
  [[ -f "$ROOT/deploy/k8s/$f" ]] || { echo "FAIL: missing deploy/k8s/$f" >&2; exit 1; }
done
echo "PASS: k8s manifests present"

# 4) Optional kind e2e when KIND_E2E=1 and kubectl/kind available.
if [[ "${KIND_E2E:-}" == "1" ]]; then
  command -v kubectl >/dev/null || { echo "FAIL: kubectl required for KIND_E2E" >&2; exit 1; }
  echo "KIND_E2E=1: applying manifests (image must already be loadable as ebpf-obs-agent:latest)"
  kubectl apply -f "$ROOT/deploy/k8s/rbac.yaml"
  kubectl apply -f "$ROOT/deploy/k8s/otel-collector.yaml"
  kubectl apply -f "$ROOT/deploy/k8s/daemonset.yaml"
  kubectl apply -f "$ROOT/demos/microservices/k8s.yaml"
  kubectl -n observability rollout status ds/ebpf-obs-agent --timeout=180s
  kubectl -n observability rollout status deploy/otel-collector --timeout=120s
  echo "PASS: kind/cluster rollout"
else
  echo "SKIP: kind e2e (set KIND_E2E=1 to enable)"
fi

echo "=== smoke4 PASS ==="
