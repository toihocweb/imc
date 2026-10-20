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

/// Adds a memory and returns the id, or -1 when the memory was not stored
/// (empty content, or a near duplicate of an existing one).
fn try_add_mem(
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
    .unwrap_or(-1)
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

#[test]
fn unrelated_query_does_not_return_a_memory() {
    // Regression: the FTS leg expanded every query token into `"tok" OR
    // "tok"*`, so the two-letter word "is" prefix-matched "iso" and one
    // spurious hit scored 1.0 after normalisation. The memory then passed the
    // proxy's relevance gate and was injected into unrelated requests.
    let store = make_store("unrelated_query");
    add_mem(
        &store,
        "Changelog entries use ISO 8601 dates",
        MemoryType::BusinessRule,
        "",
        &[],
        0.5,
    );

    let hits = retrieve(
        &store,
        None,
        "what is the deployment pipeline for kubernetes",
        &[],
        None,
        None,
        5,
    )
    .unwrap();
    assert!(
        hits.is_empty(),
        "unrelated query must not match a memory: {hits:?}"
    );

    // The matching query still finds it.
    let hits = retrieve(&store, None, "changelog iso dates", &[], None, None, 5).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
}

/// Decay is a multiplicative stage: two memories with the same content and
/// the same importance rank differently once one of them has not been used for
/// months. The additive recency term (weight 0.10) could not do that.
#[test]
fn decay_reorders_two_identical_memories() {
    let store = make_store("decay");
    let fresh = add_mem(
        &store,
        "deploy pipeline runs on ci",
        MemoryType::WorkFact,
        "ci",
        &[],
        0.7,
    );
    let stale = add_mem(
        &store,
        "kubernetes rollout waits for the health probe to pass",
        MemoryType::WorkFact,
        "ci",
        &[],
        0.7,
    );

    let now = imc_memory::now();
    // The stale one was last used four months ago.
    store
        .conn
        .execute(
            "UPDATE memories SET last_used = ?1 WHERE id = ?2",
            rusqlite::params![now - 120 * 86_400, stale],
        )
        .unwrap();

    let hits = retrieve(&store, None, "deploy pipeline kubernetes rollout health probe", &[], None, None, 5).unwrap();
    assert!(hits.len() >= 2, "{hits:?}");
    assert_eq!(hits[0].memory.id, fresh, "{hits:?}");
    assert!(
        hits[0].rank_score > hits[1].rank_score,
        "decay must reorder: {:?}",
        hits.iter().map(|h| (h.memory.id, h.rank_score)).collect::<Vec<_>>()
    );
}

/// Business rules are decisions, not observations: they do not expire.
#[test]
fn business_rules_do_not_decay() {
    let store = make_store("decay_rule");
    let rule = add_mem(
        &store,
        "always run cargo test before pushing",
        MemoryType::BusinessRule,
        "",
        &[],
        0.7,
    );
    let now = imc_memory::now();
    store
        .conn
        .execute(
            "UPDATE memories SET last_used = ?1 WHERE id = ?2",
            rusqlite::params![now - 200 * 86_400, rule],
        )
        .unwrap();

    let hits = retrieve(&store, None, "cargo test pushing", &[], None, None, 5).unwrap();
    assert!(!hits.is_empty(), "{hits:?}");
    assert!(
        (hits[0].rank_score - hits[0].score).abs() < 1e-9,
        "a business rule must not be scaled: {} vs {}",
        hits[0].rank_score,
        hits[0].score
    );
}

/// The same fact written twice with different wording must not become two
/// memories. The second write reinforces the first instead: it counts as
/// used, it counts as seen recently, and it keeps its importance.
#[test]
fn a_near_duplicate_reinforces_the_stored_memory() {
    let store = make_store("near_dup");
    let first = add_mem(
        &store,
        "the deploy pipeline runs on the ci runner",
        MemoryType::WorkFact,
        "ci",
        &[],
        0.6,
    );
    let second = try_add_mem(
        &store,
        "the deploy pipeline runs on the ci runner, on every push",
        MemoryType::WorkFact,
        "ci",
        &[],
        0.9,
    );
    assert!(first > 0, "the first memory is stored");
    assert!(second < 0, "the near duplicate is not stored as a second memory");

    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "one memory, not two");

    let (use_count, importance, last_used): (i64, f64, i64) = store
        .conn
        .query_row(
            "SELECT use_count, importance, last_used FROM memories WHERE id = ?1",
            rusqlite::params![first],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(use_count >= 1, "the existing memory was reinforced: {use_count}");
    assert!(importance >= 0.6, "importance kept: {importance}");
    assert!(last_used > 0, "seen recently: {last_used}");
}
