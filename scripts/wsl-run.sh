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
    # default-members are only agent+common; gates need testdata bins too.
    cargo build --release \
      -p obsagent -p obsagent-common \
      -p latency-server -p http-probe -p writev-server -p grpc-slow
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
  smoke6)
    strip scripts/smoke-milestone6.sh /tmp/obs-smoke6.sh
    bash /tmp/obs-smoke6.sh
    ;;
  correctness6)
    strip scripts/correctness-phase6.sh /tmp/obs-correctness6.sh
    bash /tmp/obs-correctness6.sh
    ;;
  smoke7)
    strip scripts/smoke-milestone7.sh /tmp/obs-smoke7.sh
    bash /tmp/obs-smoke7.sh
    ;;
  correctness7-grpc)
    strip scripts/correctness-phase7-grpc.sh /tmp/obs-correctness7-grpc.sh
    bash /tmp/obs-correctness7-grpc.sh
    ;;
  correctness7-h2-tls)
    strip scripts/correctness-phase7-h2-tls.sh /tmp/obs-correctness7-h2-tls.sh
    bash /tmp/obs-correctness7-h2-tls.sh
    ;;
  correctness8-dual)
    strip scripts/correctness-phase8-dual.sh /tmp/obs-correctness8-dual.sh
    bash /tmp/obs-correctness8-dual.sh
    ;;
  correctness12)
    strip scripts/correctness-phase12.sh /tmp/obs-correctness12.sh
    bash /tmp/obs-correctness12.sh
    ;;
  smoke9)
    strip scripts/smoke-milestone9.sh /tmp/obs-smoke9.sh
    bash /tmp/obs-smoke9.sh
    ;;
  pid-ns)
    strip scripts/check-pid-ns.sh /tmp/obs-pid-ns.sh
    bash /tmp/obs-pid-ns.sh
    ;;
  k3s-e2e)
    strip scripts/e2e-k3s.sh /tmp/obs-e2e-k3s.sh
    # Soft skip only when ALLOW_SKIP=1; otherwise missing cluster is FAIL.
    # Script is copied to /tmp — must not derive ROOT from BASH_SOURCE.
    OBSAGENT_ROOT="${OBSAGENT_ROOT:-/mnt/c/projects/eBPF-Observability-Agent}" \
      bash /tmp/obs-e2e-k3s.sh
    ;;
  overhead6)
    strip scripts/overhead-phase6.sh /tmp/obs-overhead6.sh
    bash /tmp/obs-overhead6.sh
    ;;
  overhead-vision95)
    strip scripts/overhead-vision95.sh /tmp/obs-overhead-vision95.sh
    bash /tmp/obs-overhead-vision95.sh
    ;;
  overload-vision95)
    strip scripts/overload-vision95.sh /tmp/obs-overload-vision95.sh
    bash /tmp/obs-overload-vision95.sh
    ;;
  *)
    echo "usage: $0 {preflight|build|test-common|test-agent|smoke0|smoke1|smoke2|smoke3|smoke4|smoke6|smoke7|smoke9|correctness2|correctness3|correctness6|correctness7-grpc|correctness7-h2-tls|correctness8-dual|correctness12|overhead6|overhead-vision95|overload-vision95|pid-ns|k3s-e2e}" >&2
    exit 2
    ;;
esac
