//! Unit tests for `imc_memory::extract_candidates` with active_paths tracking.

use imc_memory::extract_candidates;

#[test]
fn extract_candidates_tracks_active_paths() {
    // Content that mentions the paths, so overlap tracking works.
    let text = r#"
RULE: Always validate JWT tokens in src/auth/middleware.rs before accepting requests.
Note: The database connection pool in src/db/pool.rs is sized at 20 connections.
We decided to use PostgreSQL as the primary database.
Some unrelated line.
"#;

    let active_paths = vec![
        "src/auth/middleware.rs".to_string(),
        "src/db/pool.rs".to_string(),
    ];

    let cands = extract_candidates(text, &active_paths);
    assert!(cands.len() >= 3, "expected at least 3 candidates, got {cands:?}");

    // The rule about JWT should track src/auth/middleware.rs (mentioned in content).
    let jwt_cand = cands
        .iter()
        .find(|c| c.content.contains("JWT"))
        .expect("JWT rule candidate");
    assert!(
        jwt_cand.paths.contains(&"src/auth/middleware.rs".to_string()),
        "JWT rule should track auth middleware path: {jwt_cand:?}"
    );

    // The note about connection pool should track src/db/pool.rs.
    let pool_cand = cands
        .iter()
        .find(|c| c.content.contains("connection pool"))
        .expect("pool note candidate");
    assert!(
        pool_cand.paths.contains(&"src/db/pool.rs".to_string()),
        "pool note should track db pool path: {pool_cand:?}"
    );
}

#[test]
fn extract_candidates_dedupes_by_normalized_content() {
    let text = r#"
RULE: Always validate input before processing.
RULE:   always validate input before processing.
NOTE: Some unique note here.
"#;

    let cands = extract_candidates(text, &[]);
    // The two RULE lines normalize to the same content, so only one should remain.
    let rule_count = cands.iter().filter(|c| c.category == "rule").count();
    assert_eq!(rule_count, 1, "duplicate rule should be deduped: {cands:?}");

    let note_count = cands.iter().filter(|c| c.category == "note").count();
    assert_eq!(note_count, 1, "note should be present: {cands:?}");
}

#[test]
fn extract_candidates_min_length_filter() {
    let text = r#"
RULE: OK.
NOTE: This is a sufficiently long note that should be kept.
"#;

    let cands = extract_candidates(text, &[]);
    let rule_count = cands.iter().filter(|c| c.category == "rule").count();
    assert_eq!(rule_count, 0, "short rule should be filtered: {cands:?}");
    assert!(
        cands.iter().any(|c| c.content.contains("sufficiently long")),
        "long note should be kept: {cands:?}"
    );
}
