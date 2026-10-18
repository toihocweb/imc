#!/usr/bin/env python3
"""End-to-end suite for the imc Rust port.

Runs the real `target/release/imc` binary against generated fixture projects --
no mocks, no in-process shortcuts. Every assertion is about observable
behaviour: exit codes, stdout, files on disk, SQLite rows.

    python3 e2e/e2e_suite.py                # hermetic (BM25 + graph only)
    python3 e2e/e2e_suite.py --with-vectors  # also exercise the real embedding path
    python3 e2e/e2e_suite.py --keep          # leave fixture dirs behind

Suites
    A  index lifecycle   fresh / incremental add, modify, delete, force, excludes
    B  retrieval quality top-1 for unambiguous queries; junk returns nothing
    C  call graph        callers + callees + blast radius; error exit codes
    D  reader            truncation header, --symbols, --method
    E  memory            typed add, recall, reinforce, prune_stale
    F  claude wiring     settings + CLAUDE.md, idempotency, env preservation
    G  config            set/get roundtrip, masking, migration
    H  proxy lifecycle   start / status / stop of the daemon
    I  json contract     every command honours --json
    J  multi-language    python / ts / rust / go / java symbols and traces
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IMC = ROOT / "target" / "release" / "imc"
PROXYD = ROOT / "target" / "release" / "imc-proxyd"

PASS = []
FAIL = []
SKIP = []


# -- harness -------------------------------------------------------------------
def check(name, fn):
    started = time.time()
    try:
        detail = fn() or ""
        PASS.append((name, detail))
        print(f"  PASS {name}  {detail}")
    except AssertionError as e:
        FAIL.append((name, str(e)))
        print(f"  FAIL {name}\n       {e}")
    except Exception as e:  # noqa: BLE001
        FAIL.append((name, f"{type(e).__name__}: {e}"))
        print(f"  ERROR {name}: {type(e).__name__}: {e}")


def skip(name, why):
    SKIP.append((name, why))
    print(f"  SKIP {name}  ({why})")


# -- fixture -------------------------------------------------------------------
FIXTURE = {
    "src/billing.py": '''"""Billing domain."""


class InvoiceCalculator:
    def calculate_tax(self, amount, rate):
        return amount * rate

    def apply_discount(self, amount, percent):
        return amount * (1 - percent)


def generate_invoice(customer, line_items):
    """Build an invoice payload from line items."""
    calc = InvoiceCalculator()
    subtotal = sum(item.amount for item in line_items)
    return {
        "customer": customer,
        "subtotal": subtotal,
        "tax": calc.calculate_tax(subtotal, 0.2),
    }


def export_invoice(items):
    """Callers probe for the e2e suite."""
    return generate_invoice("ACME", items)


def audit_invoice(invoice):
    calc = InvoiceCalculator()
    return calc.calculate_tax(invoice["subtotal"], 0.1)
''',
    "src/payments/ledger.ts": """export interface Entry {
  id: string;
  amount: number;
}

export function appendEntry(ledger: Entry[], entry: Entry): Entry[] {
  return [...ledger, entry];
}

export function reconcileLedger(ledger: Entry[]): number {
  return ledger.reduce((sum, e) => sum + e.amount, 0);
}
""",
    "src/engine.rs": """pub struct Engine {
    rpm: u32,
}

impl Engine {
    pub fn new(rpm: u32) -> Self {
        Engine { rpm }
    }

    pub fn throttle(&self, requests: usize) -> u32 {
        (requests as u32).min(self.rpm)
    }
}

pub fn spawn_engine(rpm: u32) -> Engine {
    Engine::new(rpm)
}

pub fn boot() -> Engine {
    spawn_engine(1000)
}
""",
    "cmd/server/main.go": """package main

import "fmt"

type Server struct {
    Port int
}

func (s *Server) Start() error {
    fmt.Println("starting")
    return nil
}

func NewServer(port int) *Server {
    return &Server{Port: port}
}

func main() {
    srv := NewServer(8080)
    srv.Start()
}
""",
    "src/main/java/com/shop/OrderService.java": """package com.shop;

import java.util.List;

public class OrderService {
    private final Calculator calc = new Calculator();

    public int subtotal(List<Integer> items) {
        return items.stream().mapToInt(Integer::intValue).sum();
    }

    public int taxOn(int amount) {
        return calc.add(amount, amount / 10);
    }
}

class Calculator {
    int add(int a, int b) {
        return a + b;
    }
}
""",
    "docs/architecture.md": """# Architecture

The indexer walks the tree, parses symbols with tree-sitter, and stores
everything in one SQLite database per project.
""",
    "README.md": "# Fixture project\n\nUsed by the e2e suite.\n",
}

FILE_COUNT = len(FIXTURE)


def make_fixture(root):
    for rel, body in FIXTURE.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(body)


def new_case(tag):
    d = Path(tempfile.mkdtemp(prefix=f"imc-e2e-{tag}-"))
    make_fixture(d)
    return d


def imc_env(data_dir, vectors):
    env = dict(os.environ)
    env["IMC_DATA_DIR"] = str(data_dir)
    env["IMC_CONFIG"] = str(data_dir / "config.yaml")
    if not vectors:
        # Hermetic: never inherit a real embedding key from the user's config.
        for k in ("IMC_EMBED_API_KEY", "OPENAI_API_KEY", "IMC_VECTOR"):
            env.pop(k, None)
    return env


def run_imc(args, cwd, data_dir, vectors=False, expect_rc=0):
    p = subprocess.run(
        [str(IMC), *args],
        cwd=str(cwd),
        capture_output=True,
        text=True,
        env=imc_env(data_dir, vectors),
        timeout=900,
    )
    if expect_rc is not None and p.returncode != expect_rc:
        raise AssertionError(
            f"rc={p.returncode} (want {expect_rc}) for `imc {' '.join(args)}`\n"
            f"stdout: {p.stdout[:400]}\nstderr: {p.stderr[:400]}"
        )
    return p


def jrun(args, cwd, data_dir, vectors=False):
    return json.loads(run_imc(["--json", *args], cwd, data_dir, vectors).stdout)


def project_db(data_dir, root):
    slug = hashlib.md5(str(root.resolve()).encode()).hexdigest()[:16]
    return data_dir / "projects" / slug / "index.db"


def con(data_dir, root):
    return sqlite3.connect(project_db(data_dir, root))


# -- A. index lifecycle --------------------------------------------------------
def suite_index(vectors):
    root = new_case("index")
    data = Path(tempfile.mkdtemp(prefix="imc-e2e-data-"))

    def fresh_index():
        stats = jrun(["index", "."], root, data, vectors)
        assert stats["new_files"] == FILE_COUNT, f"expected {FILE_COUNT} new: {stats}"
        assert stats["total_files"] == FILE_COUNT, stats
        assert stats["chunks"] > 0, stats
        assert stats["symbols"] > 0, stats
        assert stats["edges"] > 0, stats
        assert stats["modified_files"] == 0, stats
        return (
            f"{stats['total_files']} files, {stats['symbols']} symbols, "
            f"{stats['chunks']} chunks, {stats['edges']} edges"
        )

    check("A1 fresh index counts every file", fresh_index)

    def all_files_landed():
        db = con(data, root)
        got = sorted(r[0] for r in db.execute("SELECT path FROM files"))
        db.close()
        assert got == sorted(FIXTURE), f"missing: {sorted(set(FIXTURE) - set(got))}"
        return f"{len(got)} rows match the tree"

    check("A2 index stores exactly the fixture files", all_files_landed)

    def incremental_noop():
        stats = jrun(["index", "."], root, data, vectors)
        assert stats["new_files"] == 0, stats
        assert stats["unchanged_files"] == FILE_COUNT, stats
        assert stats["modified_files"] == 0, stats
        return "re-index is a no-op"

    check("A3 incremental re-index skips unchanged files", incremental_noop)

    def incremental_add_modify_delete():
        (root / "src/new_module.py").write_text("def added():\n    return 1\n")
        (root / "src/billing.py").write_text(
            FIXTURE["src/billing.py"] + "\n\ndef extra():\n    return 2\n"
        )
        stats = jrun(["index", "."], root, data, vectors)
        assert stats["new_files"] == 1, stats
        assert stats["modified_files"] == 1, stats

        (root / "src/new_module.py").unlink()
        stats = jrun(["index", "."], root, data, vectors)
        assert stats["deleted_files"] == 1, stats
        db = con(data, root)
        n = db.execute("SELECT COUNT(*) FROM files WHERE path='src/new_module.py'").fetchone()[0]
        db.close()
        assert n == 0, "deleted file still indexed"
        return "add / modify / delete all tracked"

    check("A4 incremental tracks add, modify and delete", incremental_add_modify_delete)

    def forced_reindex():
        stats = jrun(["index", "--force", "."], root, data, vectors)
        db = con(data, root)
        files = db.execute("SELECT COUNT(*) FROM files").fetchone()[0]
        syms = db.execute("SELECT COUNT(*) FROM symbols").fetchone()[0]
        edges = db.execute("SELECT COUNT(*) FROM edges").fetchone()[0]
        db.close()
        assert files == FILE_COUNT, (files, stats)
        assert syms == stats["symbols"], (syms, stats)
        assert edges == stats["edges"], (edges, stats)
        return f"force rebuilt {files} files with no duplicated rows"

    check("A5 --force rebuilds without duplicating rows", forced_reindex)

    def excludes_noise():
        (root / "assets").mkdir(exist_ok=True)
        (root / "assets/index-Ab3xY9z-Q.js").write_text("x" * 4000)
        (root / "node_modules/dep").mkdir(parents=True, exist_ok=True)
        (root / "node_modules/dep/index.js").write_text("module.exports = 1;\n")
        run_imc(["--json", "index", "."], root, data, vectors)
        db = con(data, root)
        noise = db.execute(
            "SELECT path FROM files WHERE path LIKE 'assets/%' OR path LIKE 'node_modules/%'"
        ).fetchall()
        db.close()
        assert not noise, f"noise indexed: {noise}"
        return "hashed bundles and node_modules excluded"

    check("A6 build artifacts are not indexed", excludes_noise)

    return root, data


# -- B. retrieval quality ------------------------------------------------------
QUALITY = {
    "calculate tax on an invoice": "src/billing.py",
    "append an entry to the ledger": "src/payments/ledger.ts",
    "throttle requests with an engine": "src/engine.rs",
    "start the http server on a port": "cmd/server/main.go",
    "order service subtotal of items": "src/main/java/com/shop/OrderService.java",
}


def suite_retrieval(vectors, root, data):
    for q, want in QUALITY.items():
        def t(q=q, want=want):
            hits = jrun(["search", q, "-l", "5"], root, data, vectors)
            assert hits, f"no hits for {q!r}"
            got = hits[0]["file"]
            assert got == want, (
                f"{q!r}: top1={got} want={want} top5={[h['file'] for h in hits]}"
            )
            return f"top1 {got}"
        check(f"B top-1: {q[:36]}", t)

    def junk_returns_nothing():
        hits = jrun(["search", "quantum banana teleport protocol", "-l", "5"], root, data, vectors)
        assert not hits, f"junk query returned {[h['file'] for h in hits]}"
        return "junk query returns zero hits"

    check("B junk query returns nothing", junk_returns_nothing)

    def filename_query():
        hits = jrun(["search", "OrderService.java", "--no-vector", "-l", "3"], root, data, vectors)
        assert hits, "no hits"
        assert hits[0]["file"].endswith("OrderService.java"), [h["file"] for h in hits]
        return "filename query resolves to the file"

    check("B filename query hits the right file (BM25 leg alive)", filename_query)

    def java_method_lookup():
        hits = jrun(["search", "taxOn", "--no-vector", "-l", "3"], root, data, vectors)
        assert hits and "OrderService.java" in hits[0]["file"], [h["file"] for h in hits]
        return "java method name found via BM25"

    check("B java symbols are searchable by name", java_method_lookup)


# -- C. call graph -------------------------------------------------------------
def suite_trace(root, data):
    def callers():
        out = run_imc(["trace", "generate_invoice", "-d", "callers"], root, data).stdout
        assert "export_invoice" in out, out[:400]
        return "export_invoice shows up as a caller"

    check("C1 callers of a python function", callers)

    def callees():
        out = run_imc(["trace", "generate_invoice", "-d", "callees"], root, data).stdout
        assert "calculate_tax" in out, out[:400]
        return "callees resolve to method calls"

    check("C2 callees include the method call (attribute-call layer alive)", callees)

    def blast_radius():
        out = run_imc(["trace", "calculate_tax", "-d", "callers", "--depth", "3"], root, data).stdout
        assert "audit_invoice" in out and "generate_invoice" in out, out[:400]
        return "both callers found across an attribute-call edge"

    check("C3 blast radius across method callers", blast_radius)

    def java_callers():
        out = run_imc(["trace", "add", "-d", "callers"], root, data).stdout
        assert "taxOn" in out, out[:400]
        return "java method_invocation edge resolves"

    check("C4 java method_invocation edges are traced", java_callers)

    def go_selector():
        out = run_imc(["trace", "NewServer", "-d", "callers"], root, data).stdout
        assert "main" in out, out[:400]
        return "go selector_expression edge resolves"

    check("C5 go selector calls are traced", go_selector)

    def unknown_symbol():
        p = run_imc(["trace", "definitely_not_a_symbol"], root, data, expect_rc=1)
        assert "not found" in (p.stdout + p.stderr).lower(), (p.stdout + p.stderr)[:200]
        return "rc=1 with a helpful message"

    check("C6 unknown symbol exits 1", unknown_symbol)


# -- D. reader -----------------------------------------------------------------
def suite_reader(root, data):
    def symbols_outline():
        out = run_imc(["read", "src/billing.py", "--symbols"], root, data).stdout
        assert "calculate_tax" in out and "generate_invoice" in out, out[:400]
        return "outline lists methods"

    check("D1 read --symbols lists methods", symbols_outline)

    def method_extraction():
        out = run_imc(["read", "src/billing.py", "--method", "apply_discount"], root, data).stdout
        assert "return amount * (1 - percent)" in out, out[:400]
        return "single method body returned"

    check("D2 read --method extracts one method", method_extraction)

    def java_read():
        out = run_imc(["read", "src/main/java/com/shop/OrderService.java", "--symbols"], root, data).stdout
        assert "OrderService" in out and "taxOn" in out, out[:400]
        return "java outline"

    check("D3 read --symbols works for java", java_read)

    def truncation_header():
        (root / "big.txt").write_text("\n".join(f"line {i}" for i in range(20000)))
        out = run_imc(["read", "big.txt"], root, data).stdout
        assert len(out) < 70000, f"no truncation: {len(out)} chars"
        return f"truncated to {len(out)} chars"

    check("D4 read truncates an oversized file", truncation_header)

    def missing_file():
        p = run_imc(["read", "nope.txt"], root, data, expect_rc=1)
        assert "not found" in (p.stdout + p.stderr).lower()
        return "rc=1"

    check("D5 read on a missing file exits 1", missing_file)


# -- E. memory -----------------------------------------------------------------
def suite_memory(vectors, root, data):
    def typed_add_and_list():
        run_imc(
            [
                "memory-add",
                "Always run rustfmt before committing",
                "--type", "business_rule",
                "--scene", "tooling",
                "--importance", "0.9",
                "-p", "src/engine.rs",
            ],
            root, data, vectors,
        )
        rows = jrun(["memory-list", "-l", "50"], root, data, vectors)
        hit = next(r for r in rows if "rustfmt" in r["content"])
        assert hit["mem_type"] == "business_rule", hit
        assert hit["scene"] == "tooling", hit
        assert hit["paths"] == ["src/engine.rs"], hit
        return "type / scene / paths round-trip"

    check("E1 memory-add stores type, scene and paths", typed_add_and_list)

    def recall_hits():
        hits = jrun(["memory-search", "rustfmt before committing", "-l", "3"], root, data, vectors)
        assert hits and "rustfmt" in hits[0]["memory"]["content"], hits
        return f"top score {hits[0]['score']:.3f}"

    check("E2 memory-search recalls the rule", recall_hits)

    def recall_reinforces():
        before = jrun(["memory-search", "rustfmt", "-l", "1"], root, data, vectors)[0]["memory"]["use_count"]
        jrun(["memory-search", "rustfmt", "-l", "1"], root, data, vectors)
        after = jrun(["memory-search", "rustfmt", "-l", "1"], root, data, vectors)[0]["memory"]["use_count"]
        assert after == before + 2, (before, after)
        return f"use_count {before} -> {after}"

    check("E3 recall reinforces usage", recall_reinforces)

    def low_importance_retrievable():
        for i in range(12):
            run_imc(
                ["memory-add", f"filler memory {i} about unrelated topics", "--importance", "0.9"],
                root, data, vectors,
            )
        run_imc(
            ["memory-add", "zebra migration needs a double flip", "--importance", "0.01"],
            root, data, vectors,
        )
        hits = jrun(["memory-search", "zebra migration", "-l", "3"], root, data, vectors)
        assert hits and "zebra" in hits[0]["memory"]["content"], [
            h["memory"]["content"] for h in hits
        ]
        return "lowest-importance memory still ranks first"

    check("E4 low-importance memory is still retrievable", low_importance_retrievable)

    def prune_on_index():
        before = len(jrun(["memory-list", "-l", "999"], root, data, vectors))
        (root / "src/engine.rs").unlink()
        pruned = jrun(["index", "."], root, data, vectors).get("memories_pruned")
        assert pruned == 1, f"expected the rustfmt rule (tracked src/engine.rs) pruned, got {pruned}"
        rows = jrun(["memory-list", "-l", "999"], root, data, vectors)
        assert len(rows) == before - 1, (len(rows), before)
        assert any("zebra" in r["content"] for r in rows), "prune touched a pathless memory"
        return "stale pruned, pathless kept"

    check("E5 index prunes stale memories only", prune_on_index)

    def prune_command_idempotent():
        out = run_imc(["memory-prune"], root, data, vectors).stdout
        assert "pruned 0" in out, out
        return "idempotent"

    check("E6 memory-prune is idempotent", prune_command_idempotent)

    def junk_memory_query():
        hits = jrun(["memory-search", "quantum banana teleport protocol", "-l", "5"], root, data, vectors)
        assert not hits, f"junk memory query returned {len(hits)} hits"
        return "junk memory query returns nothing"

    check("E7 memory relevance floor holds", junk_memory_query)


# -- F. claude wiring ----------------------------------------------------------
def suite_claude(root, data):
    def writes_settings():
        run_imc(["claude", "wire", "--port", "12345"], root, data)
        s = json.loads((root / ".claude" / "settings.local.json").read_text())
        assert s["env"]["ANTHROPIC_BASE_URL"] == "http://127.0.0.1:12345", s
        assert s["env"]["ANTHROPIC_MODEL"] == "claude-pro-max", s
        return "base_url points at the proxy"

    check("F1 claude wire writes .claude/settings.local.json", writes_settings)

    def writes_md():
        md = (root / "CLAUDE.md").read_text()
        assert md.count("<!-- imc:begin -->") == 1, md[:200]
        assert "imc search" in md and "imc trace" in md
        return "instruction block present"

    check("F2 claude wire writes the CLAUDE.md block", writes_md)

    def idempotent():
        run_imc(["claude", "wire"], root, data)
        run_imc(["claude", "wire"], root, data)
        md = (root / "CLAUDE.md").read_text()
        assert md.count("<!-- imc:begin -->") == 1, f"markers duplicated: {md.count('<!-- imc:begin -->')}"
        return "3 runs -> 1 block"

    check("F3 wiring is idempotent", idempotent)

    def preserves_settings():
        p = root / ".claude" / "settings.local.json"
        s = json.loads(p.read_text())
        s["env"]["MY_TOKEN"] = "keep-me"
        s["permissions"] = {"allow": ["Bash"]}
        p.write_text(json.dumps(s, indent=2))
        run_imc(["claude", "wire"], root, data)
        after = json.loads(p.read_text())
        assert after["env"]["MY_TOKEN"] == "keep-me", after
        assert after["permissions"] == {"allow": ["Bash"]}, after
        assert after["env"]["ANTHROPIC_BASE_URL"] == "http://127.0.0.1:12345"
        return "user env + unrelated keys preserved"

    check("F4 wiring never clobbers user settings", preserves_settings)

    def preserves_md():
        (root / "CLAUDE.md").write_text("# My project rules\n\nAlways run tests.\n")
        run_imc(["claude", "wire"], root, data)
        md = (root / "CLAUDE.md").read_text()
        assert "Always run tests." in md and md.count("<!-- imc:begin -->") == 1
        return "existing content kept"

    check("F5 wiring preserves existing CLAUDE.md", preserves_md)


# -- G. config -----------------------------------------------------------------
def suite_config():
    d = Path(tempfile.mkdtemp(prefix="imc-e2e-cfg-"))
    env = imc_env(d, False)

    def cfg(*args, expect_rc=0):
        p = subprocess.run(
            [str(IMC), "config", *args], cwd=str(d), env=env,
            capture_output=True, text=True, timeout=60,
        )
        if expect_rc is not None and p.returncode != expect_rc:
            raise AssertionError(f"rc={p.returncode} for config {' '.join(args)}: {p.stderr[:300]}")
        return p

    def roundtrip():
        cfg("set", "llm.base_url", "https://example.test/v1")
        cfg("set", "llm.api_key", "sk-secret-1234567890")
        cfg("set", "embedding.model", "text-embedding-3-small")
        got = cfg("get", "llm.base_url").stdout.strip()
        assert got == "https://example.test/v1", got
        masked = cfg("get", "llm.api_key").stdout.strip()
        assert "1234567890" not in masked and "*" in masked, masked
        return "set/get roundtrip, secret masked"

    check("G1 config set/get roundtrip with masking", roundtrip)

    def yaml_shape():
        txt = (d / "config.yaml").read_text()
        for pat in (r"^llm:", r"^\s+base_url:", r"^\s+api_key:", r"^embedding:", r"^\s+model:"):
            assert re.search(pat, txt, re.M), f"{pat} missing from:\n{txt}"
        return "same section shape as the Python config"

    check("G2 config file has the Python shape", yaml_shape)

    def migrate_safe():
        out = cfg("migrate").stdout
        assert "nothing to migrate" in out or "migrated" in out, out
        return out.strip()[:60]

    check("G3 config migrate is safe when there is nothing to move", migrate_safe)

    def test_unreachable():
        p = cfg("test", expect_rc=1)
        assert "FAIL" in p.stdout, p.stdout[:300]
        return "unreachable endpoint reports FAIL"

    check("G4 config test reports an unreachable endpoint", test_unreachable)


# -- H. proxy lifecycle --------------------------------------------------------
def suite_proxy(root, data):
    if not PROXYD.exists():
        skip("H1 proxy start/status/stop", f"{PROXYD.name} not built")
        return

    def lifecycle():
        env = imc_env(data, False)
        start = subprocess.run(
            [str(IMC), "claude", "start", "--port", "12399"], cwd=str(root),
            env=env, capture_output=True, text=True, timeout=120,
        )
        assert start.returncode == 0, start.stderr[:400]
        deadline = time.time() + 20
        up = False
        while time.time() < deadline:
            with socket.socket() as s:
                s.settimeout(0.3)
                if s.connect_ex(("127.0.0.1", 12399)) == 0:
                    up = True
                    break
            time.sleep(0.3)
        if not up:
            subprocess.run([str(IMC), "claude", "stop"], cwd=str(root), env=env, capture_output=True)
            raise AssertionError("proxy never opened port 12399")

        st = subprocess.run(
            [str(IMC), "claude", "status"], cwd=str(root), env=env,
            capture_output=True, text=True, timeout=60,
        )
        assert "RUNNING" in st.stdout, st.stdout[:300]

        stop = subprocess.run(
            [str(IMC), "claude", "stop"], cwd=str(root), env=env,
            capture_output=True, text=True, timeout=60,
        )
        assert "stopped" in stop.stdout, stop.stdout[:300]
        return "start -> port reachable -> status RUNNING -> stop"

    check("H1 proxy start/status/stop", lifecycle)


# -- I. json contract ----------------------------------------------------------
def suite_json(root, data):
    commands = [
        ["index", "."],
        ["search", "invoice tax", "-l", "3"],
        ["trace", "generate_invoice", "-d", "callers"],
        ["stats"],
        ["projects"],
        ["config", "list"],
        ["memory-list"],
        ["memory-search", "invoice", "-l", "2"],
        ["read", "src/billing.py", "--symbols"],
    ]

    def all_json():
        for cmd in commands:
            p = run_imc(["--json", *cmd], root, data, False)
            try:
                json.loads(p.stdout)
            except json.JSONDecodeError as e:
                raise AssertionError(
                    f"`imc --json {' '.join(cmd)}` is not JSON: {e}\n{p.stdout[:300]}"
                )
        return f"{len(commands)} commands emit valid JSON"

    check("I1 --json produces valid JSON everywhere", all_json)


# -- J. multi-language ---------------------------------------------------------
def suite_languages(root, data):
    cases = {
        "python class": ("InvoiceCalculator", "src/billing.py"),
        "python method": ("calculate_tax", "src/billing.py"),
        "typescript": ("reconcileLedger", "src/payments/ledger.ts"),
        "rust": ("spawn_engine", "src/engine.rs"),
        "go": ("NewServer", "cmd/server/main.go"),
        "java class": ("OrderService", "src/main/java/com/shop/OrderService.java"),
        "java method": ("taxOn", "src/main/java/com/shop/OrderService.java"),
    }

    def resolved():
        found = []
        for lang, (sym, want_file) in cases.items():
            out = run_imc(["--json", "trace", sym, "-d", "callers"], root, data).stdout
            assert want_file in out, f"{lang}: {want_file} not in trace output for {sym!r}"
            found.append(lang)
        return "resolved: " + ", ".join(found)

    check("J1 all five languages index and trace", resolved)

    def java_indexed():
        db = con(data, root)
        n = db.execute("SELECT COUNT(*) FROM files WHERE path LIKE '%.java'").fetchone()[0]
        syms = db.execute("SELECT COUNT(*) FROM symbols WHERE file LIKE '%.java'").fetchone()[0]
        db.close()
        assert n == 1, n
        assert syms >= 4, f"java symbols: {syms}"
        return f"{n} java file, {syms} java symbols"

    check("J2 java files produce AST symbols", java_indexed)

    def docs_indexed():
        db = con(data, root)
        n = db.execute("SELECT COUNT(*) FROM files WHERE path LIKE 'docs/%'").fetchone()[0]
        db.close()
        assert n == 1, n
        return "docs/*.md indexed as text"

    check("J3 non-code files are indexed too", docs_indexed)

    def per_language_symbols():
        db = con(data, root)
        rows = db.execute(
            "SELECT file, kind, COUNT(*) c FROM symbols GROUP BY file, kind ORDER BY file"
        ).fetchall()
        db.close()
        langs = {r[0].rsplit(".", 1)[-1] for r in rows}
        for ext in ("py", "ts", "rs", "go", "java"):
            assert ext in langs, f"no symbols for .{ext}: {sorted(langs)}"
        return f"{len(rows)} (file,kind) symbol groups across {len(langs)} extensions"


# -- main ----------------------------------------------------------------------
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--with-vectors", action="store_true",
                    help="also exercise the real embedding path (slow, needs a key)")
    ap.add_argument("--keep", action="store_true", help="keep fixture directories")
    args = ap.parse_args()

    if not IMC.exists():
        print(f"binary not found: {IMC}\nrun: cargo build --release")
        return 2

    print(f"\nimc e2e suite - {IMC.stat().st_size // 1024} KiB binary, vectors={args.with_vectors}\n")

    print("A. index lifecycle")
    root, data = suite_index(args.with_vectors)
    print("\nB. retrieval quality")
    suite_retrieval(args.with_vectors, root, data)
    print("\nC. call graph")
    suite_trace(root, data)
    print("\nD. reader")
    suite_reader(root, data)
    print("\nE. memory")
    suite_memory(args.with_vectors, root, data)
    print("\nF. claude wiring")
    suite_claude(root, data)
    print("\nG. config")
    suite_config()
    print("\nH. proxy lifecycle")
    suite_proxy(root, data)
    print("\nI. json contract")
    suite_json(root, data)
    print("\nJ. multi-language")
    # Suite E deletes a fixture file (prune_stale) and suite D adds a huge one,
    # so multi-language assertions get their own pristine project.
    lang_root = new_case("lang")
    lang_data = Path(tempfile.mkdtemp(prefix="imc-e2e-lang-"))
    jrun(["index", "."], lang_root, lang_data, args.with_vectors)
    suite_languages(lang_root, lang_data)
    if not args.keep:
        shutil.rmtree(lang_root, ignore_errors=True)
        shutil.rmtree(lang_data, ignore_errors=True)

    total = len(PASS) + len(FAIL)
    print("\n" + "=" * 64)
    print(f"  {len(PASS)}/{total} passed" + (f", {len(SKIP)} skipped" if SKIP else ""))
    if FAIL:
        print("\n  failures:")
        for n, e in FAIL:
            print(f"   - {n}: {e.splitlines()[0][:120]}")
    print("=" * 64)

    if args.keep:
        print(f"\nfixtures kept: {root}  data: {data}")
    else:
        shutil.rmtree(root, ignore_errors=True)
        shutil.rmtree(data, ignore_errors=True)
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())