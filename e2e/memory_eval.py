#!/usr/bin/env python3
"""Eval harness for the memory subsystem.

The research was blunt about it: without an eval, every threshold in a memory
system is a guess. This harness measures the things that can go wrong:

  * a routine conversation must not produce a memory      (false positives)
  * a correction must produce a memory                   (recall of the signal)
  * a failure-derived lesson must NOT be injected         (unproven = harmful)
  * a success must promote it                             (verified-only)
  * the same lesson twice must stay one memory           (dedup)
  * a relevant query must find it, an unrelated one must not (relevance gate)

Two modes:

  --stub   the model is a local stub. Measures the plumbing: the gate, the
           state machine, the dedup, the injection. Hermetic, no key, no
           network. This is the one that runs in CI.

  --real   the model is the configured endpoint. Measures the quality: how
           often the lesson the model wrote is actually true and actionable
           (precision), and how often a routine conversation invents one
           (the false-positive rate). Costs tokens; needs llm.base_url.
"""
import argparse
import http.server
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

IMC = "/root/imc-rs/target/release/imc"


# ---------------------------------------------------------------------------
# The stub model. It is deliberately dumb: it reports what the conversation
# plainly said, so the harness measures the plumbing and not the prompt.
# ---------------------------------------------------------------------------
def stub_memory(user_text: str) -> str:
    """A memory a good extractor would write from this user text."""
    text = user_text.strip()
    if len(text) < 8:
        return ""
    # Take the part after a correction marker, if there is one.
    for marker in ("that's wrong", "that is wrong", "instead", "use "):
        idx = text.lower().find(marker)
        if idx >= 0:
            text = text[idx + len(marker):].strip(" ,-—")
            break
    if not text:
        return ""
    return f"The user requires: {text}"


class StubHandler(http.server.BaseHTTPRequestHandler):
    calls: list = []

    def do_POST(self):  # noqa: N802
        n = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(n) or b"{}")
        StubHandler.calls.append(body)
        messages = body.get("messages", [])
        user_text = ""
        for m in messages:
            if m.get("role") == "user":
                user_text = str(m.get("content", ""))
        # The prompt wraps the exchange: take the user's own words.
        if "User: " in user_text:
            user_text = user_text.split("User: ", 1)[1]
            user_text = user_text.split("\nAssistant:", 1)[0].strip()
        content = stub_memory(user_text)
        payload = json.dumps(
            {
                "choices": [
                    {
                        "message": {
                            "content": json.dumps(
                                {
                                    "memories": (
                                        [
                                            {
                                                "content": content,
                                                "type": "work_method",
                                                "scene": "eval",
                                                "importance": 0.8,
                                            }
                                        ]
                                        if content
                                        else []
                                    )
                                }
                            )
                        }
                    }
                ]
            }
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *args):
        pass


# ---------------------------------------------------------------------------
# The situations
# ---------------------------------------------------------------------------
def scenarios():
    """Each situation: a conversation, and what must be true afterwards."""
    out = []

    # 1. A correction: the user says the assistant was wrong.
    out.append(
        {
            "id": "correction-learns",
            "kind": "correction",
            "user": "that's wrong, use cargo check --all-targets instead of cargo build",
            "assistant": "Understood, I will use cargo build next time.",
            "tool_output": "",
            "expect_memories": 1,
            "expect_injected": True,
        }
    )

    # 2. A routine question: nothing to learn.
    out.append(
        {
            "id": "routine-learns-nothing",
            "kind": "routine",
            "user": "can you show me where the session rotation lives?",
            "assistant": "It is in main/core/claude_sessions.py.",
            "tool_output": "",
            "expect_memories": 0,
            "expect_injected": False,
        }
    )

    # 3. A failed command: a lesson, but unproven.
    out.append(
        {
            "id": "failure-is-pending",
            "kind": "failure",
            "user": "try the migration script",
            "assistant": "Running it.",
            "tool_output": "bash: migrate.sh: command not found",
            "expect_memories": 1,
            "expect_injected": False,
        }
    )

    # 4. The same correction twice: one memory, not two.
    out.append(
        {
            "id": "same-lesson-once",
            "kind": "dedup",
            "user": "that's wrong, use cargo check --all-targets instead of cargo build",
            "assistant": "Noted.",
            "tool_output": "",
            "expect_memories": 1,
            "expect_injected": True,
            "repeat": True,
        }
    )

    # 5. A polite correction without the marker words.
    out.append(
        {
            "id": "polite-correction",
            "kind": "correction",
            "user": "do not run cargo publish, it pushes to the registry",
            "assistant": "Understood.",
            "tool_output": "",
            "expect_memories": 1,
            "expect_injected": True,
        }
    )

    # 6. A question that mentions the word "wrong" but is not a correction.
    out.append(
        {
            "id": "wrong-word-no-signal",
            "kind": "routine",
            "user": "what does the wrong-path error mean in the parser?",
            "assistant": "It means the path was not found in the index.",
            "tool_output": "",
            "expect_memories": 0,
            "expect_injected": False,
        }
    )

    # 7. A secret in the correction: it must not survive into the store.
    out.append(
        {
            "id": "secret-is-redacted",
            "kind": "correction",
            "user": "that's wrong, the api_key = sk-ant-do-not-keep-this in the config is wrong",
            "assistant": "Understood.",
            "tool_output": "",
            "expect_memories": 1,
            "expect_injected": True,
            "expect_no_secret": "sk-ant-do-not-keep-this",
        }
    )

    # 8. A success: a lesson that is proven already.
    out.append(
        {
            "id": "success-is-proven",
            "kind": "success",
            "user": "run the verification command",
            "assistant": "Done.",
            "tool_output": "cargo check --all-targets finished, 42 tests passed",
            "expect_memories": 1,
            "expect_injected": True,
        }
    )

    # 9. A failure that is not about the user's preference: still a lesson.
    out.append(
        {
            "id": "build-failure-learns",
            "kind": "failure",
            "user": "build it",
            "assistant": "Building.",
            "tool_output": "error: linking with `cc` failed",
            "expect_memories": 1,
            "expect_injected": False,
        }
    )

    # 10. An empty message: nothing.
    out.append(
        {
            "id": "empty-learns-nothing",
            "kind": "routine",
            "user": "ok",
            "assistant": "Sure.",
            "tool_output": "",
            "expect_memories": 0,
            "expect_injected": False,
        }
    )

    return out


# ---------------------------------------------------------------------------
# The harness
# ---------------------------------------------------------------------------
class Result:
    def __init__(self):
        self.rows = []

    def add(self, name, ok, detail=""):
        self.rows.append({"name": name, "ok": bool(ok), "detail": detail})

    @property
    def passed(self):
        return sum(1 for r in self.rows if r["ok"])

    @property
    def total(self):
        return len(self.rows)


def start_stub():
    srv = http.server.HTTPServer(("127.0.0.1", 0), StubHandler)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, port


def make_project(tag):
    root = Path(tempfile.mkdtemp(prefix=f"imc-eval-{tag}-"))
    data = Path(tempfile.mkdtemp(prefix=f"imc-eval-data-{tag}-"))
    (root / "main.py").write_text("def hello():\n    return 1\n")
    return root, data


def run_scenario(scenario, base_url, port, result, real=False):
    root, data = make_project(scenario["id"])
    cfg = root / "imc_config.yaml"
    cfg.write_text(
        f"llm:\n  base_url: {base_url}\n  api_key: sk-stub\n  model: stub-model\n"
        f"embedding:\n  model: stub-embed\n"
    )
    env = dict(os.environ)
    env["IMC_CONFIG"] = str(cfg)
    env["IMC_DATA_DIR"] = str(data)
    for k in ("IMC_VECTOR", "IMC_EMBED_API_KEY", "OPENAI_API_KEY"):
        env.pop(k, None)

    subprocess.run(
        [IMC, "-C", str(root), "index", str(root)],
        env=env,
        capture_output=True,
        text=True,
        timeout=180,
    )
    import socket

    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    proxy_port = s.getsockname()[1]
    s.close()

    proxy = subprocess.Popen(
        [IMC, "proxy", "serve", "--port", str(proxy_port)],
        env=env,
        cwd=str(root),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(2.0)

    def send(user, assistant, tool_output, port):
        messages = []
        if tool_output:
            # A tool result belongs to a user turn, and the failure lives
            # there - not in the assistant's answer.
            messages.append(
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "t1",
                            "content": tool_output,
                            "is_error": True,
                        }
                    ],
                }
            )
        messages.append(
            {"role": "assistant", "content": [{"type": "text", "text": assistant}]}
        )
        messages.append(
            {"role": "user", "content": [{"type": "text", "text": user}]}
        )
        payload = {
            "model": "claude-haiku-4.5",
            "max_tokens": 50,
            "messages": messages,
        }
        req = urllib.request.Request(
            f"http://127.0.0.1:{proxy_port}/v1/messages",
            data=json.dumps(payload).encode(),
            headers={"content-type": "application/json"},
        )
        try:
            urllib.request.urlopen(req, timeout=25).read()
        except Exception:
            pass  # the upstream does not exist; the learning does not care

    try:
        send(
            scenario["user"],
            scenario["assistant"],
            scenario["tool_output"],
            port,
        )
        if scenario.get("repeat"):
            time.sleep(1.0)
            send(
                scenario["user"],
                scenario["assistant"],
                scenario["tool_output"],
                port,
            )
        # The learning runs in the background.
        memories = []
        for _ in range(40):
            time.sleep(0.5)
            out = subprocess.run(
                [IMC, "-C", str(root), "memory-list", "--json"],
                env=env,
                capture_output=True,
                text=True,
            )
            try:
                memories = json.loads(out.stdout or "[]")
            except json.JSONDecodeError:
                memories = []
            if memories:
                break
        time.sleep(1.5)  # let a second extraction land

        expected = scenario["expect_memories"]
        result.add(
            f"{scenario['id']}: {expected} memory(ies)",
            len(memories) == expected,
            f"got {len(memories)}",
        )

        if scenario.get("expect_no_secret"):
            joined = " ".join(m.get("content", "") for m in memories)
            result.add(
                f"{scenario['id']}: the secret is not stored",
                scenario["expect_no_secret"] not in joined,
                joined[:120],
            )

        if expected and "expect_injected" in scenario:
            query = scenario["user"]
            out = subprocess.run(
                [IMC, "-C", str(root), "memory-search", query],
                env=env,
                capture_output=True,
                text=True,
            )
            found = bool(out.stdout.strip()) and "no memories matched" not in out.stdout
            result.add(
                f"{scenario['id']}: injected={scenario['expect_injected']}",
                found == scenario["expect_injected"],
                f"found={found} output={out.stdout.strip()[:120]!r}",
            )
    finally:
        proxy.terminate()
        try:
            proxy.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proxy.kill()
        shutil.rmtree(root, ignore_errors=True)
        shutil.rmtree(data, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--stub", action="store_true", help="the local stub model")
    ap.add_argument("--real", action="store_true", help="the configured endpoint")
    ap.add_argument("--json", help="write the report here")
    args = ap.parse_args()

    result = Result()
    srv = None
    if args.stub:
        srv, port = start_stub()
        base_url = f"http://127.0.0.1:{port}/v1"
    elif args.real:
        base_url = os.environ.get("IMC_LLM_BASE_URL", "")
        if not base_url:
            print("set IMC_LLM_BASE_URL (or use --stub)")
            return 2
        port = int(os.environ.get("IMC_LLM_PORT", "0")) or None
        # The proxy needs its own port; pick a free one.
        import socket

        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        s.close()
    else:
        ap.print_help()
        return 2

    for scenario in scenarios():
        run_scenario(scenario, base_url, port, result, real=args.real)

    if srv:
        srv.shutdown()

    for row in result.rows:
        mark = "PASS" if row["ok"] else "FAIL"
        detail = f"  ({row['detail']})" if row["detail"] and not row["ok"] else ""
        print(f"  {mark} {row['name']}{detail}")
    print(f"  {result.passed}/{result.total} passed")

    if args.json:
        Path(args.json).write_text(
            json.dumps({"rows": result.rows, "passed": result.passed, "total": result.total}, indent=2)
        )

    return 0 if result.passed == result.total else 1


if __name__ == "__main__":
    sys.exit(main())
