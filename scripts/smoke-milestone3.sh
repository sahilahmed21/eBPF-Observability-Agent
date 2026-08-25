#!/usr/bin/env bash
# Milestone 3 gate: OpenSSL uprobes attach; TlsIo + HTTP rows over HTTPS; clean unload.
set -euo pipefail

ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}"
BIN="${CARGO_TARGET_DIR:-target}/release/obsagent"
SERVER="$ROOT/testdata/https-latency-server.py"
PROBE="$ROOT/testdata/https-probe.py"
LOG=$(mktemp)
PORT=18443
CERT=/tmp/obsagent-tls.crt
KEY=/tmp/obsagent-tls.key

if [ "$(id -u)" -eq 0 ]; then PRIV=(env); else PRIV=(sudo -E); fi
loaded_named() { "${PRIV[@]}" bpftool prog list | grep -c " name ${1} " || true; }

[ -x "$BIN" ] || { echo "FAIL: $BIN not built"; exit 1; }
[ -f "$SERVER" ] && [ -f "$PROBE" ] || { echo "FAIL: python HTTPS fixtures missing"; exit 1; }

# Q9: CPython ssl uses system OpenSSL.
python3 -c "import ssl,sys; print(ssl.OPENSSL_VERSION); sys.exit(0 if 'OpenSSL' in ssl.OPENSSL_VERSION else 1)" \
  || { echo "FAIL: python ssl not OpenSSL"; exit 1; }
echo "PASS: python ssl uses OpenSSL (Q9)"

openssl req -x509 -newkey rsa:2048 -keyout "$KEY" -out "$CERT" -days 1 -nodes \
  -subj "/CN=localhost" >/dev/null 2>&1

[ "$(loaded_named exit_ssl_write)" -eq 0 ] || {
  echo "FAIL: exit_ssl_write already loaded"; exit 1;
}

PORT=$PORT TLS_CERT=$CERT TLS_KEY=$KEY python3 "$SERVER" >/tmp/obs-https-server.log 2>&1 &
server_pid=$!
cleanup() {
  kill "$server_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true
}
trap cleanup EXIT
sleep 0.5
grep -q listening /tmp/obs-https-server.log || {
  echo "FAIL: https server did not listen"; cat /tmp/obs-https-server.log; exit 1;
}

OBSAGENT_HEADLESS=1 OBSAGENT_SMOKE_PROBE=1 RUST_LOG=warn timeout --signal=INT 25 \
  "${PRIV[@]}" env OBSAGENT_HEADLESS=1 OBSAGENT_SMOKE_PROBE=1 RUST_LOG=warn "$BIN" >"$LOG" 2>&1 &
runner=$!
for i in $(seq 1 40); do
  grep -q "obsagent ready" "$LOG" 2>/dev/null && break
  sleep 0.25
done
grep -q "obsagent ready" "$LOG" || { echo "FAIL: agent did not become ready"; cat "$LOG"; exit 1; }

[ "$(loaded_named exit_ssl_write_ex)" -ge 1 ] || {
  echo "FAIL: exit_ssl_write_ex not loaded (Q15 / OpenSSL 3)"; cat "$LOG"; exit 1;
}
[ "$(loaded_named enter_ssl_set_fd)" -ge 1 ] || {
  echo "FAIL: enter_ssl_set_fd not loaded (Q15)"; cat "$LOG"; exit 1;
}
echo "PASS: OpenSSL uprobes loaded (Q15)"

python3 "$PROBE" --port "$PORT" --repeat 5 \
  --path /fast --path /users/123 --path '/slow?delay_ms=20' || {
  echo "FAIL: https_probe errors"; cat /tmp/obs-https-server.log; exit 1;
}
sleep 3

grep -q 'tlsio=[1-9]' "$LOG" || {
  echo "FAIL: no tlsio events"; cat "$LOG"; exit 1;
}
echo "PASS: tlsio events observed"

grep -E -q 'GET /(fast|users/:id|slow)' "$LOG" || {
  echo "FAIL: no HTTP endpoint rows"; cat "$LOG"; exit 1;
}
echo "PASS: HTTP endpoint rows observed"

wait "$runner" || true

[ "$(loaded_named exit_ssl_write)" -eq 0 ] || {
  echo "FAIL: still loaded after exit"; exit 1;
}
echo "PASS: clean unload"

[ -z "$(ls -A /sys/fs/bpf 2>/dev/null)" ] || {
  echo "FAIL: leftover pins in /sys/fs/bpf"; exit 1;
}
echo "PASS: no leaked pins"
