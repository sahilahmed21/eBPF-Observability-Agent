#!/usr/bin/env python3
"""HTTPS client via stdlib ssl (libssl) for Phase 3 Q9. Mirrors http-probe CLI."""

from __future__ import annotations

import argparse
import ssl
import sys
import urllib.error
import urllib.request


def once(host: str, port: int, path: str) -> None:
    url = f"https://{host}:{port}{path}"
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    ctx.options |= ssl.OP_NO_TICKET
    req = urllib.request.Request(url, method="GET")
    with urllib.request.urlopen(req, context=ctx, timeout=5) as resp:
        body = resp.read(64)
        if not body and resp.status >= 400:
            raise RuntimeError(f"bad status {resp.status} for {path}")


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=18443)
    p.add_argument("--path", action="append", dest="paths")
    p.add_argument("--repeat", type=int, default=1)
    args = p.parse_args()
    paths = args.paths or ["/fast"]
    rc = 0
    for _ in range(args.repeat):
        for path in paths:
            try:
                once(args.host, args.port, path)
            except Exception as e:  # noqa: BLE001 — probe reports and continues
                print(e, file=sys.stderr)
                rc = 1
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
