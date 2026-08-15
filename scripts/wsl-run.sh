#!/usr/bin/env bash
set -euo pipefail
# Prefer the build user's cargo when running as root (Q11).
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
elif [ -f /home/sahil/.cargo/env ]; then
  # shellcheck source=/dev/null
  source /home/sahil/.cargo/env
fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/sahil/.cache/obsagent-target}"
cd /mnt/c/projects/eBPF-Observability-Agent

strip() { python3 -c "import sys; open(sys.argv[2],'wb').write(open(sys.argv[1],'rb').read().replace(b'\r',b''))" "$1" "$2"; }

case "${1:-}" in
  preflight)
    strip scripts/preflight.sh /tmp/obs-preflight.sh
    bash /tmp/obs-preflight.sh
    ;;
  build)
    cargo build --release
    ;;
  test-common)
    # .cargo/config.toml sets runner=sudo -E (for agent load). Unit tests must not use it.
    cargo test -p obsagent-common --features user --config 'target."cfg(all())".runner="env"'
    ;;
  test-agent)
    cargo test -p obsagent --config 'target."cfg(all())".runner="env"'
    ;;
  smoke0)
    strip scripts/smoke-milestone0.sh /tmp/obs-smoke0.sh
    # Q11: privileged path is wsl -d Ubuntu -u root (caller should be root already).
    bash /tmp/obs-smoke0.sh
    ;;
  smoke1)
    strip scripts/smoke-milestone1.sh /tmp/obs-smoke1.sh
    bash /tmp/obs-smoke1.sh
    ;;
  smoke2)
    strip scripts/smoke-milestone2.sh /tmp/obs-smoke2.sh
    bash /tmp/obs-smoke2.sh
    ;;
  smoke3)
    strip scripts/smoke-milestone3.sh /tmp/obs-smoke3.sh
    bash /tmp/obs-smoke3.sh
    ;;
  correctness2)
    strip scripts/correctness-phase2.sh /tmp/obs-correctness2.sh
    bash /tmp/obs-correctness2.sh
    ;;
  correctness3)
    strip scripts/correctness-phase3.sh /tmp/obs-correctness3.sh
    bash /tmp/obs-correctness3.sh
    ;;
  smoke4)
    strip scripts/smoke-milestone5.sh /tmp/obs-smoke4.sh
    bash /tmp/obs-smoke4.sh
    ;;
  *)
    echo "usage: $0 {preflight|build|test-common|test-agent|smoke0|smoke1|smoke2|smoke3|smoke4|correctness2|correctness3}" >&2
    exit 2
    ;;
esac
