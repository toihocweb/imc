//! Repository Map generator: produces a concise, token-budgeted architectural
//! outline of the entire codebase from AST symbols and call graph dependencies.
//!
//! Inspired by Aider's repomap and Repo-mapper, but built natively on imc's
//! tree-sitter symbol database and SQLite call graph.

use anyhow::Result;
use imc_core::store::Store;
use imc_core::types::Symbol;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct RepoMapFileEntry {
    pub file: String,
    pub score: f64,
    pub symbols: Vec<RepoMapSymbolEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoMapSymbolEntry {
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoMapReport {
    pub files: Vec<RepoMapFileEntry>,
    pub total_files: usize,
    pub shown_files: usize,
    pub token_estimate: usize,
    pub text: String,
}

/// Generate a compact repository map within a token budget.
pub fn generate_repo_map(store: &Store, max_tokens: usize) -> Result<RepoMapReport> {
    let files = store.all_file_hashes();
    let total_files = files.len();

    // 1. Gather definitions per file and rank files by structural importance
    let mut ranked_files: Vec<(String, f64, Vec<Symbol>)> = Vec::new();

    for (rel, _) in &files {
        let defs = store.definitions_in_file(rel);
        if defs.is_empty() {
            continue;
        }

        // Calculate structural importance:
        // - Base weight: number of definitions
        // - In-degree boost: number of callers pointing to definitions in this file
        let mut in_degree = 0;
        for d in &defs {
            let callers = store.callers(&d.name);
            in_degree += callers.len();
        }

        let is_test = rel.contains("/tests/")
            || rel.contains("/test/")
            || rel.starts_with("tests/")
            || rel.contains("_test.")
            || rel.contains(".test.")
            || rel.contains("_spec.")
            || rel.contains(".spec.");

        let mut score = (defs.len() as f64) + (in_degree as f64 * 2.0);
        if is_test {
            score *= 0.2; // Deprioritize test files in architectural overview
        }

        ranked_files.push((rel.clone(), score, defs));
    }

    // Sort descending by score
    ranked_files.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // 2. Format entries while keeping within the token budget (~4 characters per token)
    let char_budget = max_tokens.max(100) * 4;
    let mut current_chars = 0;
    let mut out_text = String::new();
    let mut shown_entries: Vec<RepoMapFileEntry> = Vec::new();

    out_text.push_str("# Repository Map (Architectural Skeleton)\n\n");
    current_chars += out_text.len();

    let mut shown_files = 0;

    for (file, score, defs) in ranked_files {
        let mut file_buf = String::new();
        file_buf.push_str(&format!("{file}:\n"));

        // Organize symbols: parent -> children
        let mut parents: BTreeMap<String, Vec<&Symbol>> = BTreeMap::new();
        let mut top_level: Vec<&Symbol> = Vec::new();

        for sym in &defs {
            if let Some(p) = &sym.parent {
                parents.entry(p.clone()).or_default().push(sym);
            } else {
                top_level.push(sym);
            }
        }

        let mut sym_entries: Vec<RepoMapSymbolEntry> = Vec::new();

        for sym in top_level {
            sym_entries.push(RepoMapSymbolEntry {
                name: sym.name.clone(),
                kind: sym.kind.as_str().to_string(),
                line: sym.line,
                parent: None,
            });
            file_buf.push_str(&format!("  {} {}\n", sym.kind.as_str(), sym.name));

            if let Some(children) = parents.get(&sym.name) {
                for child in children {
                    sym_entries.push(RepoMapSymbolEntry {
                        name: child.name.clone(),
                        kind: child.kind.as_str().to_string(),
                        line: child.line,
                        parent: Some(sym.name.clone()),
                    });
                    file_buf.push_str(&format!("    {} {}\n", child.kind.as_str(), child.name));
                }
            }
        }

        // Also add any children whose parent was not in top_level
        for (p, children) in &parents {
            if !defs.iter().any(|s| s.parent.is_none() && &s.name == p) {
                file_buf.push_str(&format!("  parent {p}:\n"));
                for child in children {
                    sym_entries.push(RepoMapSymbolEntry {
                        name: child.name.clone(),
                        kind: child.kind.as_str().to_string(),
                        line: child.line,
                        parent: Some(p.clone()),
                    });
                    file_buf.push_str(&format!("    {} {}\n", child.kind.as_str(), child.name));
                }
            }
        }

        if current_chars + file_buf.len() > char_budget && shown_files > 0 {
            let remaining = total_files.saturating_sub(shown_files);
            if remaining > 0 {
                out_text.push_str(&format!("\n... [{remaining} additional files omitted to respect token budget]\n"));
            }
            break;
        }

        current_chars += file_buf.len();
        out_text.push_str(&file_buf);
        shown_files += 1;

        shown_entries.push(RepoMapFileEntry {
            file,
            score,
            symbols: sym_entries,
        });
    }

    let token_estimate = (current_chars + 3) / 4;

    Ok(RepoMapReport {
        files: shown_entries,
        total_files,
        shown_files,
        token_estimate,
        text: out_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use imc_core::store;
    use imc_core::types::{Edge, EdgeKind, Symbol, SymbolKind};

    fn tmp_db(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("imc-repomap-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn repomap_ranks_and_formats_skeleton() {
        let db = tmp_db("test");
        let mut store = store::open(&db).unwrap();

        store.upsert_file("src/main.rs", "h1", Some("rust"), 100).unwrap();
        store.upsert_file("src/auth.rs", "h2", Some("rust"), 200).unwrap();

        let sym_main = Symbol {
            id: "src/main.rs::main".into(),
            name: "main".into(),
            qualname: "main".into(),
            kind: SymbolKind::Function,
            file: "src/main.rs".into(),
            line: 1,
            end_line: 10,
            start_byte: 0,
            end_byte: 100,
            parent: None,
        };
        store.insert_symbols("src/main.rs", &[sym_main]).unwrap();

        let sym_auth_struct = Symbol {
            id: "src/auth.rs::AuthService".into(),
            name: "AuthService".into(),
            qualname: "AuthService".into(),
            kind: SymbolKind::Struct,
            file: "src/auth.rs".into(),
            line: 1,
            end_line: 20,
            start_byte: 0,
            end_byte: 200,
            parent: None,
        };
        let sym_login = Symbol {
            id: "src/auth.rs::AuthService::login".into(),
            name: "login".into(),
            qualname: "AuthService::login".into(),
            kind: SymbolKind::Method,
            file: "src/auth.rs".into(),
            line: 5,
            end_line: 15,
            start_byte: 50,
            end_byte: 150,
            parent: Some("AuthService".into()),
        };
        store.insert_symbols("src/auth.rs", &[sym_auth_struct, sym_login]).unwrap();

        // Edge: main calls login
        let edge = Edge {
            src: "src/main.rs::main".into(),
            dst: "login".into(),
            kind: EdgeKind::Call,
            file: "src/main.rs".into(),
            line: 5,
        };
        store.insert_edges("src/main.rs", &[edge]).unwrap();

        let report = generate_repo_map(&store, 1000).unwrap();
        assert_eq!(report.total_files, 2);
        assert_eq!(report.shown_files, 2);
        assert!(report.text.contains("src/auth.rs:"));
        assert!(report.text.contains("struct AuthService"));
        assert!(report.text.contains("method login"));
        assert!(report.text.contains("src/main.rs:"));
        assert!(report.text.contains("function main"));

        // AuthService should rank higher due to call edge from main
        assert_eq!(report.files[0].file, "src/auth.rs");
    }
}
