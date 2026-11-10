#!/usr/bin/env python3
"""Client for mock-llm-server with client-side ground truth (one JSON line per call on stdout).

ttfb_ms: just before the request is written -> response headers returned.
total_ms: same start -> the `message_stop` event line has been read.
"""

from __future__ import annotations

import argparse
import http.client
import json
import ssl
import time


def call(host: str, port: int, knobs: dict) -> dict:
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    ctx.options |= ssl.OP_NO_TICKET
    conn = http.client.HTTPSConnection(host, port, context=ctx, timeout=10)
    conn.connect()
    body = json.dumps(knobs).encode()
    t0 = time.monotonic()
    conn.request("POST", "/v1/messages", body=body, headers={"Content-Type": "application/json", "Content-Length": str(len(body))})
    resp = conn.getresponse()
    t1 = time.monotonic()
    while True:
        line = resp.readline()
        if not line or b"message_stop" in line:
            break
    t2 = time.monotonic()
    conn.close()
    return {"ttfb_ms": (t1 - t0) * 1000, "total_ms": (t2 - t0) * 1000, "in": knobs.get("input_tokens", 25), "out": knobs.get("output_tokens", 15)}


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=18450)
    p.add_argument("--repeat", type=int, default=1)
    p.add_argument("--ttft-ms", type=int, default=300)
    p.add_argument("--gap-ms", type=int, default=40)
    p.add_argument("--deltas", type=int, default=5)
    p.add_argument("--input-tokens", type=int, default=25)
    p.add_argument("--output-tokens", type=int, default=15)
    a = p.parse_args()
    knobs = {"ttft_ms": a.ttft_ms, "gap_ms": a.gap_ms, "deltas": a.deltas, "input_tokens": a.input_tokens, "output_tokens": a.output_tokens}
    for _ in range(a.repeat):
        print(json.dumps(call(a.host, a.port, knobs)), flush=True)


if __name__ == "__main__":
    main()
