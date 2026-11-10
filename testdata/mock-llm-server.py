#!/usr/bin/env python3
"""Deterministic mock streaming LLM API over TLS (stdlib ssl -> libssl), HTTP/1.1 chunked SSE.

POST /v1/messages with a JSON body of knobs (all optional):
  ttft_ms (default 300)   delay before response headers
  gap_ms (40)             delay between stream events
  deltas (5)              number of content_block_delta events
  input_tokens (25)       reported in message_start
  output_tokens (15)      reported (cumulative) in the final message_delta
Event shapes follow https://platform.claude.com/docs/en/build-with-claude/streaming.
"""

from __future__ import annotations

import json
import os
import socket
import ssl
import time
from http.server import BaseHTTPRequestHandler, HTTPServer


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args) -> None:
        pass

    def _chunk(self, data: str) -> None:
        b = data.encode()
        self.wfile.write(f"{len(b):x}\r\n".encode() + b + b"\r\n")
        self.wfile.flush()

    def _openai(self, req: dict) -> None:
        """OpenAI-style streaming (what the Atlas gateway's upstream client speaks).

        Env knobs: OAI_HEADER_MS (delay before headers, 300), OAI_FIRST_TOKEN_MS (extra delay after
        headers before the first content chunk, 0), OAI_GAP_MS (40), OAI_DELTAS (5).
        prompt_tokens = len(request body) // 4 so rows differ; usage is sent only when
        stream_options.include_usage is true, as the real API does.
        """
        env = os.environ.get
        time.sleep(int(env("OAI_HEADER_MS", "300")) / 1000)
        self.send_response(200)
        self.send_header("content-type", "text/event-stream; charset=utf-8")
        self.send_header("transfer-encoding", "chunked")
        self.end_headers()
        self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        time.sleep(int(env("OAI_FIRST_TOKEN_MS", "0")) / 1000)
        gap, deltas = int(env("OAI_GAP_MS", "40")) / 1000, int(env("OAI_DELTAS", "5"))
        base = {"id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": 1700000000, "model": "mock"}

        def emit(obj: dict) -> None:
            self._chunk(f"data: {json.dumps({**base, **obj}, separators=(',', ':'))}\n\n")

        emit({"choices": [{"index": 0, "delta": {"role": "assistant"}}]})
        for i in range(deltas):
            if i:
                time.sleep(gap)
            emit({"choices": [{"index": 0, "delta": {"content": f"w{i}"}}]})
        time.sleep(gap)
        emit({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]})
        if (req.get("stream_options") or {}).get("include_usage"):
            pt = len(json.dumps(req)) // 4
            emit({"choices": [], "usage": {"prompt_tokens": pt, "completion_tokens": deltas, "total_tokens": pt + deltas}})
        self._chunk("data: [DONE]\n\n")
        self.wfile.write(b"0\r\n\r\n")
        self.wfile.flush()

    def do_POST(self) -> None:
        n = int(self.headers.get("Content-Length", "0"))
        knobs = json.loads(self.rfile.read(n) or b"{}")
        if self.path == "/v1/chat/completions":
            return self._openai(knobs)
        ttft = knobs.get("ttft_ms", 300) / 1000
        gap = knobs.get("gap_ms", 40) / 1000
        deltas = knobs.get("deltas", 5)
        tin, tout = knobs.get("input_tokens", 25), knobs.get("output_tokens", 15)
        time.sleep(ttft)
        self.send_response(200)
        self.send_header("content-type", "text/event-stream; charset=utf-8")
        self.send_header("transfer-encoding", "chunked")
        self.end_headers()
        self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        start = {"type": "message_start", "message": {"model": "mock", "usage": {"input_tokens": tin, "output_tokens": 1}}}
        self._chunk(f"event: message_start\ndata: {json.dumps(start, separators=(',', ':'))}\n\n")
        for i in range(deltas):
            time.sleep(gap)
            d = {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": f"w{i}"}}
            self._chunk(f"event: content_block_delta\ndata: {json.dumps(d, separators=(',', ':'))}\n\n")
        time.sleep(gap)
        md = {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": tout}}
        self._chunk(f"event: message_delta\ndata: {json.dumps(md, separators=(',', ':'))}\n\n")
        time.sleep(gap)
        self._chunk('event: message_stop\ndata: {"type":"message_stop"}\n\n')
        self.wfile.write(b"0\r\n\r\n")
        self.wfile.flush()


def main() -> None:
    port = int(os.environ.get("PORT", "18450"))
    httpd = HTTPServer(("127.0.0.1", port), Handler)
    scheme = "http"
    if os.environ.get("TLS", "1") != "0":  # TLS=0: cleartext, like Atlas's default local workers
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(certfile=os.environ["TLS_CERT"], keyfile=os.environ["TLS_KEY"])
        httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)
        scheme = "https"
    print(f"mock-llm-server listening on {scheme}://127.0.0.1:{port}", flush=True)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
