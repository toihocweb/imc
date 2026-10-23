//! Repository Map generator: produces a concise, token-budgeted architectural
//! skeleton of the codebase from AST symbols and call graph dependencies.
//!
//! Designed for coding agents to grasp the multi-module structure, core
//! abstractions (classes, interfaces, traits, structs), and domain entrypoints
//! without drowning in trivial getters, setters, and HTTP verb boilerplate.

use anyhow::Result;
use imc_core::store::Store;
use imc_core::types::{Symbol, SymbolKind};
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

/// Identifies trivial CRUD verbs, boilerplate accessors, and utility names
/// that inflate graph metrics and clutter architectural summaries.
fn is_trivial_boilerplate(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "get"
            | "set"
            | "is"
            | "has"
            | "of"
            | "to"
            | "from"
            | "with"
            | "post"
            | "put"
            | "delete"
            | "patch"
            | "head"
            | "options"
            | "build"
            | "builder"
            | "create"
            | "new"
            | "init"
            | "run"
            | "execute"
            | "apply"
            | "call"
            | "send"
            | "recv"
            | "close"
            | "read"
            | "write"
            | "equals"
            | "hashcode"
            | "hash"
            | "tostring"
            | "clone"
            | "default"
            | "drop"
            | "constructor"
            | "val"
            | "var"
            | "data"
            | "self"
    ) || lower.starts_with("get_")
        || lower.starts_with("set_")
        || lower.starts_with("is_")
}

/// Is this an architectural symbol type (Class, Interface, Struct, Trait, Enum)?
fn is_primary_abstraction(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class
            | SymbolKind::Interface
            | SymbolKind::Struct
            | SymbolKind::Trait
            | SymbolKind::Enum
            | SymbolKind::Component
            | SymbolKind::Type
    )
}

/// Extract module prefix or top-level directory for a file path.
fn extract_module_group(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 1 {
        return "root".to_string();
    }
    // Handle monorepos / workspaces like `crates/imc-core` or `dhp-integration/src`
    if (parts[0] == "crates" || parts[0] == "packages" || parts[0] == "modules") && parts.len() > 2 {
        format!("{}/{}", parts[0], parts[1])
    } else {
        parts[0].to_string()
    }
}

/// Generate a high-signal architectural repository map within a token budget.
pub fn generate_repo_map(store: &Store, max_tokens: usize) -> Result<RepoMapReport> {
    let files = store.all_file_hashes();
    let total_files = files.len();

    // 1. Group files by module and extract relevant architectural symbols
    let mut module_file_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut file_data: Vec<(String, String, f64, Vec<Symbol>)> = Vec::new();

    for (rel, _) in &files {
        let module = extract_module_group(rel);
        *module_file_counts.entry(module.clone()).or_default() += 1;

        let defs = store.definitions_in_file(rel);
        if defs.is_empty() {
            continue;
        }

        let is_test = rel.contains("/tests/")
            || rel.contains("/test/")
            || rel.starts_with("tests/")
            || rel.contains("_test.")
            || rel.contains(".test.")
            || rel.contains("_spec.")
            || rel.contains(".spec.");

        // Calculate score based on architectural significance:
        // - Primary abstractions (classes, interfaces, structs) carry high weight
        // - Call-graph callers boost score ONLY for non-generic domain names
        let mut score: f64 = 0.0;
        let mut has_primary = false;

        for d in &defs {
            if is_primary_abstraction(d.kind) {
                score += 5.0;
                has_primary = true;
            } else if !is_trivial_boilerplate(&d.name) {
                score += 1.0;
                let callers = store.callers(&d.name);
                score += (callers.len() as f64) * 1.5;
            }
        }

        if !has_primary {
            score *= 0.5;
        }
        if is_test {
            score *= 0.1; // Heavily penalize test files
        }

        file_data.push((rel.clone(), module, score, defs));
    }

    // Sort files descending by structural score
    file_data.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    // Group ranked files by module to ensure breadth-first coverage
    let mut module_ranked_files: BTreeMap<String, Vec<(String, f64, Vec<Symbol>)>> = BTreeMap::new();
    for (rel, module, score, defs) in file_data {
        module_ranked_files.entry(module).or_default().push((rel, score, defs));
    }

    // 2. Render breadth-first skeleton within the token budget
    let char_budget = max_tokens.max(100) * 4;
    let mut current_chars = 0;
    let mut out_text = String::new();
    let mut shown_entries: Vec<RepoMapFileEntry> = Vec::new();

    let module_count = module_file_counts.len();
    out_text.push_str(&format!(
        "# Repository Map (Architectural Skeleton) — {total_files} files across {module_count} modules\n\n"
    ));
    current_chars += out_text.len();

    let mut shown_files = 0;

    // Iterate module by module to avoid starving other packages
    for (module, files_in_module) in &module_ranked_files {
        let total_in_mod = module_file_counts.get(module).copied().unwrap_or(files_in_module.len());
        let mod_header = format!("[{module}] ({total_in_mod} files)\n");

        if current_chars + mod_header.len() > char_budget && shown_files > 0 {
            break;
        }
        current_chars += mod_header.len();
        out_text.push_str(&mod_header);

        // Within each module, pick the top 3-5 most important files
        for (rel, score, defs) in files_in_module {
            let mut file_buf = String::new();
            let mut sym_entries: Vec<RepoMapSymbolEntry> = Vec::new();

            // Extract primary abstractions first, then at most 2 key domain methods
            let primaries: Vec<&Symbol> = defs.iter().filter(|s| is_primary_abstraction(s.kind)).collect();
            let secondary_methods: Vec<&Symbol> = defs
                .iter()
                .filter(|s| !is_primary_abstraction(s.kind) && !is_trivial_boilerplate(&s.name))
                .take(2)
                .collect();

            if primaries.is_empty() && secondary_methods.is_empty() {
                continue;
            }

            file_buf.push_str(&format!("  {rel}:\n"));

            for p in &primaries {
                sym_entries.push(RepoMapSymbolEntry {
                    name: p.name.clone(),
                    kind: p.kind.as_str().to_string(),
                    line: p.line,
                    parent: p.parent.clone(),
                });
                file_buf.push_str(&format!("    {} {}\n", p.kind.as_str(), p.name));
            }

            // Include at most 2 high-signal non-generic domain methods/functions
            for m in &secondary_methods {
                sym_entries.push(RepoMapSymbolEntry {
                    name: m.name.clone(),
                    kind: m.kind.as_str().to_string(),
                    line: m.line,
                    parent: m.parent.clone(),
                });
                let indent = if m.parent.is_some() { "      " } else { "    " };
                file_buf.push_str(&format!("{indent}{} {}\n", m.kind.as_str(), m.name));
            }

            if current_chars + file_buf.len() > char_budget && shown_files > 0 {
                break;
            }

            current_chars += file_buf.len();
            out_text.push_str(&file_buf);
            shown_files += 1;

            shown_entries.push(RepoMapFileEntry {
                file: rel.clone(),
                score: *score,
                symbols: sym_entries,
            });
        }
        out_text.push('\n');
    }

    if shown_files < total_files {
        let remaining = total_files - shown_files;
        out_text.push_str(&format!(
            "... [{remaining} additional files omitted to respect token budget]\n"
        ));
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
    fn repomap_filters_crud_and_groups_architectural_skeleton() {
        let db = tmp_db("test_skeleton");
        let mut store = store::open(&db).unwrap();

        store.upsert_file("client/api/Client.ts", "h1", Some("typescript"), 100).unwrap();
        store.upsert_file("server/auth/AuthService.java", "h2", Some("java"), 200).unwrap();

        // Client.ts has a class and trivial CRUD methods
        let sym_client = Symbol {
            id: "client/api/Client.ts::Client".into(),
            name: "Client".into(),
            qualname: "Client".into(),
            kind: SymbolKind::Class,
            file: "client/api/Client.ts".into(),
            line: 1,
            end_line: 20,
            start_byte: 0,
            end_byte: 100,
            parent: None,
        };
        let sym_get = Symbol {
            id: "client/api/Client.ts::Client::get".into(),
            name: "get".into(),
            qualname: "Client::get".into(),
            kind: SymbolKind::Method,
            file: "client/api/Client.ts".into(),
            line: 5,
            end_line: 7,
            start_byte: 20,
            end_byte: 30,
            parent: Some("Client".into()),
        };
        let sym_post = Symbol {
            id: "client/api/Client.ts::Client::post".into(),
            name: "post".into(),
            qualname: "Client::post".into(),
            kind: SymbolKind::Method,
            file: "client/api/Client.ts".into(),
            line: 8,
            end_line: 10,
            start_byte: 35,
            end_byte: 45,
            parent: Some("Client".into()),
        };
        store.insert_symbols("client/api/Client.ts", &[sym_client, sym_get, sym_post]).unwrap();

        // AuthService.java has an interface and domain methods
        let sym_auth = Symbol {
            id: "server/auth/AuthService.java::AuthService".into(),
            name: "AuthService".into(),
            qualname: "AuthService".into(),
            kind: SymbolKind::Interface,
            file: "server/auth/AuthService.java".into(),
            line: 1,
            end_line: 30,
            start_byte: 0,
            end_byte: 200,
            parent: None,
        };
        let sym_verify = Symbol {
            id: "server/auth/AuthService.java::AuthService::verifyCredentials".into(),
            name: "verifyCredentials".into(),
            qualname: "AuthService::verifyCredentials".into(),
            kind: SymbolKind::Method,
            file: "server/auth/AuthService.java".into(),
            line: 10,
            end_line: 20,
            start_byte: 50,
            end_byte: 150,
            parent: Some("AuthService".into()),
        };
        store.insert_symbols("server/auth/AuthService.java", &[sym_auth, sym_verify]).unwrap();

        // Even if Client.ts has 10 calls to "get", it must not pollute with get/post methods
        let edge = Edge {
            src: "server/auth/AuthService.java::AuthService::verifyCredentials".into(),
            dst: "get".into(),
            kind: EdgeKind::Call,
            file: "server/auth/AuthService.java".into(),
            line: 12,
        };
        store.insert_edges("server/auth/AuthService.java", &[edge]).unwrap();

        let report = generate_repo_map(&store, 1000).unwrap();

        // 1. Must show module headers
        assert!(report.text.contains("[client]"));
        assert!(report.text.contains("[server]"));

        // 2. Must show architectural abstractions
        assert!(report.text.contains("class Client"));
        assert!(report.text.contains("interface AuthService"));
        assert!(report.text.contains("method verifyCredentials"));

        // 3. MUST NOT list trivial get / post boilerplate!
        assert!(!report.text.contains("method get\n"));
        assert!(!report.text.contains("method post\n"));
    }
}
