#!/usr/bin/env bash
# Phase E gate: detection-only policy audit (openat/execve) on labeled scenarios.
# Violations are HARMLESS canaries (reads of /tmp/obs-canary files, exec of copies of /bin/true).
# Reports detected/expected and unexpected violations (false positives). Needs root + BPF.
set -euo pipefail

BIN="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}/release/obsagent"
LOG=/tmp/obs-policy-agent.log
RULES=/tmp/obs-policy.rules
CANARY=/tmp/obs-canary
[ -x "$BIN" ]
pkill -f release/obsagent 2>/dev/null || true
sleep 0.5

rm -rf "$CANARY" /tmp/obsbanned-4 /tmp/obsbanned-5
mkdir -p "$CANARY"
for i in 1 2 3; do echo "canary $i" > "$CANARY/secret-$i.txt"; done
cp /bin/true /tmp/obsbanned-4; cp /bin/true /tmp/obsbanned-5
cat > "$RULES" <<EOF
deny_open $CANARY
deny_exec obsbanned-4
deny_exec obsbanned-5
EOF

OBSAGENT_HEADLESS=1 OBSAGENT_K8S=0 OBSAGENT_FILEOPS=1 OBSAGENT_POLICY="$RULES" RUST_LOG=warn \
  OBSAGENT_COMM_DENY=dockerd,containerd,kubelet,kube-proxy,otelcol,otelcol-contrib \
  "$BIN" >"$LOG" 2>&1 &
agent_pid=$!
for _ in $(seq 1 120); do grep -q "obsagent ready" "$LOG" && break; sleep 1; done
grep -q "obsagent ready" "$LOG" || { echo "FAIL: agent never ready"; exit 1; }
sleep 1

# Benign scenarios (must produce no violation).
cat /etc/hostname >/dev/null
ls /tmp >/dev/null
python3 -c "open('/etc/passwd').read()"
/bin/true
sh -c 'echo ok' >/dev/null
# Violation scenarios (expected: one detection each).
cat "$CANARY/secret-1.txt" >/dev/null                      # 1 open via cat
python3 -c "open('$CANARY/secret-2.txt').read()"            # 2 open via python
head -c1 "$CANARY/secret-3.txt" >/dev/null                  # 3 open via head
/tmp/obsbanned-4                                            # 4 exec direct
sh -c '/tmp/obsbanned-5'                                    # 5 exec via shell
sleep 2
kill -INT "$agent_pid" 2>/dev/null || true
wait "$agent_pid" 2>/dev/null || true

expected=0; detected=0
for m in secret-1 secret-2 secret-3 obsbanned-4 obsbanned-5; do
  expected=$((expected + 1))
  if grep -q "policy_violation.*$m" "$LOG"; then detected=$((detected + 1)); else echo "MISSED: $m"; fi
done
fp=$(grep "policy_violation" "$LOG" | grep -vc -e "obs-canary" -e "obsbanned" || true)
total=$(grep -c "policy_violation" "$LOG" || true)
echo "detected=$detected/$expected violations_total=$total unexpected(false positives)=$fp benign_scenarios=5"
[ "$detected" -eq "$expected" ] && [ "$fp" -eq 0 ] && echo "PASS" || { echo "FAIL"; exit 1; }
