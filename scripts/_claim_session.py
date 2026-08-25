#!/usr/bin/env python3
"""Session helper for claim gates. Strip CRLF when copying bash scripts."""
import os
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path("/mnt/c/projects/eBPF-Observability-Agent")
LOGS = ROOT / "docs/handoff/artifacts/logs"
LOGS.mkdir(parents=True, exist_ok=True)
os.environ["KUBECONFIG"] = "/etc/rancher/k3s/k3s.yaml"
os.environ["CARGO_TARGET_DIR"] = "/home/sahil/.cache/obsagent-target"
os.chdir(ROOT)


def sh(cmd: str, check: bool = True) -> int:
    print("+", cmd, flush=True)
    r = subprocess.run(cmd, shell=True, env=os.environ.copy())
    if check and r.returncode != 0:
        raise SystemExit(r.returncode)
    return r.returncode


def strip_copy(src: str, dst: str) -> None:
    data = pathlib.Path(src).read_bytes().replace(b"\r\n", b"\n").replace(b"\r", b"")
    pathlib.Path(dst).write_bytes(data)


def pause_ds() -> None:
    sh(
        "kubectl -n observability get ds ebpf-obs-agent "
        "-o jsonpath='{.spec.template.spec.nodeSelector}' || true",
        check=False,
    )
    # Prefer replace; ignore failure if already paused.
    sh(
        "kubectl -n observability patch ds ebpf-obs-agent --type merge "
        "-p '{\"spec\":{\"template\":{\"spec\":{\"nodeSelector\":{\"obsagent-pause\":\"true\"}}}}}'",
        check=False,
    )
    sh("kubectl -n observability delete pod -l app=ebpf-obs-agent --wait=false", check=False)
    for _ in range(30):
        out = subprocess.check_output(
            "kubectl -n observability get pods -l app=ebpf-obs-agent --no-headers 2>/dev/null | wc -l",
            shell=True,
            text=True,
        ).strip()
        if out == "0":
            print("agent pods gone", flush=True)
            return
        time.sleep(2)
    print("WARN pods remain", flush=True)


def resume_ds() -> None:
    sh(
        "kubectl -n observability patch ds ebpf-obs-agent --type json "
        "-p '[{\"op\":\"remove\",\"path\":\"/spec/template/spec/nodeSelector\"}]'",
        check=False,
    )
    sh("kubectl -n observability rollout status ds/ebpf-obs-agent --timeout=180s", check=False)


def main() -> None:
    cmd = sys.argv[1] if len(sys.argv) > 1 else "help"
    if cmd == "correctness6":
        pause_ds()
        strip_copy("scripts/correctness-phase6.sh", "/tmp/c6.sh")
        rc = sh(f"bash /tmp/c6.sh 2>&1 | tee {LOGS}/correctness6.log", check=False)
        resume_ds()
        raise SystemExit(rc)
    if cmd == "correctness12":
        pause_ds()
        strip_copy("scripts/correctness-phase12.sh", "/tmp/c12.sh")
        sh(
            "bash -lc 'source /home/sahil/.cargo/env; "
            "cargo build --release -p latency-server -p http-probe -p obsagent'",
            check=False,
        )
        rc = sh(f"bash /tmp/c12.sh 2>&1 | tee {LOGS}/correctness12.log", check=False)
        resume_ds()
        raise SystemExit(rc)
    print("usage: _claim_session.py {correctness6|correctness12}")
    raise SystemExit(2)


if __name__ == "__main__":
    main()
