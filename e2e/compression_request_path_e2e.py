#!/usr/bin/env python3
"""End-to-end check that the proxy compresses tool results *in the request path*.

Spins up a stub Anthropic upstream, points the proxy at it through a temporary
config, sends a `/v1/messages` request whose tool result is a large build log,
and then asserts:

  * the body the upstream actually received carries the compressed text plus a
    retrieval handle (so compression ran before forwarding, not after),
  * the handle resolves through `POST /v1/retrieve` to the exact original bytes,
  * a small tool result is forwarded untouched (no false positives),
  * an `is_error` tool result is forwarded untouched (errors are the answer).
"""
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IMC = ROOT / "target" / "release" / "imc"

captured = {}


class Stub(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        body = self.rfile.read(length)
        captured["body"] = body
        captured["path"] = self.path
        resp = {
            "id": "msg_stub",
            "type": "message",
            "role": "assistant",
            "model": captured.get("model", "stub"),
            "content": [{"type": "text", "text": "stub ok"}],
            "stop_reason": "end_turn",
            "stop_sequence": None,
            "usage": {"input_tokens": 1, "output_tokens": 1},
        }
        payload = json.dumps(resp).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *a):
        pass


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_health(base, timeout=30):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(base + "/health", timeout=1) as r:
                if r.status == 200:
                    return True
        except Exception:
            time.sleep(0.3)
    return False


failures = []


def check(name, ok, detail=""):
    print(f"  [{'PASS' if ok else 'FAIL'}] {name:<56} {detail}")
    if not ok:
        failures.append(name)


def messages_payload(model, tool_text, is_error=False):
    block = {
        "type": "tool_result",
        "tool_use_id": "toolu_01",
        "content": tool_text,
    }
    if is_error:
        block["is_error"] = True
    return {
        "model": model,
        "max_tokens": 64,
        "messages": [{"role": "user", "content": [block]}],
    }


def post(base, path, payload):
    req = urllib.request.Request(
        base + path,
        data=json.dumps(payload).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read())


def forwarded_text():
    body = json.loads(captured["body"])
    for m in body.get("messages", []):
        content = m.get("content")
        if isinstance(content, list):
            for b in content:
                if isinstance(b, dict) and b.get("type") == "tool_result":
                    return b.get("content", "")
    return ""


def main():
    stub_port = free_port()
    proxy_port = free_port()
    stub = HTTPServer(("127.0.0.1", stub_port), Stub)
    threading.Thread(target=stub.serve_forever, daemon=True).start()

    tmp = Path(tempfile.mkdtemp(prefix="imc-rp-"))
    cfg_path = tmp / "config.json"
    cfg_path.write_text(json.dumps({
        "providers": {
            "stub": {
                "enabled": True,
                "baseUrl": f"http://127.0.0.1:{stub_port}",
                "apiKey": "stub-key",
            }
        },
        "modelMappings": {"stub--test-model": "stub:test-model"},
    }))

    env = dict(os.environ)
    env["COPILOT_API_CONFIG_PATH"] = str(cfg_path)
    env["IMC_CCR_DB"] = str(tmp / "ccr.db")
    env.pop("IMC_DATA_DIR", None)

    log = open(tmp / "proxy.log", "wb")
    proxy = subprocess.Popen(
        [str(IMC), "proxy", "serve", "--port", str(proxy_port)],
        env=env, stdout=log, stderr=subprocess.STDOUT,
    )
    base = f"http://127.0.0.1:{proxy_port}"
    try:
        if not wait_health(base):
            print("proxy did not come up; log tail:")
            print((tmp / "proxy.log").read_text()[-2000:])
            return 1

        print("request-path compression")
        print("-" * 84)

        big_log = "\n".join(
            [f"2026-10-19 10:00:{i:02d} INFO compiling module_{i}" for i in range(200)]
            + ["2026-10-19 10:05:00 ERROR src/net.rs:42 connection refused"]
        )

        post(base, "/v1/messages", messages_payload("stub--test-model", big_log))
        got = forwarded_text()
        check("upstream received the request", bool(captured.get("body")), captured.get("path", ""))
        check("tool result was compressed before forwarding", len(got) < len(big_log),
              f"{len(big_log)} -> {len(got)} bytes")
        check("compressed body keeps the ERROR line", "connection refused" in got)
        check("compressed body carries a retrieval handle", "/v1/retrieve" in got)

        id_start = got.find('{"id":"')
        rid = got[id_start + 7: id_start + 7 + got[id_start + 7:].find('"')] if id_start >= 0 else ""
        check("handle carries a non-empty id", bool(rid), rid[:16])

        if rid:
            back = post(base, "/v1/retrieve", {"id": rid})
            check("retrieve returns the exact original bytes", back.get("original") == big_log,
                  f'{len(back.get("original", ""))} vs {len(big_log)}')

        # Small result: must be forwarded verbatim.
        captured.clear()
        small = "ok: 3 files changed"
        post(base, "/v1/messages", messages_payload("stub--test-model", small))
        check("small tool result is forwarded untouched", forwarded_text() == small,
              repr(forwarded_text())[:40])

        # Error result: must be forwarded verbatim even when large.
        captured.clear()
        err_log = big_log + "\nERROR: build failed"
        post(base, "/v1/messages", messages_payload("stub--test-model", err_log, is_error=True))
        check("error tool result is never compressed", forwarded_text() == err_log,
              f"{len(forwarded_text())} bytes")
    finally:
        proxy.terminate()
        try:
            proxy.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proxy.kill()
        stub.shutdown()
        log.close()

    print()
    if failures:
        print(f"FAILURES: {failures}")
        print("--- proxy log tail ---")
        print((tmp / "proxy.log").read_text()[-1500:])
        return 1
    print("all request-path checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
