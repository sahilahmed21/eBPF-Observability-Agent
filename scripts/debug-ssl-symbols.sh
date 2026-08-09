#!/bin/bash
set -euo pipefail
SSLSO=$(python3 -c 'import _ssl; print(_ssl.__file__)')
echo "sslso=$SSLSO"
ldd "$SSLSO" | grep -i ssl || true
echo '--- nm _ssl ---'
nm -D "$SSLSO" | grep -E 'SSL_(write|read|set_fd)' | head -40 || true
echo '--- curl ---'
ldd /usr/bin/curl | grep -i ssl || true
echo '--- libssl exports ---'
nm -D /lib/x86_64-linux-gnu/libssl.so.3 | grep -E ' SSL_write$| SSL_write_ex$| SSL_set_fd$| SSL_read$| SSL_read_ex$' || true
