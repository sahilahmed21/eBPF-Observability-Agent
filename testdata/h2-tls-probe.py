#!/usr/bin/env python3
"""OpenSSL HTTP/2 client (stdlib ssl → libssl) for Phase 7 h2-tls correctness."""

from __future__ import annotations

import argparse
import ssl
from socket import create_connection

PREFACE = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"
TYPE_HEADERS = 0x1
TYPE_SETTINGS = 0x4
FLAG_END_STREAM = 0x1
FLAG_END_HEADERS = 0x4


def pack_frame(ty: int, flags: int, stream_id: int, payload: bytes = b"") -> bytes:
    n = len(payload)
    return bytes([(n >> 16) & 0xFF, (n >> 8) & 0xFF, n & 0xFF, ty, flags]) + stream_id.to_bytes(
        4, "big"
    ) + payload


def hpack_get(path: bytes) -> bytes:
    return bytes([0x82, 0x04, len(path)]) + path


def read_exact(sock: ssl.SSLSocket, n: int) -> bytes:
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("eof")
        buf.extend(chunk)
    return bytes(buf)


def read_frame(sock: ssl.SSLSocket) -> tuple[int, int, int, bytes]:
    hdr = read_exact(sock, 9)
    length = (hdr[0] << 16) | (hdr[1] << 8) | hdr[2]
    ty, flags = hdr[3], hdr[4]
    stream_id = int.from_bytes(hdr[5:9], "big") & 0x7FFFFFFF
    payload = read_exact(sock, length) if length else b""
    return ty, flags, stream_id, payload


def once(host: str, port: int, path: str) -> None:
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    ctx.set_alpn_protocols(["h2"])
    raw = create_connection((host, port), timeout=5)
    ssock = ctx.wrap_socket(raw, server_hostname=host)
    try:
        path_b = path.encode()
        ssock.sendall(
            PREFACE
            + pack_frame(TYPE_SETTINGS, 0, 0)
            + pack_frame(
                TYPE_HEADERS,
                FLAG_END_HEADERS | FLAG_END_STREAM,
                1,
                hpack_get(path_b),
            )
        )
        while True:
            ty, flags, stream_id, _ = read_frame(ssock)
            if ty == TYPE_HEADERS and stream_id == 1:
                if flags & FLAG_END_HEADERS:
                    return
    finally:
        try:
            ssock.close()
        except OSError:
            pass


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=18447)
    p.add_argument("--path", default="/slow")
    p.add_argument("--repeat", type=int, default=1)
    args = p.parse_args()
    rc = 0
    for _ in range(args.repeat):
        try:
            once(args.host, args.port, args.path)
        except Exception as e:  # noqa: BLE001
            print(e, file=__import__("sys").stderr)
            rc = 1
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
