#!/usr/bin/env bash
# Claim-lock / M10 gate hygiene (review fixes). Exit 0 only when invariants hold.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLAIM="$ROOT/docs/handoff/SESSION-VISION-95.md"
README="$ROOT/README.md"
RESUME="$ROOT/docs/interview/RESUME_AND_STORY.md"
E2E="$ROOT/scripts/e2e-k3s.sh"
fail=0

fail_msg() {
  echo "FAIL: $1" >&2
  fail=1
}

# --- Public claims must not affirm unproven VISION-95 outcomes ---
affirmative_claim() {
  local f="$1"
  # Affirmative "we achieved X" — not "do not claim X".
  if grep -qiE '(achieves|with|at)\s+(under|&lt;|<)\s*2%\s*CPU' "$f" 2>/dev/null; then
    return 0
  fi
  if grep -qE 'under 2% CPU overhead' "$f"; then
    return 0
  fi
  if grep -qiE 'live service map purely from kernel' "$f"; then
    return 0
  fi
  return 1
}

if affirmative_claim "$README"; then
  fail_msg "README still ships affirmative unproven VISION-95 claim"
fi
if [[ -f "$RESUME" ]] && affirmative_claim "$RESUME"; then
  fail_msg "RESUME_AND_STORY still ships affirmative unproven VISION-95 claim"
fi

# Row 4 must keep screenshot contract (not diluted "stems").
if grep -q 'Grafana stems' "$CLAIM"; then
  fail_msg "claim lock row 4 diluted to 'Grafana stems' (must be screenshot or incomplete)"
fi

# Every table row 1-10: status cell must not be bare **PASS** (HEAD evidence required).
# HISTORICAL / INCOMPLETE / BLOCKED / PARTIAL are allowed; **PASS** alone is not.
while IFS= read -r line; do
  [[ "$line" =~ ^\|[[:space:]]*([0-9]+)[[:space:]]\| ]] || continue
  row="${BASH_REMATCH[1]}"
  (( row >= 1 && row <= 10 )) || continue
  # 4th column = PASS status (split on |)
  status="$(echo "$line" | awk -F'|' '{print $4}')"
  if echo "$status" | grep -qE '\*\*PASS\*\*'; then
    if ! echo "$status" | grep -qE 'HISTORICAL|INCOMPLETE|BLOCKED|PARTIAL'; then
      fail_msg "claim lock row $row is bare **PASS** without HISTORICAL/artifact qualifier"
    fi
  fi
done < "$CLAIM"

# Row 5 PASS requires a non-skipped e2e artifact (binds ALLOW_SKIP out of claim path).
row5="$(grep -E '^\| 5 \|' "$CLAIM" || true)"
if echo "$row5" | grep -qE '\*\*PASS\*\*' && ! echo "$row5" | grep -qE 'BLOCKED|INCOMPLETE|HISTORICAL'; then
  art="$ROOT/docs/handoff/artifacts/e2e-k3s.pass"
  if [[ ! -f "$art" ]]; then
    fail_msg "claim lock row 5 PASS requires docs/handoff/artifacts/e2e-k3s.pass (no ALLOW_SKIP soft-pass)"
  fi
fi

# Score line must not claim 10/10 while any row is BLOCKED/INCOMPLETE/HISTORICAL/PARTIAL.
if grep -qE 'Score: 10/10' "$CLAIM"; then
  if grep -qE '\*\*(BLOCKED|INCOMPLETE|HISTORICAL|PARTIAL)\*\*' "$CLAIM"; then
    fail_msg "claim lock Score 10/10 while non-PASS rows remain"
  fi
fi

# e2e-k3s: no hardcoded WSL path as default ROOT (caller/wsl-run may set OBSAGENT_ROOT).
if grep -qE 'OBSAGENT_ROOT:-/mnt/c/' "$E2E"; then
  fail_msg "e2e-k3s.sh hardcodes /mnt/c default ROOT"
fi
# wsl-run copies e2e to /tmp — must export OBSAGENT_ROOT so ROOT is not /.
if grep -q 'k3s-e2e)' "$ROOT/scripts/wsl-run.sh"; then
  if ! grep -A6 'k3s-e2e)' "$ROOT/scripts/wsl-run.sh" | grep -q 'OBSAGENT_ROOT='; then
    fail_msg "wsl-run.sh k3s-e2e does not set OBSAGENT_ROOT (ROOT=/ when script is in /tmp)"
  fi
fi

# Soft skip must require ALLOW_SKIP=1.
if grep -A5 '^skip()' "$E2E" | grep -q 'exit 0'; then
  if ! grep -A5 '^skip()' "$E2E" | grep -q 'ALLOW_SKIP'; then
    fail_msg "e2e-k3s skip() exits 0 without ALLOW_SKIP gate"
  fi
fi

# Privileged patch must restore; restore must not be best-effort-only.
if grep -q 'privileged","value":true' "$E2E"; then
  if ! grep -q 'PRIV_PATCHED' "$E2E"; then
    fail_msg "e2e-k3s privileged patch has no PRIV_PATCHED restore"
  fi
  # Restore path must surface failure (RESTORE_FAILED or exit 1), not only || true.
  if ! grep -q 'RESTORE_FAILED' "$E2E"; then
    fail_msg "e2e-k3s privileged restore has no RESTORE_FAILED fail-closed path"
  fi
fi

# Agent image must fail closed (no WARN-and-continue on missing/failed import).
if grep -q 'WARN: docker image ebpf-obs-agent:latest missing' "$E2E"; then
  fail_msg "e2e-k3s soft-continues when agent image missing (must fail closed)"
fi
if grep -q 'WARN: k3s ctr import failed' "$E2E"; then
  fail_msg "e2e-k3s soft-continues when k3s ctr import fails (must fail closed)"
fi
if ! grep -q 'import_agent_image' "$E2E"; then
  fail_msg "e2e-k3s missing import_agent_image preflight"
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi
echo "PASS: claim-lock hygiene"
