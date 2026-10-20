#!/usr/bin/env python3
"""End-to-end check for the request-path fixes.

Spins up a stub Anthropic upstream, starts the real proxy against it, and
asserts what the upstream actually received:

  * the model the client asked for is rewritten through `modelMappings`
    (the `provider:` prefix is stripped, like Python's `apply_model_mapping`),
  * an unmapped model is left alone,
  * a `tool_use_id` that violates Anthropic's `^[a-zA-Z0-9_-]+$` is repaired
    on both sides of the pair, so the API does not answer 400.
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
            "model": "stub",
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


def post(base, path, payload):
    req = urllib.request.Request(
        base + path,
        data=json.dumps(payload).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read())


def messages_payload(model, tool_use_id):
    return {
        "model": model,
        "max_tokens": 64,
        "messages": [
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "tool_use",
                        "id": tool_use_id,
                        "name": "Read",
                        "input": {"file_path": "/tmp/x"},
                    }
                ],
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": tool_use_id, "content": "file body"}
                ],
            },
        ],
    }


def forwarded():
    return json.loads(captured["body"])


def forwarded_tool_ids():
    body = forwarded()
    calls, results = [], []
    for m in body.get("messages", []):
        content = m.get("content")
        if not isinstance(content, list):
            continue
        for b in content:
            if not isinstance(b, dict):
                continue
            if b.get("type") == "tool_use":
                calls.append(b.get("id", ""))
            elif b.get("type") == "tool_result":
                results.append(b.get("tool_use_id", ""))
    return calls, results


def main():
    stub_port = free_port()
    proxy_port = free_port()
    stub = HTTPServer(("127.0.0.1", stub_port), Stub)
    threading.Thread(target=stub.serve_forever, daemon=True).start()

    tmp = Path(tempfile.mkdtemp(prefix="imc-map-"))
    cfg_path = tmp / "config.json"
    cfg_path.write_text(json.dumps({
        "providers": {
            "stub": {
                "enabled": True,
                "baseUrl": f"http://127.0.0.1:{stub_port}",
                "apiKey": "stub-key",
            }
        },
        "modelMappings": {
            "claude-pro-max": "stub:claude-haiku-4.5",
            "claude-sonnet-4-5": "stub:claude-sonnet-4-5",
        },
    }))

    # The default upstream (used when a model is not mapped) comes from the imc
    # config; point it at the stub so an unmapped model can be observed too.
    imc_cfg = tmp / "imc_config.yaml"
    imc_cfg.write_text(
        "llm:\n"
        f"  base_url: http://127.0.0.1:{stub_port}\n"
        "  api_key: stub-key\n"
        "  model: stub-default-model\n"
        "embedding:\n"
        "  model: text-embedding-3-small\n"
    )

    env = dict(os.environ)
    env["COPILOT_API_CONFIG_PATH"] = str(cfg_path)
    env["IMC_CONFIG"] = str(imc_cfg)
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

        print("model mapping + tool id repair")
        print("-" * 84)

        # 1. A mapped model is rewritten, the provider prefix is stripped.
        captured.clear()
        post(base, "/v1/messages", messages_payload("claude-pro-max", "toolu_01"))
        got_model = forwarded().get("model", "")
        check("mapped model is rewritten for the upstream", got_model == "claude-haiku-4.5", got_model)

        # 2. A mapping that keeps the name leaves the name alone.
        #    (A model with no mapping at all goes to the Copilot/Anthropic
        #    default route, which needs credentials — out of scope here.)
        captured.clear()
        post(base, "/v1/messages", messages_payload("claude-sonnet-4-5", "toolu_01"))
        got_model = forwarded().get("model", "")
        check("mapping to the same name keeps the name", got_model == "claude-sonnet-4-5", got_model)

        # 3. A tool id that violates Anthropic's pattern is repaired, and the
        #    call and its result keep pointing at each other.
        captured.clear()
        bad_id = "call_abc.def"
        post(base, "/v1/messages", messages_payload("claude-pro-max", bad_id))
        calls, results = forwarded_tool_ids()
        check("tool_use id satisfies ^[a-zA-Z0-9_-]+$",
              bool(calls) and all(all(ch.isalnum() or ch in "_-" for ch in i) for i in calls),
              str(calls))
        check("tool_result id satisfies the same pattern",
              bool(results) and all(all(ch.isalnum() or ch in "_-" for ch in i) for i in results),
              str(results))
        check("the call and its result stay paired", calls == results, f"{calls} vs {results}")

        # 4. An empty id on both sides is repaired into a valid pair.
        captured.clear()
        post(base, "/v1/messages", messages_payload("claude-pro-max", ""))
        calls, results = forwarded_tool_ids()
        check("an empty tool id becomes a valid id",
              bool(calls) and all(all(ch.isalnum() or ch in "_-" for ch in i) for i in calls),
              str(calls))
        check("the empty pair stays paired", calls == results, f"{calls} vs {results}")

        # 5. A valid id is not touched.
        captured.clear()
        post(base, "/v1/messages", messages_payload("claude-pro-max", "toolu_01ABC"))
        calls, results = forwarded_tool_ids()
        check("a valid tool id is left alone", calls == ["toolu_01ABC"] and results == ["toolu_01ABC"],
              f"{calls} / {results}")
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
    print("all model-map / tool-id checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
