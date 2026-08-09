#!/usr/bin/env python3
"""OpenSSL-backed HTTPS latency server for Phase 3 (Q9). Uses stdlib ssl → libssl."""

from __future__ import annotations

import os
import ssl
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt: str, *args) -> None:  # quieter
        pass

    def do_GET(self) -> None:
        parsed = urlparse(self.path)
        path = parsed.path
        qs = parse_qs(parsed.query)
        delay_ms = 0
        status = 200
        body = b"fast"
        if path.startswith("/slow"):
            delay_ms = int(qs.get("delay_ms", ["50"])[0])
            body = b"slow"
        elif path.startswith("/users/"):
            body = b"user"
        elif path.startswith("/err"):
            status = 500
            body = b"err"
        if delay_ms > 0:
            time.sleep(delay_ms / 1000.0)
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)


def main() -> None:
    port = int(os.environ.get("PORT", "18443"))
    cert = os.environ["TLS_CERT"]
    key = os.environ["TLS_KEY"]
    httpd = HTTPServer(("127.0.0.1", port), Handler)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(certfile=cert, keyfile=key)
    httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)
    print(f"https-latency-server listening on https://127.0.0.1:{port}", flush=True)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
