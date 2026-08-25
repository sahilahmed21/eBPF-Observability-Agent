#!/usr/bin/env bash
# Phase 10 named-edge gate on a real node (k3s default).
# HTTP frontend → Service ClusterIP `api` must scrape as src=demo/frontend-… dst=demo/api.
# Does not mark Milestone 10 GREEN from this script's presence — only a PASS run does.
#
# Exit codes:
#   0 = named-edge PASS (or ALLOW_SKIP=1 soft skip)
#   1 = FAIL / missing cluster without ALLOW_SKIP
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${OBSAGENT_ROOT:-$(cd "${SCRIPT_DIR}/.." && pwd)}"
if [[ ! -f "$ROOT/deploy/k8s/rbac.yaml" ]]; then
  echo "FAIL: ROOT=$ROOT missing deploy/k8s/rbac.yaml (set OBSAGENT_ROOT to the repo root)" >&2
  exit 1
fi
K3S_E2E="${K3S_E2E:-}"
ALLOW_SKIP="${ALLOW_SKIP:-}"
PRIV_PATCHED=0
RESTORE_FAILED=0
PF_PID=""

cleanup() {
  local rc=$?
  if [[ -n "${PF_PID}" ]]; then
    kill "${PF_PID}" >/dev/null 2>&1 || true
  fi
  if [[ "${PRIV_PATCHED}" == "1" ]]; then
    if ! kubectl -n observability patch ds ebpf-obs-agent --type=json \
      -p='[{"op":"replace","path":"/spec/template/spec/containers/0/securityContext/privileged","value":false}]' \
      >/dev/null 2>&1; then
      echo "FAIL: could not restore DaemonSet privileged=false" >&2
      RESTORE_FAILED=1
    fi
  fi
  if [[ "${RESTORE_FAILED}" == "1" ]]; then
    exit 1
  fi
  exit "$rc"
}
trap cleanup EXIT

skip() {
  echo "SKIP: $1"
  if [[ "${ALLOW_SKIP}" == "1" ]]; then
    exit 0
  fi
  echo "HINT: set ALLOW_SKIP=1 for soft skip, or K3S_E2E=1 / fix kubeconfig for a required gate" >&2
  exit 1
}

if ! command -v kubectl >/dev/null 2>&1; then
  if [[ "$K3S_E2E" == "1" ]]; then
    echo "FAIL: kubectl required when K3S_E2E=1" >&2
    exit 1
  fi
  skip "no kubectl (set K3S_E2E=1 to require a cluster)"
fi
if ! kubectl get nodes >/dev/null 2>&1; then
  if [[ "$K3S_E2E" == "1" ]]; then
    echo "FAIL: kubectl cannot reach a cluster (K3S_E2E=1)" >&2
    echo "HINT: Milestone 10 needs k3s+containerd (not kind-in-Docker). Fix kubeconfig or install k3s." >&2
    exit 1
  fi
  skip "no reachable cluster (need k3s+containerd; kind-in-Docker is not M10)"
fi

echo "=== Phase 10 e2e-k3s ==="
echo "ROOT=$ROOT"
kubectl get nodes -o wide
RUNTIME="$(kubectl get nodes -o jsonpath='{.items[0].status.nodeInfo.containerRuntimeVersion}' 2>/dev/null || true)"
if echo "$RUNTIME" | grep -qi docker; then
  echo "FAIL: runtime is Docker ($RUNTIME). Milestone 10 is k3s/containerd, not kind-in-Docker." >&2
  exit 1
fi

import_agent_image() {
  # Fail closed: M10 evidence must run the image under test, not a stale node cache.
  if command -v docker >/dev/null 2>&1; then
    if ! docker image inspect ebpf-obs-agent:latest >/dev/null 2>&1; then
      echo "FAIL: docker image ebpf-obs-agent:latest missing (build/load before e2e)" >&2
      exit 1
    fi
    if command -v k3s >/dev/null 2>&1; then
      if ! docker save ebpf-obs-agent:latest | sudo k3s ctr images import -; then
        echo "FAIL: k3s ctr import failed for ebpf-obs-agent:latest" >&2
        exit 1
      fi
      return 0
    fi
    return 0
  fi
  if command -v k3s >/dev/null 2>&1; then
    if ! sudo k3s ctr images ls | grep -q 'ebpf-obs-agent'; then
      echo "FAIL: no docker and ebpf-obs-agent not in k3s ctr images" >&2
      exit 1
    fi
    return 0
  fi
  echo "FAIL: cannot verify agent image (need docker and/or k3s ctr)" >&2
  exit 1
}

import_agent_image

kubectl apply -f "$ROOT/deploy/k8s/rbac.yaml"
kubectl apply -f "$ROOT/deploy/k8s/otel-collector.yaml"
kubectl apply -f "$ROOT/deploy/k8s/daemonset.yaml"
kubectl apply -f "$ROOT/demos/microservices/k8s.yaml"

if ! kubectl -n observability rollout status ds/ebpf-obs-agent --timeout=90s; then
  echo "WARN: capability DaemonSet not Ready; retrying privileged:true (restored on EXIT)"
  if ! kubectl -n observability patch ds ebpf-obs-agent --type=json \
    -p='[{"op":"replace","path":"/spec/template/spec/containers/0/securityContext/privileged","value":true}]'; then
    echo "FAIL: privileged patch failed" >&2
    exit 1
  fi
  PRIV_PATCHED=1
  kubectl -n observability rollout status ds/ebpf-obs-agent --timeout=120s
fi
kubectl -n observability rollout status deploy/otel-collector --timeout=180s
kubectl -n demo rollout status deploy/frontend --timeout=240s
kubectl -n demo rollout status deploy/api --timeout=240s

AGENT_POD="$(kubectl -n observability get pod -l app=ebpf-obs-agent -o jsonpath='{.items[0].metadata.name}')"
if [[ -z "$AGENT_POD" ]]; then
  echo "FAIL: no agent pod" >&2
  exit 1
fi

FRONTEND_UID="$(kubectl -n demo get pod -l app=frontend -o jsonpath='{.items[0].metadata.uid}')"
if [[ -z "$FRONTEND_UID" ]]; then
  echo "FAIL: no frontend pod uid" >&2
  exit 1
fi

# P10-Q2: a demo frontend tgid (the workload BPF will see) must exist in /host/proc.
kubectl -n observability exec "$AGENT_POD" -- env FRONTEND_UID="$FRONTEND_UID" /bin/sh -c '
  UID_US=$(echo "$FRONTEND_UID" | tr "-" "_")
  found=
  for d in /host/proc/[0-9]*; do
    [ -r "$d/cgroup" ] || continue
    if grep -q "$FRONTEND_UID" "$d/cgroup" 2>/dev/null || grep -q "$UID_US" "$d/cgroup" 2>/dev/null; then
      found=${d##*/}
      break
    fi
  done
  if [ -z "$found" ]; then
    echo "FAIL: P10-Q2 no /host/proc tgid for frontend uid $FRONTEND_UID" >&2
    exit 1
  fi
  if [ ! -e "/host/proc/$found/ns/pid" ] && [ ! -r "/host/proc/$found/cgroup" ]; then
    echo "FAIL: P10-Q2 /host/proc/$found not visible" >&2
    exit 1
  fi
  echo "PASS: P10-Q2 frontend tgid=$found in /host/proc"
'

echo "waiting for OTLP export + Prometheus scrape (frontend loops every 1s; export every 10s)"
sleep 25

SCRAPE=""
PF_LOG="$(mktemp)"
kubectl -n observability port-forward svc/otel-collector 18889:8889 >"$PF_LOG" 2>&1 &
PF_PID=$!
sleep 2

fetch_scrape() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsS http://127.0.0.1:18889/metrics || true
  else
    python3 -c "import urllib.request; print(urllib.request.urlopen('http://127.0.0.1:18889/metrics', timeout=5).read().decode())" || true
  fi
}

for _ in 1 2 3 4 5 6; do
  SCRAPE="$(fetch_scrape)"
  if echo "$SCRAPE" | grep -q 'dst="demo/api"'; then
    break
  fi
  sleep 10
done

if [[ -z "$SCRAPE" ]]; then
  echo "FAIL: empty collector scrape (port-forward?)" >&2
  cat "$PF_LOG" >&2 || true
  exit 1
fi

EDGE="$(echo "$SCRAPE" | grep 'dst="demo/api"' | grep 'src="demo/frontend' | grep -v 'frontend-tls' | grep -v 'frontend-grpc' || true)"
if [[ -z "$EDGE" ]]; then
  echo "FAIL: scrape missing a line with src=demo/frontend… and dst=demo/api" >&2
  echo "$SCRAPE" | grep -E 'http_client' | head -n 40 >&2 || true
  exit 1
fi
if echo "$SCRAPE" | grep 'dst="demo/api"' | grep 'src="proc:unknown"' >/dev/null; then
  echo "FAIL: demo/api series has src=proc:unknown (PID ns / cgroup join failed)" >&2
  exit 1
fi

echo "PASS: named edge src=demo/frontend… dst=demo/api"
echo "=== e2e-k3s PASS ==="
