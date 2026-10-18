//! Port of `main/cli/commands/trace.py` (89 lines).
//!
//! CLI command: `imc trace <target> [--depth N] [--callers] [--both] [--all] [--json]`
//! — AST-based call hierarchy / execution-flow tracer.
//!
//! `main/core/tracer.py` is `ASTTracer`; the Rust `imc-trace` crate is still a
//! stub, so the tracer surface this command needs is expressed as the [`Tracer`]
//! trait below (same method names/shapes as the Python class). A real
//! implementation plugs in later without changing this module's logic.
//!
//! Pure logic: [`run_trace`] returns the text (or JSON) to print; the CLI prints
//! it to stdout, or [`TraceError::messages`] to stderr before exiting 1.

use serde_json::{json, Value};

/// Which direction a tree was walked in (mirrors `direction="callees"|"callers"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Callees,
    Callers,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::Callees => "callees",
            Direction::Callers => "callers",
        }
    }
}

/// One resolved target definition: `(node_id, node_data)` like Python's candidates.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub data: Value,
}

/// Everything `run_trace` accepts (the Python keyword arguments).
#[derive(Debug, Clone)]
pub struct TraceOptions {
    pub target: String,
    pub depth: usize,
    /// `--callers`
    pub callers: bool,
    /// reverse lookup (caller side); combined with `callers` via OR, as in Python
    pub reverse: bool,
    /// `--both`
    pub both: bool,
    /// `--all`
    pub show_all: bool,
    /// `--json`
    pub json_output: bool,
    /// `--project <name_or_id>`
    pub project: Option<String>,
    /// optional file scope for resolution / caller lookup
    pub file_filter: Option<String>,
}

impl TraceOptions {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            depth: 2,
            callers: false,
            reverse: false,
            both: false,
            show_all: false,
            json_output: false,
            project: None,
            file_filter: None,
        }
    }

    /// `is_reverse = callers or reverse`
    fn is_reverse(&self) -> bool {
        self.callers || self.reverse
    }
}

/// A resolved project + AST graph, matching the parts of `ASTTracer` trace.py uses.
pub trait Tracer {
    /// `None` when no indexed project covers the current directory (`not tracer.project`).
    fn project_name(&self) -> Option<String>;
    /// `not tracer.graph` when the AST property graph is missing.
    fn has_graph(&self) -> bool;
    /// `tracer.resolve_target(target, file_filter)` -> list of `(node_id, data)`.
    fn resolve_target(&self, target: &str, file_filter: Option<&str>) -> Vec<Candidate>;
    /// `tracer.trace_callees(nid, max_depth=..., show_all=...)`.
    fn trace_callees(&self, nid: &str, max_depth: usize, show_all: bool) -> Value;
    /// `tracer.trace_callers(name, target_file=..., max_depth=...)`.
    fn trace_callers(&self, raw_target_name: &str, target_file: Option<&str>, max_depth: usize) -> Value;
    /// `tracer.format_ascii_tree(tree, direction=...)`.
    fn format_ascii_tree(&self, tree: &Value, direction: Direction) -> String;
    /// Is `target` referenced anywhere in the graph?
    ///
    /// Reverse tracing (`-d callers`) legitimately accepts names that have no
    /// definition of their own — builtins and imported helpers like `print` are
    /// only ever a *call target*. That allowance must not extend to names that
    /// appear nowhere at all, or a typo silently returns "no callers".
    fn target_is_known(&self, target: &str) -> bool {
        let _ = target;
        true
    }
}

/// Failure cases; each carries the stderr lines and process exit code of the Python version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceError {
    NoProject,
    NoGraph { project: String },
    SymbolNotFound { target: String, project: String },
}

impl TraceError {
    /// Lines printed to stderr before `exit(1)` (Python printed them verbatim).
    pub fn messages(&self) -> Vec<String> {
        match self {
            TraceError::NoProject => vec![
                "[imc trace] Error: No indexed project found for current directory.".into(),
                "Run `imc index` first, or specify `--project <name_or_id>`.".into(),
            ],
            TraceError::NoGraph { project } => vec![
                format!("[imc trace] Error: AST Graph not found for project '{project}'."),
                "Run `imc index` to build the AST property graph first.".into(),
            ],
            TraceError::SymbolNotFound { target, project } => vec![
                format!("[imc trace] Symbol '{target}' not found in project '{project}'."),
                "Tip: Check spelling, or run `imc find --symbols <file>` to inspect available symbols.".into(),
            ],
        }
    }

    /// Python always exited 1 on these errors.
    pub fn exit_code(&self) -> i32 {
        1
    }
}

/// What the CLI should print on success.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceOutput {
    /// Text for stdout (sections joined by a blank line; empty in `--json` mode).
    pub text: String,
    /// Pretty-printed JSON (only in `--json` mode).
    pub json: Option<String>,
}

/// Execute the trace. On success returns what to print; on failure returns the
/// stderr lines + exit code (Python wrote to stderr and called `sys.exit(1)`).
pub fn run_trace(tracer: &dyn Tracer, opts: &TraceOptions) -> Result<TraceOutput, TraceError> {
    let Some(project) = tracer.project_name() else {
        return Err(TraceError::NoProject);
    };
    if !tracer.has_graph() {
        return Err(TraceError::NoGraph { project });
    }

    // Resolve target definition.
    let candidates = tracer.resolve_target(&opts.target, opts.file_filter.as_deref());
    let is_reverse = opts.is_reverse();

    // No definition: fatal outright when walking forward, and fatal for reverse
    // too unless the name is referenced somewhere as a call target. Before this
    // guard, `imc trace typo -d callers` printed nothing and exited 0.
    if candidates.is_empty() && (!is_reverse || !tracer.target_is_known(&opts.target)) {
        return Err(TraceError::SymbolNotFound { target: opts.target.clone(), project });
    }

    // Preserve the selected definition identity and embedded file scope.
    // Single candidate -> use its node id; ambiguous -> fall back to the raw target name.
    let raw_target_name = if candidates.len() == 1 {
        candidates[0].id.clone()
    } else {
        opts.target.clone()
    };

    if opts.json_output {
        let mut results: Vec<Value> = Vec::new();
        if opts.both || !is_reverse {
            for c in &candidates {
                let tree = tracer.trace_callees(&c.id, opts.depth, opts.show_all);
                results.push(json!({ "mode": "callees", "tree": tree }));
            }
        }
        if opts.both || is_reverse {
            let tree = tracer.trace_callers(
                &raw_target_name,
                opts.file_filter.as_deref(),
                opts.depth,
            );
            results.push(json!({ "mode": "callers", "tree": tree }));
        }
        let result_json = json!({
            "project": project,
            "target": opts.target,
            "depth": opts.depth,
            "results": results,
        });
        let rendered = serde_json::to_string_pretty(&result_json)
            .expect("trace result is always serializable");
        return Ok(TraceOutput { text: String::new(), json: Some(rendered) });
    }

    let mut sections: Vec<String> = Vec::new();
    if opts.both || !is_reverse {
        for c in &candidates {
            let tree = tracer.trace_callees(&c.id, opts.depth, opts.show_all);
            sections.push(tracer.format_ascii_tree(&tree, Direction::Callees));
        }
    }
    if opts.both || is_reverse {
        let tree = tracer.trace_callers(&raw_target_name, opts.file_filter.as_deref(), opts.depth);
        sections.push(tracer.format_ascii_tree(&tree, Direction::Callers));
    }

    // Python: print("\n\n".join(sections))
    Ok(TraceOutput { text: sections.join("\n\n"), json: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeTracer {
        project: Option<String>,
        graph: bool,
        candidates: Vec<Candidate>,
    }

    impl FakeTracer {
        fn indexed() -> Self {
            Self {
                project: Some("demo".into()),
                graph: true,
                candidates: vec![Candidate { id: "nid1".into(), data: json!({}) }],
            }
        }
        fn empty_target() -> Self {
            Self { project: Some("demo".into()), graph: true, candidates: vec![] }
        }
    }

    impl Tracer for FakeTracer {
        fn project_name(&self) -> Option<String> {
            self.project.clone()
        }
        fn has_graph(&self) -> bool {
            self.graph
        }
        fn resolve_target(&self, _target: &str, _file_filter: Option<&str>) -> Vec<Candidate> {
            self.candidates.clone()
        }
        fn trace_callees(&self, nid: &str, _max_depth: usize, _show_all: bool) -> Value {
            json!({ "node": nid, "dir": "callees" })
        }
        fn trace_callers(&self, raw_target_name: &str, _target_file: Option<&str>, _max_depth: usize) -> Value {
            json!({ "node": raw_target_name, "dir": "callers" })
        }
        fn format_ascii_tree(&self, tree: &Value, direction: Direction) -> String {
            format!("{} tree({})", tree["node"].as_str().unwrap_or("?"), direction.as_str())
        }
    }

    #[test]
    fn forward_trace_is_one_callees_section() {
        let out = run_trace(&FakeTracer::indexed(), &TraceOptions::new("foo")).unwrap();
        assert_eq!(out.text, "nid1 tree(callees)");
        assert!(out.json.is_none());
    }

    #[test]
    fn json_mode_emits_callees_results() {
        let mut opts = TraceOptions::new("foo");
        opts.json_output = true;
        let out = run_trace(&FakeTracer::indexed(), &opts).unwrap();
        let parsed: Value = serde_json::from_str(out.json.as_ref().unwrap()).unwrap();
        assert_eq!(parsed["project"], json!("demo"));
        assert_eq!(parsed["target"], json!("foo"));
        assert_eq!(parsed["depth"], json!(2));
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["mode"], json!("callees"));
        assert_eq!(results[0]["tree"]["node"], json!("nid1"));
    }

    #[test]
    fn callers_mode_uses_single_candidate_id_as_target_name() {
        let mut opts = TraceOptions::new("foo");
        opts.callers = true;
        let out = run_trace(&FakeTracer::indexed(), &opts).unwrap();
        // Single candidate -> node id "nid1" is the caller lookup target.
        assert_eq!(out.text, "nid1 tree(callers)");
    }

    #[test]
    fn callers_mode_falls_back_to_raw_target_when_ambiguous() {
        let mut tracer = FakeTracer::indexed();
        tracer.candidates.push(Candidate { id: "nid2".into(), data: json!({}) });
        // Two candidates + callers=true -> raw target name "foo" is used, and
        // (per `both or not is_reverse`) no callee sections are rendered.
        let mut opts = TraceOptions::new("foo");
        opts.callers = true;
        let out = run_trace(&tracer, &opts).unwrap();
        assert_eq!(out.text, "foo tree(callers)");

        // Same ambiguity in --both mode: callee sections come first.
        let mut both = TraceOptions::new("foo");
        both.both = true;
        let out = run_trace(&tracer, &both).unwrap();
        assert_eq!(out.text, "nid1 tree(callees)\n\nnid2 tree(callees)\n\nfoo tree(callers)");
    }

    #[test]
    fn both_mode_renders_callees_then_callers() {
        let mut opts = TraceOptions::new("foo");
        opts.both = true;
        let out = run_trace(&FakeTracer::indexed(), &opts).unwrap();
        assert_eq!(out.text, "nid1 tree(callees)\n\nnid1 tree(callers)");
    }

    #[test]
    fn reverse_allows_empty_candidate_set() {
        let mut opts = TraceOptions::new("foo");
        opts.reverse = true;
        let out = run_trace(&FakeTracer::empty_target(), &opts).unwrap();
        // No candidates, but reverse mode still traces callers by raw name.
        assert_eq!(out.text, "foo tree(callers)");
    }

    #[test]
    fn missing_symbol_is_an_error_in_forward_mode() {
        let err = run_trace(&FakeTracer::empty_target(), &TraceOptions::new("nope")).unwrap_err();
        assert_eq!(err.exit_code(), 1);
        assert_eq!(
            err.messages()[0],
            "[imc trace] Symbol 'nope' not found in project 'demo'."
        );
    }

    #[test]
    fn missing_project_and_graph_errors() {
        let no_project = FakeTracer { project: None, graph: false, candidates: vec![] };
        let err = run_trace(&no_project, &TraceOptions::new("foo")).unwrap_err();
        assert_eq!(err, TraceError::NoProject);
        assert!(err.messages()[0].contains("No indexed project"));

        let no_graph = FakeTracer { project: Some("demo".into()), graph: false, candidates: vec![] };
        let err = run_trace(&no_graph, &TraceOptions::new("foo")).unwrap_err();
        assert_eq!(err, TraceError::NoGraph { project: "demo".into() });
    }
}