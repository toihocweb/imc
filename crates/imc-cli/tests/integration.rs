//! End-to-end tests: parse → index → search, plus memory CRUD.

use std::fs;

use imc_core::{config::Config, store};
use imc_embed::HashingEmbedder;
use imc_search::{self, SearchOptions};

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("imc-rs-test-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    dir
}

/// Thin unwrapping wrapper so tests read as plain stats.
fn run_index(
    cfg: &Config,
    root: &std::path::Path,
    ignores: &[String],
    force: bool,
) -> imc_cli::index::IndexStats {
    imc_cli::index::run(cfg, root, ignores, force).expect("index run")
}

#[test]
fn indexes_and_finds_python_symbols() {
    let root = temp_root("py");
    fs::write(
        root.join("src/app.py"),
        r#"
class PaymentGateway:
    def charge(self, amount):
        return self._validate(amount)

    def _validate(self, amount):
        if amount <= 0:
            raise ValueError("bad amount")
        return True


def process_refund(order_id):
    """Refund an order through the gateway."""
    gateway = PaymentGateway()
    gateway.charge(10)
    return order_id
"#,
    )
    .unwrap();

    let mut cfg = Config::default();
    cfg.data_dir = root.join(".imc-data");
    let stats = run_index(&cfg, &root, &[], true);
    assert!(stats.symbols >= 3, "expected symbols, got {}", stats.symbols);
    assert!(stats.edges >= 1, "expected call edges, got {}", stats.edges);

    let db = cfg.project_db(&root);
    let store = store::open(&db).unwrap();
    let hits = imc_search::search(
        &store,
        &HashingEmbedder::new(384),
        "refund order",
        &SearchOptions::default(),
    )
    .unwrap();
    assert!(!hits.is_empty(), "expected a hit");
    let joined: String = hits.iter().map(|h| h.text.clone()).collect();
    assert!(
        joined.contains("refund") || joined.contains("Refund"),
        "expected refund-related content, got: {joined:.300}"
    );
}

#[test]
fn indexes_rust_and_javascript() {
    let root = temp_root("multi");
    fs::write(
        root.join("src/lib.rs"),
        "pub fn compute_total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("src/app.tsx"),
        "export function RenderPanel() {\n  return <div>hi</div>;\n}\n",
    )
    .unwrap();

    let mut cfg = Config::default();
    cfg.data_dir = root.join(".imc-data");
    let stats = run_index(&cfg, &root, &[], true);
    assert!(stats.symbols >= 2, "symbols: {}", stats.symbols);
}

#[test]
fn incremental_index_skips_unchanged_files() {
    let root = temp_root("incr");
    fs::write(root.join("src/a.py"), "def alpha():\n    return 1\n").unwrap();

    let mut cfg = Config::default();
    cfg.data_dir = root.join(".imc-data");
    let first = run_index(&cfg, &root, &[], false);
    assert_eq!(first.new_files, 1);

    let second = run_index(&cfg, &root, &[], false);
    assert_eq!(second.new_files, 0);
    assert_eq!(second.modified_files, 0);
    assert_eq!(second.unchanged_files, 1);

    fs::write(root.join("src/a.py"), "def alpha():\n    return 2\n").unwrap();
    let third = run_index(&cfg, &root, &[], false);
    assert_eq!(third.modified_files, 1);
}

#[test]
fn deleted_files_are_dropped() {
    let root = temp_root("del");
    fs::write(root.join("src/keep.py"), "def keep():\n    return 1\n").unwrap();
    fs::write(root.join("src/gone.py"), "def gone():\n    return 1\n").unwrap();

    let mut cfg = Config::default();
    cfg.data_dir = root.join(".imc-data");
    run_index(&cfg, &root, &[], false);
    fs::remove_file(root.join("src/gone.py")).unwrap();
    let stats = run_index(&cfg, &root, &[], false);

    assert_eq!(stats.deleted_files, 1);
    let store = store::open(&cfg.project_db(&root)).unwrap();
    assert!(store
        .find_symbols_by_name("gone", 5)
        .is_empty());
}

#[test]
fn memory_crud_and_dedup() {
    let root = temp_root("mem");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/m.py"), "def f():\n    return 1\n").unwrap();

    let mut cfg = Config::default();
    cfg.data_dir = root.join(".imc-data");
    run_index(&cfg, &root, &[], false);
    let store = store::open(&cfg.project_db(&root)).unwrap();
    let embedder = HashingEmbedder::new(384);

    let id = imc_memory::add(
        &store,
        "Rule: all indexing writes go through the FileIngester",
        "rule",
        &["src/m.py".to_string()],
        &["indexing".to_string()],
        0.8,
        "test",
    )
    .unwrap()
    .expect("memory inserted");
    imc_memory::embed_and_store(&store, &embedder, id, "Rule: all indexing writes go through the FileIngester").unwrap();

    // Same content, different whitespace/case → duplicate rejected.
    let dup = imc_memory::add(
        &store,
        "rule:   all indexing writes go through the fileingester",
        "rule",
        &[],
        &[],
        0.5,
        "test",
    )
    .unwrap();
    assert!(dup.is_none(), "duplicate should be rejected");

    let hits = imc_memory::retrieve(&store, &embedder, "indexing FileIngester", &["src/m.py".to_string()], 5).unwrap();
    assert!(!hits.is_empty(), "expected memory recall");
    assert!(imc_memory::delete(&store, id).unwrap());
    assert!(imc_memory::list(&store, 5).unwrap().is_empty());
}

#[test]
fn extraction_finds_conventions_and_decisions() {
    let text = r#"
RULE: Never push directly to main; open a pull request instead.
Note: The Rust port stores embeddings as f32 blobs inside SQLite.
We decided to drop LanceDB and use SQLite FTS5 plus a vectors table.
Some unrelated line of prose.
"#;
    let cands = imc_memory::extract_candidates(text, &[]);
    assert!(cands.len() >= 3, "candidates: {cands:?}");
    assert!(cands.iter().any(|c| c.category == "rule"));
    assert!(cands.iter().any(|c| c.category == "decision"));
}