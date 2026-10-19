//! Unit tests for imc-memory retrieval: type/scene filtering, relevance gate,
//! overlap boost, and duplicate rejection. Exercises `imc_memory::retrieve` and
//! `imc_memory::recall` directly without the CLI.

use imc_core::store;
use imc_memory::{retrieve, recall, MemoryType};
use imc_embed::HashingEmbedder;

fn tmp_db(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("imc-memunit-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn make_store(tag: &str) -> imc_core::store::Store {
    let db = tmp_db(tag);
    store::open(&db).unwrap()
}

fn add_mem(
    store: &imc_core::store::Store,
    content: &str,
    mem_type: MemoryType,
    scene: &str,
    paths: &[&str],
    importance: f64,
) -> i64 {
    let paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
    imc_memory::add_typed(
        store,
        content,
        mem_type,
        scene,
        "test_category",
        &paths,
        &[],
        importance,
        "test",
    )
    .unwrap()
    .unwrap()
}

#[test]
fn mem_type_filter_excludes_non_matching() {
    let store = make_store("typefilter");
    add_mem(&store, "deploy pipeline rustfmt", MemoryType::WorkTask, "ci", &["src/ci.rs"], 0.8);
    add_mem(
        &store,
        "calculate tax rate amount",
        MemoryType::WorkFact,
        "billing",
        &["src/billing.rs"],
        0.9,
    );

    // Only work_task memories should match.
    let hits = retrieve(&store, None, "deploy pipeline", &[], Some("work_task"), None, 5).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].memory.mem_type, MemoryType::WorkTask);

    // Only work_fact memories should match.
    let hits = retrieve(&store, None, "calculate tax", &[], Some("work_fact"), None, 5).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].memory.mem_type, MemoryType::WorkFact);
}

#[test]
fn scene_filter_excludes_non_matching() {
    let store = make_store("scene_filter");
    add_mem(
        &store,
        "deploy pipeline rustfmt",
        MemoryType::WorkTask,
        "ci",
        &["src/ci.rs"],
        0.8,
    );
    add_mem(
        &store,
        "deploy pipeline rustfmt prod",
        MemoryType::WorkTask,
        "prod",
        &["src/ci.rs"],
        0.8,
    );

    let hits = retrieve(&store, None, "deploy pipeline", &[], None, Some("ci"), 5).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].memory.scene, "ci");

    let hits = retrieve(&store, None, "deploy pipeline", &[], None, Some("prod"), 5).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].memory.scene, "prod");
}

#[test]
fn overlap_boost_ranks_context_memory_higher() {
    let store = make_store("overlap");
    let embedder = HashingEmbedder::new(384);

    // Two memories with different content but both match "fix the login".
    add_mem(&store, "fix the login bug in auth module", MemoryType::WorkTask, "bugfix", &["src/auth.rs"], 0.5);
    add_mem(
        &store,
        "fix the login bug in payments module",
        MemoryType::WorkTask,
        "bugfix",
        &["src/payments.rs"],
        0.5,
    );
    embed_and_store_test(&store, &embedder);

    // Query with active_paths including src/auth.rs -> that memory gets +0.10 overlap boost and ranks first.
    let hits = recall(
        &store,
        Some(&embedder),
        "fix the login bug",
        &["src/auth.rs".to_string()],
        None,
        None,
        5,
    )
    .unwrap();
    assert!(!hits.is_empty(), "{hits:?}");
    assert_eq!(hits[0].memory.paths, vec!["src/auth.rs".to_string()], "auth.rs should rank first due to overlap: {hits:?}");
}

fn embed_and_store_test(store: &imc_core::store::Store, embedder: &HashingEmbedder) {
    let all = imc_memory::list_all(store, 10_000).unwrap();
    for m in all {
        imc_memory::embed_and_store(store, Some(embedder), m.id, &m.content).unwrap();
    }
}

#[test]
fn nonsense_query_returns_nothing() {
    let store = make_store("nonsense");
    add_mem(
        &store,
        "filler memory about unrelated topics",
        MemoryType::WorkFact,
        "",
        &[],
        0.9,
    );

    let hits = retrieve(
        &store,
        None,
        "quantum banana teleport protocol",
        &[],
        None,
        None,
        5,
    )
    .unwrap();
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn duplicate_content_rejected() {
    let store = make_store("dup");
    let _id1 = add_mem(
        &store,
        "Rule: never push to main",
        MemoryType::WorkFact,
        "ci",
        &[],
        0.8,
    );
    let id2 = imc_memory::add_typed(
        &store,
        "Rule: never push to main",
        MemoryType::WorkFact,
        "ci",
        "test_category",
        &[],
        &[],
        0.5,
        "test",
    )
    .unwrap();
    assert!(id2.is_none(), "second insert of same content should be None, got {id2:?}");
}

#[test]
fn recall_reinforces_use_count() {
    let store = make_store("reinforce");
    let embedder = HashingEmbedder::new(384);
    let id = add_mem(
        &store,
        "unique memory content xyz",
        MemoryType::WorkFact,
        "",
        &[],
        0.5,
    );
    embed_and_store_test(&store, &embedder);

    let before: i64 = imc_memory::get(&store, id).unwrap().unwrap().use_count;

    let hits = recall(
        &store,
        Some(&embedder),
        "unique memory content xyz",
        &[],
        None,
        None,
        5,
    )
    .unwrap();
    assert!(!hits.is_empty());

    let after: i64 = imc_memory::get(&store, id).unwrap().unwrap().use_count;
    assert_eq!(after, before + 1, "use_count should increment on recall");
}
