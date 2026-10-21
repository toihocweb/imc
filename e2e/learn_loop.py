#!/usr/bin/env python3
"""End-to-end check: the proxy learns a memory from the traffic it sees.

The point of the check is the *whole* loop:

    a request with a correction  ->  the proxy gates it  ->  a background
    extraction  ->  the memory store  ->  the memory is there

The model is a local stub, so the check needs no key and no network.
"""
import http.server
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

IMC = "/root/imc-rs/target/release/imc"

EXTRACTED = {
    "memories": [
        {
            "content": "the deploy pipeline must run rustfmt before commit",
            "type": "work_method",
            "scene": "deployment",
            "importance": 0.8,
        }
    ]
}

calls = []


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):  # noqa: N802
        n = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(n) or b"{}")
        calls.append({"path": self.path, "body": body})
        content = json.dumps(EXTRACTED)
        payload = json.dumps({"choices": [{"message": {"content": content}}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *args):
        pass


def main() -> int:
    srv = http.server.HTTPServer(("127.0.0.1", 0), Handler)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    root = Path(tempfile.mkdtemp(prefix="imc-learn-"))
    data = Path(tempfile.mkdtemp(prefix="imc-learn-data-"))
    cfg = root / "imc_config.yaml"
    cfg.write_text(
        f"llm:\n  base_url: http://127.0.0.1:{port}/v1\n"
        f"  api_key: sk-stub\n  model: stub-model\n"
        f"embedding:\n  model: stub-embed\n"
    )
    (root / "main.py").write_text("def hello():\n    return 1\n")

    env = dict(os.environ)
    env["IMC_CONFIG"] = str(cfg)
    env["IMC_DATA_DIR"] = str(data)
    for k in ("IMC_VECTOR", "IMC_EMBED_API_KEY", "OPENAI_API_KEY"):
        env.pop(k, None)

    # A project the proxy can detect.
    subprocess.run([IMC, "-C", str(root), "index", str(root)], env=env, capture_output=True, text=True)

    proxy_log = open("/tmp/imc-learn-proxy.log", "wb")
    proxy = subprocess.Popen(
        [IMC, "proxy", "serve", "--port", str(port + 1)],
        env=env,
        cwd=str(root),
        stdout=proxy_log,
        stderr=subprocess.STDOUT,
    )
    time.sleep(2.5)

    # A conversation: the assistant said something, the user corrects it.
    payload = {
        "model": "claude-haiku-4.5",
        "max_tokens": 100,
        "messages": [
            {"role": "assistant", "content": [{"type": "text", "text": "Run cargo build to check."}]},
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "that's wrong, use cargo check --all-targets instead",
                    }
                ],
            },
        ],
    }
    import urllib.request

    req = urllib.request.Request(
        f"http://127.0.0.1:{port + 1}/v1/messages",
        data=json.dumps(payload).encode(),
        headers={"content-type": "application/json"},
    )
    try:
        urllib.request.urlopen(req, timeout=20).read()
    except Exception as e:  # the upstream does not exist; the proxy may 5xx
        print(f"   (proxy answered: {e})")

    # The learning runs in the background; give it a moment.
    stored = []
    for _ in range(40):
        time.sleep(0.5)
        out = subprocess.run(
            [IMC, "-C", str(root), "memory-list", "--json"],
            env=env,
            capture_output=True,
            text=True,
        )
        try:
            stored = json.loads(out.stdout or "[]")
        except json.JSONDecodeError:
            stored = []
        if stored:
            break

    proxy.terminate()
    try:
        proxy.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proxy.kill()
    srv.shutdown()

    extraction_calls = [c for c in calls if c["body"].get("response_format")]
    print(f"extraction calls to the model: {len(extraction_calls)}")
    print("proxy log tail:")
    print(open("/tmp/imc-learn-proxy.log").read()[-1200:])
    print(f"memories stored: {len(stored)}")
    for m in stored:
        print(f"  #{m['id']} [{m['mem_type']}] scene={m['scene']!r} imp={m['importance']}: {m['content']}")

    ok = True
    if not extraction_calls:
        print("FAIL: the proxy never asked the model what was worth keeping")
        ok = False
    if not stored:
        print("FAIL: nothing was stored")
        ok = False
    elif not any("rustfmt" in m["content"] for m in stored):
        print("FAIL: the stored memory is not the extracted one")
        ok = False

    shutil.rmtree(root, ignore_errors=True)
    shutil.rmtree(data, ignore_errors=True)
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
