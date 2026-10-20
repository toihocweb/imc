#!/usr/bin/env python3
"""End-to-end check for the MCP server over HTTP.

The proxy serves MCP at `POST /mcp` (streamable HTTP). This starts the real
proxy and asserts that:

  * `initialize` answers with the protocol version and the server info,
  * `tools/list` advertises the imc tools,
  * `tools/call` runs a real search against the project named by the
    `x-imc-root` header,
  * a notification (no `id`) gets no body back,
  * an unknown method answers with JSON-RPC error -32601.
"""
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IMC = ROOT / "target" / "release" / "imc"

failures = []


def check(name, ok, detail=""):
    print(f"  [{'PASS' if ok else 'FAIL'}] {name:<52} {detail}")
    if not ok:
        failures.append(name)


def free_port():
    import socket

    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def post(base, payload, headers=None):
    req = urllib.request.Request(
        base + "/mcp",
        data=json.dumps(payload).encode(),
        headers={"content-type": "application/json", **(headers or {})},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            body = r.read().decode()
            return r.status, body
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def wait_up(base, timeout=30):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(base + "/health", timeout=1) as r:
                if r.status == 200:
                    return True
        except Exception:
            time.sleep(0.3)
    return False


def main():
    port = free_port()
    env = dict(os.environ)
    env.pop("IMC_DATA_DIR", None)
    tmp = Path(tempfile.mkdtemp(prefix="imc-mcp-http-"))
    env["IMC_CCR_DB"] = str(tmp / "ccr.db")

    log = open(tmp / "proxy.log", "wb")
    proxy = subprocess.Popen(
        [str(IMC), "proxy", "serve", "--port", str(port)],
        env=env, stdout=log, stderr=subprocess.STDOUT,
    )
    base = f"http://127.0.0.1:{port}"
    try:
        if not wait_up(base):
            print("proxy did not come up; log tail:")
            print((tmp / "proxy.log").read_text()[-2000:])
            return 1

        print("MCP over HTTP")
        print("-" * 84)

        status, body = post(base, {
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05"},
        })
        data = json.loads(body) if body else {}
        result = data.get("result", {})
        check("initialize answers", status == 200 and result.get("serverInfo", {}).get("name") == "imc",
              result.get("serverInfo", {}).get("name", ""))
        check("protocol version is echoed", result.get("protocolVersion") == "2024-11-05",
              result.get("protocolVersion", ""))

        status, body = post(base, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        tools = json.loads(body).get("result", {}).get("tools", []) if body else []
        names = [t.get("name") for t in tools]
        check("tools/list advertises the imc tools",
              "search" in names and "trace" in names, ",".join(names[:6]))

        status, body = post(base, {
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "search", "arguments": {"query": "index project", "limit": 2}},
        }, {"x-imc-root": "/root/imc-rs"})
        data = json.loads(body) if body else {}
        content = data.get("result", {}).get("content", [])
        text = content[0].get("text", "") if content else ""
        check("tools/call runs a real search", bool(text) and "file" in text, f"{len(text)} chars")

        status, body = post(base, {"jsonrpc": "2.0", "method": "notifications/initialized"})
        check("a notification gets no body", body.strip() == "", f"status {status}")

        status, body = post(base, {"jsonrpc": "2.0", "id": 4, "method": "nope/nope"})
        data = json.loads(body) if body else {}
        check("unknown method answers -32601",
              data.get("error", {}).get("code") == -32601, body[:80])
    finally:
        proxy.terminate()
        try:
            proxy.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proxy.kill()
        log.close()

    print()
    if failures:
        print(f"FAILURES: {failures}")
        print("--- proxy log tail ---")
        print((tmp / "proxy.log").read_text()[-1200:])
        return 1
    print("all MCP-over-HTTP checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
