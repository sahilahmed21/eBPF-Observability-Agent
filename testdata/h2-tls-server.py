#!/usr/bin/env python3
"""OpenSSL-backed HTTP/2 latency server (stdlib ssl → libssl). Not rustls."""

from __future__ import annotations

import os
import ssl
import time
from socket import AF_INET, SOCK_STREAM, SOL_SOCKET, SO_REUSEADDR, socket

PREFACE = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"
TYPE_HEADERS = 0x1
TYPE_SETTINGS = 0x4
TYPE_GOAWAY = 0x7
FLAG_END_STREAM = 0x1
FLAG_END_HEADERS = 0x4


def pack_frame(ty: int, flags: int, stream_id: int, payload: bytes = b"") -> bytes:
    n = len(payload)
    return bytes([(n >> 16) & 0xFF, (n >> 8) & 0xFF, n & 0xFF, ty, flags]) + stream_id.to_bytes(
        4, "big"
    ) + payload


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


def hpack_status_200() -> bytes:
    return b"\x88"


def main() -> None:
    port = int(os.environ.get("PORT", "18447"))
    delay_ms = int(os.environ.get("DELAY_MS", "50"))
    cert = os.environ["TLS_CERT"]
    key = os.environ["TLS_KEY"]
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(certfile=cert, keyfile=key)
    ctx.set_alpn_protocols(["h2"])
    ls = socket(AF_INET, SOCK_STREAM)
    ls.setsockopt(SOL_SOCKET, SO_REUSEADDR, 1)
    ls.bind(("127.0.0.1", port))
    ls.listen(8)
    print(f"h2-tls-server listening on https://127.0.0.1:{port}", flush=True)
    while True:
        raw, _ = ls.accept()
        ssock = ctx.wrap_socket(raw, server_side=True)
        try:
            handle(ssock, delay_ms)
        except (ConnectionError, ssl.SSLError, OSError):
            pass
        finally:
            try:
                ssock.close()
            except OSError:
                pass


def handle(ssock: ssl.SSLSocket, delay_ms: int) -> None:
    got = bytearray()
    while len(got) < len(PREFACE):
        chunk = ssock.recv(len(PREFACE) - len(got))
        if not chunk:
            return
        got.extend(chunk)
    if not bytes(got).startswith(PREFACE):
        return
    ssock.sendall(pack_frame(TYPE_SETTINGS, 0, 0))
    while True:
        ty, flags, stream_id, _payload = read_frame(ssock)
        if ty == TYPE_GOAWAY:
            return
        if ty == TYPE_HEADERS and stream_id != 0:
            time.sleep(delay_ms / 1000.0)
            ssock.sendall(
                pack_frame(
                    TYPE_HEADERS,
                    FLAG_END_HEADERS | FLAG_END_STREAM,
                    stream_id,
                    hpack_status_200(),
                )
            )
            return


if __name__ == "__main__":
    main()
