//! Adapter: exposes the ported `imc-trace` engine through `trace_cmd::Tracer`,
//! so `run_trace` — the faithful port of Python's `main/cli/commands/trace.py` —
//! drives the real engine instead of a fake.

use std::path::Path;

use serde_json::Value;

use imc_core::store::Store;
use imc_trace::tree::{self, CalleeNode, CallerNode, TraceOptions as TreeOptions};

use crate::trace_cmd::{Candidate, Direction, Tracer};

pub struct ImcTracer<'a> {
    inner: tree::Tracer<'a>,
    store: &'a Store,
    project_name: Option<String>,
}

impl<'a> ImcTracer<'a> {
    /// `indexed` is false when no project DB exists for `root` — that maps to
    /// Python's "no indexed project for current directory" failure.
    pub fn new(store: &'a Store, root: &Path, indexed: bool) -> Self {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| root.to_string_lossy().to_string());
        Self {
            inner: tree::Tracer::new(store),
            store,
            project_name: indexed.then_some(name),
        }
    }
}

impl Tracer for ImcTracer<'_> {
    fn project_name(&self) -> Option<String> {
        self.project_name.clone()
    }

    /// Python: `not tracer.graph` — the AST graph exists once symbols are indexed.
    fn has_graph(&self) -> bool {
        self.store.count_symbols() > 0
    }

    fn resolve_target(&self, target: &str, file_filter: Option<&str>) -> Vec<Candidate> {
        self.inner
            .resolve_target(target, file_filter)
            .into_iter()
            .map(|t| Candidate {
                id: t.id.clone(),
                data: serde_json::to_value(&t).unwrap_or(Value::Null),
            })
            .collect()
    }

    fn trace_callees(&self, nid: &str, max_depth: usize, show_all: bool) -> Value {
        let opts = TreeOptions::new().with_depth(max_depth).with_show_all(show_all);
        serde_json::to_value(self.inner.trace_callees(nid, &opts)).unwrap_or(Value::Null)
    }

    fn trace_callers(&self, raw_target_name: &str, target_file: Option<&str>, max_depth: usize) -> Value {
        // Python's callers path has no `--all`; builtin filtering stays on.
        let opts = TreeOptions::new().with_depth(max_depth);
        serde_json::to_value(self.inner.trace_callers(raw_target_name, target_file, &opts))
            .unwrap_or(Value::Null)
    }

    /// Known == it has a definition, or it is called from somewhere. Builtins
    /// (`print`) and imported helpers satisfy the second half; a typo satisfies
    /// neither and correctly reports "symbol not found".
    fn target_is_known(&self, target: &str) -> bool {
        if !self.store.find_symbols_by_name(target, 1).is_empty() {
            return true;
        }
        !self.store.callers(target).is_empty()
    }

    fn format_ascii_tree(&self, tree: &Value, direction: Direction) -> String {
        match direction {
            Direction::Callees => serde_json::from_value::<CalleeNode>(tree.clone())
                .map(|t| tree::format_callee_tree(&t))
                .unwrap_or_default(),
            Direction::Callers => serde_json::from_value::<CallerNode>(tree.clone())
                .map(|t| tree::format_caller_tree(&t))
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imc_core::store;
    use imc_core::types::{Edge, EdgeKind, Symbol, SymbolKind};

    fn tmp_db(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("imc-adapter-{tag}-{}.db", std::process::id()))
    }

    fn seed(path: &std::path::Path) -> Store {
        let _ = std::fs::remove_file(path);
        let mut store = store::open(path).unwrap();
        // `Tracer::new` indexes via `files` -> `symbols_in_file`, so the row
        // must exist (a real index always writes it during `imc index`).
        store.upsert_file("a.py", "deadbeef", Some("python"), 64).unwrap();
        store
            .insert_symbols(
                "a.py",
                &[Symbol {
                    id: Symbol::make_id("a.py", "alpha"),
                    name: "alpha".into(),
                    qualname: "alpha".into(),
                    kind: SymbolKind::Function,
                    file: "a.py".into(),
                    line: 1,
                    end_line: 2,
                    start_byte: 0,
                    end_byte: 10,
                    parent: None,
                }],
            )
            .unwrap();
        store
            .insert_edges(
                "a.py",
                &[Edge {
                    src: Symbol::make_id("a.py", "alpha"),
                    dst: "beta".into(),
                    kind: EdgeKind::Call,
                    file: "a.py".into(),
                    line: 2,
                }],
            )
            .unwrap();
        store
    }

    #[test]
    fn unindexed_project_reports_no_project() {
        let db = tmp_db("none");
        let store = store::open(&db).unwrap();
        let tracer = ImcTracer::new(&store, Path::new("/tmp/proj"), false);
        assert!(tracer.project_name().is_none(), "unindexed -> NoProject");
        let err =
            crate::trace_cmd::run_trace(&tracer, &crate::trace_cmd::TraceOptions::new("x")).unwrap_err();
        assert_eq!(err, crate::trace_cmd::TraceError::NoProject);
        assert_eq!(err.exit_code(), 1);
        assert!(err.messages()[0].contains("No indexed project"));
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn drives_real_engine_end_to_end() {
        let db = tmp_db("real");
        let store = seed(&db);

        let tracer = ImcTracer::new(&store, Path::new("/tmp/proj"), true);
        assert_eq!(tracer.project_name().as_deref(), Some("proj"));
        assert!(tracer.has_graph(), "symbol rows mean the graph is usable");

        let out =
            crate::trace_cmd::run_trace(&tracer, &crate::trace_cmd::TraceOptions::new("alpha")).unwrap();
        assert!(out.text.contains("alpha"), "{}", out.text);

        let mut rev = crate::trace_cmd::TraceOptions::new("beta");
        rev.callers = true;
        let out = crate::trace_cmd::run_trace(&tracer, &rev).unwrap();
        assert!(out.text.contains("alpha"), "{}", out.text);

        let mut js = crate::trace_cmd::TraceOptions::new("alpha");
        js.json_output = true;
        let out = crate::trace_cmd::run_trace(&tracer, &js).unwrap();
        let raw = out.json.as_deref().expect("json output");
        let v: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert_eq!(v["project"], "proj");
        assert!(v["results"][0]["mode"] == "callees", "{v}");

        let err = crate::trace_cmd::run_trace(
            &tracer,
            &crate::trace_cmd::TraceOptions::new("nope_xyz"),
        )
        .unwrap_err();
        assert!(matches!(err, crate::trace_cmd::TraceError::SymbolNotFound { .. }));
        assert_eq!(err.exit_code(), 1);

        let _ = std::fs::remove_file(&db);
    }
}
