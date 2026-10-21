//! End-to-end tests: parse → index → search, plus memory CRUD.

use std::fs;

use imc_core::{config::Config, store};
use imc_embed::HashingEmbedder;
use imc_search::{self, SearchOptions};

/// Hermetic config for tests: isolated data dir and **no** embedding endpoint,
/// so a real key in ~/.imc_config.yaml can never make tests hit the network.
fn test_cfg(root: &std::path::Path) -> Config {
    let mut cfg = Config::default();
    cfg.data_dir = root.join(".imc-data");
    cfg.embedding.api_key = None;
    cfg
}

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

    let cfg = test_cfg(&root);
    let stats = run_index(&cfg, &root, &[], true);
    assert!(stats.symbols >= 3, "expected symbols, got {}", stats.symbols);
    assert!(stats.edges >= 1, "expected call edges, got {}", stats.edges);

    let db = cfg.project_db(&root);
    let store = store::open(&db).unwrap();
    let hits = imc_search::search(
        &store,
        Some(&HashingEmbedder::new(384)),
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

    let cfg = test_cfg(&root);
    let stats = run_index(&cfg, &root, &[], true);
    assert!(stats.symbols >= 2, "symbols: {}", stats.symbols);
}

#[test]
fn incremental_index_skips_unchanged_files() {
    let root = temp_root("incr");
    fs::write(root.join("src/a.py"), "def alpha():\n    return 1\n").unwrap();

    let cfg = test_cfg(&root);
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

    let cfg = test_cfg(&root);
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

    let cfg = test_cfg(&root);
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
    imc_memory::embed_and_store(&store, Some(&embedder), id, "Rule: all indexing writes go through the FileIngester").unwrap();

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

    let hits = imc_memory::retrieve(&store, Some(&embedder), "indexing FileIngester", &["src/m.py".to_string()], None, None, 5).unwrap();
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

// ───────────────────────── memory retrieval regressions ─────────────────────────

fn mem_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("imc-memfix-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.py"), "def f():\n    return 1\n").unwrap();
    dir
}

/// Regression: the candidate query used to be `ORDER BY importance DESC LIMIT
/// limit*6`, so a low-importance memory was never scored at all — an exact
/// lexical hit lost to unrelated high-importance filler.
#[test]
fn low_importance_memory_is_still_retrievable() {
    let root = mem_root("lowimp");
    let cfg = test_cfg(&root);
    run_index(&cfg, &root, &[], false);
    let store = store::open(&cfg.project_db(&root)).unwrap();

    for i in 0..12 {
        imc_memory::add(
            &store,
            &format!("filler memory number {i} about unrelated topics"),
            "note",
            &[],
            &[],
            0.9,
            "test",
        )
        .unwrap();
    }
    let id = imc_memory::add(
        &store,
        "zebra migration requires double flip confirmation",
        "note",
        &[],
        &[],
        0.01,
        "test",
    )
    .unwrap()
    .expect("inserted");

    let hits = imc_memory::retrieve(&store, None, "zebra migration", &[], None, None, 5).unwrap();
    assert!(
        hits.first().map(|h| h.memory.id) == Some(id),
        "exact match must rank first despite 0.01 importance: {:?}",
        hits.iter().map(|h| (&h.memory.content, h.score)).collect::<Vec<_>>()
    );
}

/// Regression: cosine is never exactly 0, so without a relevance floor a
/// nonsense query returned the highest-importance filler.
#[test]
fn nonsense_query_returns_nothing() {
    let root = mem_root("nonsense");
    let cfg = test_cfg(&root);
    run_index(&cfg, &root, &[], false);
    let store = store::open(&cfg.project_db(&root)).unwrap();

    for i in 0..5 {
        imc_memory::add(
            &store,
            &format!("filler memory number {i} about unrelated topics"),
            "note",
            &[],
            &[],
            0.9,
            "test",
        )
        .unwrap();
    }
    let hits = imc_memory::retrieve(&store, None, "quantum banana teleport protocol", &[], None, None, 5).unwrap();
    assert!(hits.is_empty(), "unrelated query must not match: {hits:?}");
}

/// `recall` must reinforce, otherwise the usage/recency terms never move.
#[test]
fn recall_reinforces_returned_memories() {
    let root = mem_root("reinforce");
    let cfg = test_cfg(&root);
    run_index(&cfg, &root, &[], false);
    let store = store::open(&cfg.project_db(&root)).unwrap();
    let id = imc_memory::add(&store, "deploy pipeline must run rustfmt before commit", "note", &[], &[], 0.6, "test")
        .unwrap()
        .unwrap();

    assert_eq!(imc_memory::get(&store, id).unwrap().unwrap().use_count, 0);
    imc_memory::recall(&store, None, "deploy pipeline rustfmt", &[], None, None, 5).unwrap();
    let after = imc_memory::get(&store, id).unwrap().unwrap();
    assert_eq!(after.use_count, 1, "recall must bump use_count");
    assert!(after.last_used > 0, "recall must stamp last_used");
}

/// Overlap term: a memory tracking a context file must outrank an otherwise
/// equal memory that does not.
#[test]
fn context_path_overlap_boosts() {
    let root = mem_root("overlap");
    let cfg = test_cfg(&root);
    run_index(&cfg, &root, &[], false);
    let store = store::open(&cfg.project_db(&root)).unwrap();

    imc_memory::add(&store, "shared keyword alpha memory", "note", &[], &[], 0.5, "test").unwrap();
    let tracked = imc_memory::add(
        &store,
        "shared keyword beta memory",
        "note",
        &["src/a.py".to_string()],
        &[],
        0.5,
        "test",
    )
    .unwrap()
    .unwrap();

    let hits = imc_memory::retrieve(&store, None, "shared keyword", &["src/a.py".to_string()], None, None, 5).unwrap();
    assert_eq!(hits.first().map(|h| h.memory.id), Some(tracked), "overlap must win: {hits:?}");
    assert!(hits[0].matched_paths.contains(&"src/a.py".to_string()));
}

/// Pins the relevance floor itself: cosine below the measured 0.45 gap must be
/// rejected even though it is strictly greater than zero (which is what let a
/// nonsense query return filler when the real embedding endpoint was wired up).
#[test]
fn cosine_below_floor_is_rejected() {
    struct FakeEmbedder;
    impl imc_embed::Embedder for FakeEmbedder {
        fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| {
                    // Same-topic texts share a direction; unrelated ones are
                    // orthogonal, so cos is 1.0 or exactly 0.0.
                    let mut v = vec![0f32; 4];
                    if t.to_lowercase().contains("zebra") {
                        v[0] = 1.0;
                    } else if t.to_lowercase().contains("filler") {
                        v[1] = 1.0;
                    } else {
                        v[2] = 1.0;
                    }
                    v
                })
                .collect())
        }
        fn name(&self) -> String {
            "fake".into()
        }
        fn dim(&self) -> usize {
            4
        }
    }

    let root = mem_root("floor");
    let cfg = test_cfg(&root);
    run_index(&cfg, &root, &[], false);
    let store = store::open(&cfg.project_db(&root)).unwrap();

    let id = imc_memory::add(&store, "zebra migration needs care", "note", &[], &[], 0.5, "test")
        .unwrap()
        .unwrap();
    imc_memory::embed_and_store(
        &store,
        Some(&FakeEmbedder),
        id,
        "zebra migration needs care",
    )
    .unwrap();

    // Same topic -> cosine 1.0 -> accepted.
    let hits = imc_memory::retrieve(&store, Some(&FakeEmbedder), "zebra migration", &[], None, None, 5).unwrap();
    assert!(!hits.is_empty(), "same-topic must pass the floor");

    // Unrelated topic -> cosine 0.0 -> rejected even though importance is 0.5.
    let miss = imc_memory::retrieve(
        &store,
        Some(&FakeEmbedder),
        "total gibberish query",
        &[],
        None,
        None,
        5,
    )
    .unwrap();
    assert!(miss.is_empty(), "orthogonal query must not match: {miss:?}");
}

// ───────────────────── prompt claims must be real commands ─────────────────────

/// Every `imc <cmd>` and flag the shipped instruction block mentions must exist
/// in the binary that ships it. This asks the binary itself (`--help`) instead
/// of keeping a hand-written list: the previous version hard-coded the command
/// names and asserted the block taught `-d callers`, so it kept passing while
/// `-d` silently changed meaning from direction to depth, and while `callers`
/// and `--both` were added.
#[test]
fn rule_file_only_references_real_commands_and_flags() {
    fn help(args: &[&str]) -> String {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_imc"))
            .args(args)
            .arg("--help")
            .output()
            .expect("run `imc --help`");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    let block = imc_cli::claude::imc_rule_md();
    assert!(!block.is_empty(), "the rule file is empty");

    // Odd segments of a backtick split are the ones *inside* backticks — that is
    // where the prompt writes its commands.
    let mut referenced: Vec<(String, Vec<String>)> = Vec::new();
    for (i, inside) in block.split('`').enumerate() {
        if i % 2 == 0 {
            continue;
        }
        let mut words = inside.split_whitespace();
        if words.next() != Some("imc") {
            continue;
        }
        let Some(cmd) = words.next() else { continue };
        if cmd.starts_with('-') {
            continue;
        }
        let flags: Vec<String> = words
            .filter(|w| w.starts_with('-'))
            .map(|w| {
                w.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-')
                    .to_string()
            })
            .filter(|w| w.len() > 1)
            .collect();
        match referenced.iter_mut().find(|(c, _)| c == cmd) {
            Some((_, f)) => f.extend(flags),
            None => referenced.push((cmd.to_string(), flags)),
        }
    }
    assert!(!referenced.is_empty(), "no `imc <cmd>` references parsed");

    let top = help(&[]);
    for (cmd, flags) in &referenced {
        assert!(
            top.contains(format!("  {cmd}").as_str()) || top.contains(format!("{cmd} ").as_str()),
            "`imc {cmd}` is not a subcommand of this binary"
        );
        let h = help(&[cmd]);
        for f in flags {
            assert!(h.contains(f.as_str()), "`imc {cmd} {f}` is not a flag:\n{h}");
        }
    }
}

/// Names that only ever existed as Python wrappers must not creep back in.
#[test]
fn rule_file_omits_python_only_wrappers() {
    let block = imc_cli::claude::imc_rule_md();
    assert!(
        !block.contains("imc-read"),
        "`imc-read` is a Python console script, not a subcommand here"
    );
    // Direction is taught with the flags that exist, not with `-d`.
    assert!(block.contains("--callers"), "should teach `trace --callers`");
    assert!(
        !block.contains("-d callers"),
        "`-d` is depth (Python parity); direction is `--callers`"
    );
}

/// The commands the agent reaches for most must be in the block that the
/// agent actually reads. `imc pack` was missing from it: the long rule file
/// had it, the block written into the project did not, so the agent assembled
/// its context by hand and paid for it in tokens.
#[test]
fn rule_file_mentions_the_core_commands() {
    let block = imc_cli::claude::imc_rule_md();
    for cmd in [
        "imc search",
        "imc pack",
        "imc read",
        "imc context",
        "imc trace",
        "imc memory-search",
    ] {
        assert!(
            block.contains(cmd),
            "`{cmd}` is missing from the instruction block"
        );
    }
}

/// The two rule texts are written to different places and drift apart. The
/// short one (the instruction block) and the long one (the rule file) must
/// name the same commands.
#[test]
fn the_rule_texts_name_the_same_commands() {
    let short = imc_cli::claude::imc_rule_md();
    let long = imc_cli::claude::imc_rule_md();
    for cmd in [
        "imc search",
        "imc pack",
        "imc read",
        "imc context",
        "imc trace",
        "imc memory-search",
    ] {
        assert!(short.contains(cmd), "the instruction block misses `{cmd}`");
        assert!(long.contains(cmd), "the rule file misses `{cmd}`");
    }

    // The third copy: the rule file that is checked into docs/. The three
    // drift apart — the docs one had `imc pack` while the Rust ones did not.
    let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/imc-rule.md");
    let docs = std::fs::read_to_string(&docs).expect("docs/imc-rule.md");
    for cmd in [
        "imc search",
        "imc pack",
        "imc read",
        "imc context",
        "imc trace",
        "imc memory-search",
    ] {
        assert!(docs.contains(cmd), "docs/imc-rule.md misses `{cmd}`");
    }
}

/// `imc graph` was folded into `imc trace`: the definition is the root of the
/// tree, so one verb answers both questions.
#[test]
fn trace_carries_the_definition_and_the_graph_verb_is_gone() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_imc"))
        .args(["--help"])
        .output()
        .expect("run imc --help");
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(
        !help.contains("\n  graph "),
        "`imc graph` must be gone: {help}"
    );
}
