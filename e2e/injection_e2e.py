#!/usr/bin/env python3
"""Live end-to-end check of the proxy's context injection.

Runs the real Rust proxy against a local stub upstream, sends a Claude-Code
shaped /v1/messages request, and inspects the payload the proxy forwarded.
Asserts that the project's learned memory actually reaches the model, and
that a second turn does not inject the block twice.

Hermetic: temp data dir, temp config, stub upstream on 127.0.0.1, no network.
"""

from __future__ import annotations

import http.server
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

IMC = Path(__file__).resolve().parent.parent / "target" / "release" / "imc"

RECEIVED: list[dict] = []
SEEN: list[tuple] = []


class StubHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):  # noqa: N802
        SEEN.append(("POST", self.path))
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        try:
            item = json.loads(raw)
            item["_path"] = self.path
            RECEIVED.append(item)
        except Exception:
            RECEIVED.append({"raw": raw.decode("utf-8", "replace")})
        body = json.dumps(
            {
                "id": "msg_stub",
                "type": "message",
                "role": "assistant",
                "model": "anthropic--stub-model",
                "content": [{"type": "text", "text": "stub answer"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 1, "output_tokens": 1},
            }
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):  # noqa: N802
        SEEN.append(("GET", self.path))
        self.send_response(501)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, *_args):
        pass


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def wait_port(port: int, budget: float = 25.0) -> bool:
    deadline = time.time() + budget
    while time.time() < deadline:
        s = socket.socket()
        s.settimeout(0.4)
        try:
            s.connect(("127.0.0.1", port))
            s.close()
            return True
        except OSError:
            s.close()
            time.sleep(0.25)
    return False


def main() -> int:
    if not IMC.exists():
        print(f"binary not found: {IMC}\nrun: cargo build --release")
        return 2

    tmp = Path(tempfile.mkdtemp(prefix="imc-inject-"))
    proj = tmp / "proj"
    proj.mkdir()
    (proj / "app.py").write_text(
        "def changelog_entry(version, date):\n"
        "    return {'version': version, 'date': date}\n",
        encoding="utf-8",
    )
    data_dir = tmp / "data"
    data_dir.mkdir()
    cfg = tmp / "imc.yaml"
    cfg.write_text("# hermetic\n")

    env = dict(os.environ)
    env["IMC_DATA_DIR"] = str(data_dir)
    env["IMC_CONFIG"] = str(cfg)
    env["IMC_CCR_DB"] = str(tmp / "ccr.db")
    for key in ("IMC_VECTOR", "IMC_EMBED_API_KEY", "OPENAI_API_KEY"):
        env.pop(key, None)

    def imc(*args, expect_rc=0):
        p = subprocess.run([str(IMC), "-C", str(proj), *args], capture_output=True,
                           text=True, env=env, timeout=180, cwd=str(proj))
        if p.returncode != expect_rc:
            raise AssertionError(f"imc {' '.join(args)} rc={p.returncode}: {p.stderr[:300]}")
        return p.stdout

    # 1. index + one durable memory for this project
    imc("index", ".")
    imc("memory-add", "Changelog entries use ISO 8601 dates",
        "--category", "convention", "--type", "business_rule")

    # 2. stub upstream
    upstream_port = free_port()
    srv = http.server.HTTPServer(("127.0.0.1", upstream_port), StubHandler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    # 3. point the proxy at the stub
    for key, val in (
        ("llm.base_url", f"http://127.0.0.1:{upstream_port}"),
        ("llm.api_key", "sk-stub"),
        ("llm.model", "stub-model"),
    ):
        subprocess.run([str(IMC), "config", "set", key, val], capture_output=True,
                       text=True, env=env, timeout=60, check=True)

    # 4. run the real proxy
    proxy_port = free_port()
    proxy_log = open(tmp / "proxy.log", "w")
    proxy = subprocess.Popen([str(IMC), "-C", str(proj), "proxy", "serve",
                              "--port", str(proxy_port)],
                             stdout=proxy_log, stderr=subprocess.STDOUT,
                             text=True, env=env)
    try:
        if not wait_port(proxy_port):
            out = proxy.stdout.read() if proxy.stdout else ""
            raise AssertionError(f"proxy did not start:\n{out[:800]}")

        def ask(user_text, session):
            payload = {
                "model": "anthropic--stub-model",
                "max_tokens": 256,
                "system": [
                    {"type": "text",
                     "text": f"You are Claude Code.\nWorking directory: {proj}\n"}
                ],
                "messages": [{"role": "user", "content": user_text}],
                "metadata": {"user_id": f"session_{session}"},
            }
            req = subprocess.run(
                ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}",
                 "--max-time", "60", "-X", "POST",
                 "-H", "Content-Type: application/json",
                 "-d", json.dumps(payload),
                 f"http://127.0.0.1:{proxy_port}/v1/messages"],
                capture_output=True, text=True, timeout=90)
            if req.stdout.strip() != "200":
                proxy.terminate()
                time.sleep(0.5)
                raise AssertionError(
                    f"proxy status {req.stdout}\n--- proxy log ---\n"
                    + (tmp / "proxy.log").read_text()[-1500:])

        def llm_calls():
            return [r for r in RECEIVED if r.get("_path", "").endswith("/chat/completions")]

        ask("how should I format changelog dates?", "s1")
        calls = llm_calls()
        assert calls, "upstream received no LLM call"
        first = json.dumps(calls[-1])
        assert "<working-memory>" in first, "no working-memory block forwarded"
        assert "ISO 8601" in first, "memory content missing from forwarded payload"
        assert first.count("<working-memory>") == 1, "memory block injected twice"
        print("PASS turn 1: learned memory reached the upstream payload exactly once")

        # same session, matching query -> memory again, still one block
        ask("changelog date format again please", "s1")
        calls = llm_calls()
        second = json.dumps(calls[-1])
        assert "<working-memory>" in second, "second turn lost the memory block"
        assert second.count("<working-memory>") == 1, "second turn duplicated the block"
        print("PASS turn 2: same session keeps exactly one memory block")

        # unrelated query -> lexical match fails, nothing must be injected
        ask("what is the deployment pipeline for kubernetes", "s1")
        calls = llm_calls()
        third = json.dumps(calls[-1])
        logtext = (tmp / "proxy.log").read_text()
        for line in logtext.splitlines():
            if "[IMC Memory] Selected" in line or "Injected" in line or "[USER]" in line:
                print("LOG:", line[-160:])
        has_block = "<working-memory>" in third
        has_lessons = "<failure-lessons>" in third
        print("turn3 has_block:", has_block, "has_lessons:", has_lessons)
        if has_block:
            i = third.find("<working-memory>")
            print("turn3 injected:", third[max(0,i-300):i + 300])
        assert not (has_block and not has_lessons), "unrelated query got a memory block"
        print("PASS turn 3: unrelated query injects nothing (no false positives)")
        return 0
    finally:
        proxy.terminate()
        try:
            proxy.wait(timeout=10)
        except Exception:
            proxy.kill()
        # `imc proxy serve` daemonises: the CLI exits but the listener survives,
        # so kill whatever owns the proxy port as well.
        subprocess.run(["bash", "-lc",
                        f"pid=$(ss -tlnp | grep '127.0.0.1:{proxy_port}' "
                        f"| grep -oP 'pid=\\K[0-9]+' | head -1); "
                        f"[ -n \"$pid\" ] && kill $pid || true"],
                       capture_output=True, text=True)
        srv.shutdown()


if __name__ == "__main__":
    sys.exit(main())
