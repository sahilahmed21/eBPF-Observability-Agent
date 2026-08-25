#!/usr/bin/env bash
# P10-Q2: a tgid must exist in the proc tree the agent mounts.
# Pass the BPF/workload tgid as argv1 or BPF_TGID. With no argument this
# checks /proc/self (local smoke only — e2e-k3s.sh passes a frontend tgid).
set -euo pipefail

PROC_ROOT="${OBSAGENT_PROC_ROOT:-}"
if [[ -z "$PROC_ROOT" ]]; then
  if [[ -d /host/proc ]]; then
    PROC_ROOT=/host/proc
  else
    PROC_ROOT=/proc
  fi
fi

TGID="${1:-${BPF_TGID:-}}"
if [[ -z "$TGID" ]]; then
  TGID="$(awk '/^Pid:/{print $2; exit}' /proc/self/status || true)"
fi
if [[ -z "$TGID" ]]; then
  echo "FAIL: P10-Q2 no tgid (pass argv1 or BPF_TGID)" >&2
  exit 1
fi

if [[ ! -e "$PROC_ROOT/$TGID/ns/pid" ]] && [[ ! -r "$PROC_ROOT/$TGID/cgroup" ]] && [[ ! -r "$PROC_ROOT/$TGID/status" ]]; then
  echo "FAIL: P10-Q2 $PROC_ROOT/$TGID not visible (BPF tgid missing from mounted proc)" >&2
  exit 1
fi
echo "PASS: P10-Q2 $PROC_ROOT/$TGID visible"
