//! Call-hierarchy tree expansion: forward call trees (callees), backward call
//! trees (callers / blast radius), depth limits and high-signal vs `--all`
//! filtering.
//!
//! Port of `ASTTracer.trace_callees`, `ASTTracer.trace_callers`,
//! `ASTTracer.resolve_target` and `ASTTracer.format_ascii_tree` from
//! `main/core/tracer.py`.
//!
//! Python walks a NetworkX property graph loaded from `graph_store.json`
//! (definition nodes plus `call:<name>` placeholder nodes, `CALLS` edges).
//! The Rust port walks `imc_core::store::Store` instead: `symbols` rows carry
//! `id = file::qualname`, and `edges` rows of kind `call` carry
//! `src = symbol id`, `dst = unresolved callee NAME`. The placeholder-node
//! resolution step (`_resolve_callee_node` / `_call_target`) therefore becomes
//! name resolution via `find_symbols_by_name`, with the same conservative rule:
//! a call only binds to a definition when the candidate set is unique in the
//! caller's file, or unique project-wide.
//!
//! Not ported: the Python tracer hardcodes a `ProjectManager` lookup and
//! `EndpointBridge` (HTTP route / RTK-Query cross-boundary discovery). The
//! tracer here takes a `&Store` directly, and route discovery is behind the
//! `RouteBridge` trait — supply an implementation to re-enable route nodes,
//! route-external nodes and cross-boundary callers; with `None` the traversal
//! matches Python's `bridge = None` path exactly.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use imc_core::store::Store;
use imc_core::types::SymbolKind;

use crate::builtins;

/// Node kinds emitted by the tracer. Python stores them as free-form strings on
/// graph nodes; the store has [`SymbolKind`], so kinds are carried as `&str`s
/// with these same names.
pub const KIND_EXTERNAL: &str = "external";
pub const KIND_ROUTE: &str = "route";
pub const KIND_ROUTE_EXTERNAL: &str = "route_external";
pub const KIND_CLIENT_CALL: &str = "client_call";
pub const KIND_CLIENT_HOOK: &str = "client_hook";
const DEFAULT_KIND: &str = "function";

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Traversal options — one struct covering the CLI flags `--depth`, `--all`
/// and the `max_depth` / `show_all` parameters of the Python tracer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceOptions {
    /// `max_depth`: recursion stops once a node's depth reaches this value, so
    /// children at `max_depth` are emitted as leaves. Python default `2`.
    pub max_depth: usize,
    /// `show_all` (`--all`): keep builtins and trivial POJO accessors.
    pub show_all: bool,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self { max_depth: 2, show_all: false }
    }
}

impl TraceOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    pub fn with_show_all(mut self, show_all: bool) -> Self {
        self.show_all = show_all;
        self
    }
}

// ---------------------------------------------------------------------------
// Cross-boundary bridge (HTTP routes / RTK Query hooks)
// ---------------------------------------------------------------------------

/// An HTTP route as discovered by a route scanner — the Rust equivalent of
/// `EndpointBridge`'s route records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpRoute {
    pub method: String,
    /// Raw path, used for display (`HTTP GET /api/projects`).
    pub path: String,
    /// Path used for route ids (`route:GET:/api/projects`).
    pub normalized_path: String,
    pub handler_node_id: String,
    pub handler_name: String,
    pub file: String,
    pub line: Option<usize>,
}

/// A client-side API call (RTK Query hook / fetch) found in the frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientCall {
    pub caller_name: String,
    pub caller_node_id: String,
    pub file: String,
    pub line: Option<usize>,
    pub raw_url: String,
    pub method: String,
    pub normalized_path: String,
}

/// Route / client-call discovery. Python gets this from
/// `EndpointBridge(Path(project.path), symbol_defs)`; here it is injected so the
/// crate stays independent of any particular scanner.
pub trait RouteBridge {
    /// `GET /api/projects`-style target lookup.
    fn routes_for_target(&self, method: &str, path: &str) -> Vec<HttpRoute>;

    /// `get_routes_for_caller(caller_name, caller_file)` — pairs of a client
    /// call site and the routes it reaches (empty when the route is external to
    /// the indexed project).
    fn routes_for_caller(&self, caller_name: &str, caller_file: &str)
        -> Vec<(ClientCall, Vec<HttpRoute>)>;

    /// `get_client_callers_for_handler(handler_name, handler_file)` — the
    /// reverse direction: routes served by this handler and their frontend
    /// callers.
    fn client_callers_for_handler(
        &self,
        handler_name: &str,
        handler_file: &str,
    ) -> Vec<(HttpRoute, Vec<ClientCall>)>;

    /// `calls_by_caller` — client call sites by caller symbol name, used by
    /// `resolve_target` to reach frontend-only symbols.
    fn client_calls_by_caller(&self, caller_name: &str) -> Vec<ClientCall>;
}

/// Bridge-less tracer: matches the Python code path where `self.bridge is None`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoBridge;

impl RouteBridge for NoBridge {
    fn routes_for_target(&self, _method: &str, _path: &str) -> Vec<HttpRoute> {
        Vec::new()
    }
    fn routes_for_caller(
        &self,
        _caller_name: &str,
        _caller_file: &str,
    ) -> Vec<(ClientCall, Vec<HttpRoute>)> {
        Vec::new()
    }
    fn client_callers_for_handler(
        &self,
        _handler_name: &str,
        _handler_file: &str,
    ) -> Vec<(HttpRoute, Vec<ClientCall>)> {
        Vec::new()
    }
    fn client_calls_by_caller(&self, _caller_name: &str) -> Vec<ClientCall> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Output structs (serde — the CLI `--json` serializes these verbatim)
// ---------------------------------------------------------------------------

fn is_false(v: &bool) -> bool {
    !*v
}

/// A resolved definition / synthetic node, i.e. what Python carried as a
/// `(node_id, data)` tuple.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTarget {
    pub id: String,
    pub name: String,
    pub file: String,
    pub line: Option<usize>,
    #[serde(rename = "type")]
    pub kind: String,
    /// Present on synthetic `route` targets only: the backend handler the
    /// forward trace jumps into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler_node_id: Option<String>,
}

impl ResolvedTarget {
    fn symbol(file: &str, name: &str, qualname: &str, kind: SymbolKind, line: usize) -> Self {
        Self {
            id: imc_core::types::Symbol::make_id(file, qualname),
            name: name.to_string(),
            file: file.to_string(),
            line: Some(line),
            kind: kind.as_str().to_string(),
            handler_node_id: None,
        }
    }

    fn unknown(id: &str) -> Self {
        Self {
            id: id.to_string(),
            // Python: `node_data.get("name") or node_id.split(":")[-1]`.
            name: id.rsplit("::").next().unwrap_or(id).to_string(),
            file: String::new(),
            line: None,
            kind: DEFAULT_KIND.to_string(),
            handler_node_id: None,
        }
    }
}

/// One node of a forward call tree (`trace_callees`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalleeNode {
    pub id: String,
    pub name: String,
    pub file: String,
    pub line: Option<usize>,
    #[serde(rename = "type")]
    pub kind: String,
    /// Set when the node was already seen on this branch — recursion marker.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_cycle: bool,
    /// `"unresolved"` for external nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    /// Why resolution failed. Python carried this from graph node data; here it
    /// is filled in when a name has several candidate definitions and no unique
    /// local one (`None` otherwise, as in Python).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub callees: Vec<CalleeNode>,
    pub depth: usize,
}

impl Default for CalleeNode {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            file: String::new(),
            line: None,
            kind: DEFAULT_KIND.to_string(),
            is_cycle: false,
            resolution: None,
            reason: None,
            callees: Vec::new(),
            depth: 0,
        }
    }
}

/// One node of a backward call tree (`trace_callers` / blast radius).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallerNode {
    pub id: String,
    pub name: String,
    pub file: String,
    pub line: Option<usize>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_cycle: bool,
    /// Number of definitions the target name resolved to. Absent on synthetic
    /// nodes (route / client-caller leaves), exactly like the Python dicts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definitions_count: Option<usize>,
    pub callers: Vec<CallerNode>,
    pub depth: usize,
}

impl Default for CallerNode {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            file: String::new(),
            line: None,
            kind: DEFAULT_KIND.to_string(),
            is_cycle: false,
            definitions_count: None,
            callers: Vec::new(),
            depth: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Regexes (target formats)
// ---------------------------------------------------------------------------

/// `re.fullmatch(r"(GET|POST|PUT|DELETE|PATCH|ALL)\s+(.+)", target, re.IGNORECASE)`
fn route_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)^(GET|POST|PUT|DELETE|PATCH|ALL)\s+(.+)$").unwrap())
}

/// `re.match(r"^use([A-Z0-9_][a-zA-Z0-9_]+?)(Mutation|Query)$", name)`
fn hook_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^use([A-Z0-9_][a-zA-Z0-9_]+?)(Mutation|Query)$").unwrap())
}

/// `useCreatePhaseMutation` → `createPhase`.
fn endpoint_name_from_hook(hook: &str) -> Option<String> {
    let caps = hook_re().captures(hook)?;
    let cap = caps.get(1)?.as_str();
    let mut chars = cap.chars();
    let first = chars.next()?.to_lowercase().to_string();
    Some(format!("{first}{}", chars.as_str()))
}

fn normalize_path(p: &str) -> String {
    p.replace('\\', "/").to_lowercase()
}

/// Production-over-test priority from `resolve_target._target_priority`; lower
/// sorts first.
fn target_priority(file: &str) -> i32 {
    let f = file.to_lowercase();
    let mut score = 0;
    if [
        ".spec.",
        ".test.",
        ".cy.",
        ".pom.",
        "/cypress/",
        "/tests/",
        "/test/",
        "/fixtures/",
        "/mocks/",
    ]
    .iter()
    .any(|t| f.contains(t))
    {
        score += 100;
    }
    if ["/service/", "/impl/", "/controller/", "/router/"]
        .iter()
        .any(|t| f.contains(t))
    {
        score -= 50;
    }
    if f.contains("/src/main/") {
        score -= 30;
    }
    score
}

fn dedupe(v: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    v.into_iter().filter(|x| seen.insert(x.clone())).collect()
}

// ---------------------------------------------------------------------------
// Tracer
// ---------------------------------------------------------------------------

/// Deterministic static call tracing over an indexed store.
pub struct Tracer<'a> {
    store: &'a Store,
    /// Definition-kind symbols by id — Python's graph node table.
    by_id: HashMap<String, ResolvedTarget>,
    /// Definition-kind symbols by name — Python's `symbol_defs` index.
    by_name: HashMap<String, Vec<ResolvedTarget>>,
    /// Definition-kind symbols by project-relative file.
    by_file: HashMap<String, Vec<ResolvedTarget>>,
    /// Synthetic route / client-call nodes created by `resolve_target` —
    /// Python's `ASTTracer._synthetic_nodes`.
    synthetic: RefCell<HashMap<String, ResolvedTarget>>,
    bridge: Option<&'a dyn RouteBridge>,
}

impl<'a> Tracer<'a> {
    /// Build the symbol indexes from the store's `files` / `symbols` tables.
    pub fn new(store: &'a Store) -> Self {
        Self {
            store,
            by_id: HashMap::new(),
            by_name: HashMap::new(),
            by_file: HashMap::new(),
            synthetic: RefCell::new(HashMap::new()),
            bridge: None,
        }
        .with_indexes()
    }

    fn with_indexes(mut self) -> Self {
        for (path, _) in self.store.all_file_hashes() {
            for s in self.store.symbols_in_file(&path) {
                let target = ResolvedTarget::symbol(&s.file, &s.name, &s.qualname, s.kind, s.line);
                self.by_file.entry(s.file.clone()).or_default().push(target.clone());
                // Python indexes only definition types in `symbol_defs`; any
                // symbol row is still addressable by id.
                if s.kind.is_definition() {
                    self.by_name.entry(s.name.clone()).or_default().push(target.clone());
                }
                self.by_id.insert(s.id.clone(), target);
            }
        }
        self
    }

    /// Attach a cross-boundary route/client-call bridge.
    pub fn with_bridge(mut self, bridge: &'a dyn RouteBridge) -> Self {
        self.bridge = Some(bridge);
        self
    }

    fn bridge(&self) -> Option<&'a dyn RouteBridge> {
        self.bridge
    }

    /// Node data for `id`, synthetic (route / client-call) nodes first, then
    /// the symbol table, then a name-only placeholder.
    fn node_info(&self, id: &str) -> ResolvedTarget {
        if let Some(syn) = self.synthetic.borrow().get(id) {
            return syn.clone();
        }
        self.by_id.get(id).cloned().unwrap_or_else(|| ResolvedTarget::unknown(id))
    }

    /// Remember a synthetic node so later traces can start from it.
    fn remember(&self, target: &ResolvedTarget) -> ResolvedTarget {
        self.synthetic
            .borrow_mut()
            .entry(target.id.clone())
            .or_insert_with(|| target.clone())
            .clone()
    }

    fn is_known(&self, id: &str) -> bool {
        self.by_id.contains_key(id) || self.synthetic.borrow().contains_key(id)
    }

    /// Conservative name → definition resolution (`_resolve_callee_node`):
    /// prefer a single definition in the caller's file; otherwise accept only a
    /// globally unique definition. Anything else stays unresolved.
    pub fn resolve_callee(&self, name: &str, caller_file: &str) -> Option<&ResolvedTarget> {
        let defs = self.by_name.get(name)?;
        let local: Vec<&ResolvedTarget> =
            defs.iter().filter(|d| d.file == caller_file).collect();
        if local.len() == 1 {
            return Some(local[0]);
        }
        if !local.is_empty() {
            return None;
        }
        if defs.len() == 1 {
            Some(&defs[0])
        } else {
            None
        }
    }

    // -- resolve_target ----------------------------------------------------

    /// Find candidate definition nodes for a target string. Supported formats:
    /// `run_find`, `PhaseController.createPhase`, `main/cli/commands/find.py:run_find`,
    /// `find.py:run_find`, `run_find()`, a symbol id, a file path, `GET /api/x`.
    pub fn resolve_target(&self, target: &str, file_filter: Option<&str>) -> Vec<ResolvedTarget> {
        let mut target = target.trim().to_string();
        if let Some(stripped) = target.strip_suffix("()") {
            target = stripped.trim().to_string();
        }

        // Route targets begin at their backend handler.
        if let Some(caps) = route_re().captures(&target) {
            if let Some(bridge) = self.bridge() {
                let method = caps[1].to_uppercase();
                let path = caps[2].to_string();
                let targets: Vec<ResolvedTarget> = bridge
                    .routes_for_target(&method, &path)
                    .into_iter()
                    .map(|route| ResolvedTarget {
                        id: format!(
                            "route:{}:{}:{}",
                            route.method, route.normalized_path, route.handler_node_id
                        ),
                        name: format!("HTTP {} {}", route.method, route.path),
                        file: route.file,
                        line: route.line,
                        kind: KIND_ROUTE.to_string(),
                        handler_node_id: Some(route.handler_node_id),
                    })
                    .collect();
                // Python caches synthetic route nodes in `_synthetic_nodes`.
                let known: Vec<ResolvedTarget> = targets.iter().map(|t| self.remember(t)).collect();
                return known;
            }
        }

        // Definition ids are opaque identities, not `file:name` queries.
        if let Some(known) = self.by_id.get(&target) {
            if file_filter.map(|f| matches_file(&known.file, f)).unwrap_or(true) {
                return vec![known.clone()];
            }
            return Vec::new();
        }

        let mut target_name = target.clone();
        let mut target_file = file_filter.map(|s| s.to_string());
        if let Some((file, name)) = target.split_once(':') {
            target_file = Some(file.trim().to_string());
            target_name = name.trim().to_string();
        }

        let mut candidates: Vec<ResolvedTarget> = self.resolve_name(&target_name);

        // File fallback: every definition in that file, file stem first.
        if candidates.is_empty() {
            if let Some(cands) = self.by_file.get(target.as_str()) {
                if !cands.is_empty() {
                    let stem = std::path::Path::new(target.as_str())
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    let stem_match: Vec<ResolvedTarget> =
                        cands.iter().filter(|c| c.name == stem).cloned().collect();
                    return if stem_match.is_empty() {
                        cands.clone()
                    } else {
                        stem_match
                    };
                }
            }
        }

        // Client API calls / RTK Query hooks, as a last resort.
        if candidates.is_empty() {
            if let Some(bridge) = self.bridge() {
                let mut client_calls = bridge.client_calls_by_caller(&target_name);
                if client_calls.is_empty() {
                    if let Some(ep) = hook_endpoint_name(&target_name) {
                        client_calls = bridge.client_calls_by_caller(&ep);
                    }
                }
                let synthetic: Vec<ResolvedTarget> = client_calls
                    .into_iter()
                    .map(|call| {
                        let kind = if target_name.starts_with("use") {
                            KIND_CLIENT_HOOK
                        } else {
                            KIND_CLIENT_CALL
                        };
                        ResolvedTarget {
                            id: format!("client_call:{}:{}", call.file, target_name),
                            name: target_name.clone(),
                            file: call.file,
                            line: call.line,
                            kind: kind.to_string(),
                            handler_node_id: None,
                        }
                    })
                    .collect();
                candidates.extend(synthetic.iter().map(|c| self.remember(c)));
            }
        }

        if let Some(tf) = target_file.as_deref() {
            if !candidates.is_empty() {
                candidates.retain(|c| matches_file(&c.file, tf));
            }
        }
        if !candidates.is_empty() {
            // Stable sort by production priority, like Python's
            // `candidates.sort(key=_target_priority)`.
            candidates.sort_by_key(|c| target_priority(&c.file));
        }
        candidates
    }

    /// Name lookup with the `ClassName.methodName` fallback.
    fn resolve_name(&self, target_name: &str) -> Vec<ResolvedTarget> {
        if let Some(defs) = self.by_name.get(target_name) {
            return defs.clone();
        }
        // `PhaseController.createPhase` — a matching filename is not evidence
        // that a method belongs to the requested class, so match qualnames.
        // Python compares dotted qualified names; the store uses `::`.
        if let Some((_, method)) = target_name.rsplit_once('.') {
            if let Some(method_candidates) = self.by_name.get(method) {
                let dotted = format!(".{target_name}");
                return method_candidates
                    .iter()
                    .filter(|c| {
                        let qual = symbol_qualname(c).replace("::", ".");
                        qual == target_name || qual.ends_with(&dotted)
                    })
                    .cloned()
                    .collect();
            }
        }
        Vec::new()
    }

    // -- forward -----------------------------------------------------------

    /// Forward trace: what this symbol calls.
    pub fn trace_callees(&self, node_id: &str, opts: &TraceOptions) -> CalleeNode {
        self.callees(node_id, opts, 0, &[])
    }

    fn callees(
        &self,
        node_id: &str,
        opts: &TraceOptions,
        depth: usize,
        visited: &[String],
    ) -> CalleeNode {
        let info = self.node_info(node_id);
        let mut tree = CalleeNode {
            id: info.id.clone(),
            name: info.name.clone(),
            file: info.file.clone(),
            line: info.line,
            kind: info.kind.clone(),
            depth,
            ..Default::default()
        };
        if depth >= opts.max_depth {
            return tree;
        }
        let mut path = visited.to_vec();
        path.push(node_id.to_string());

        // Route targets begin directly at their backend handler.
        if info.kind == KIND_ROUTE {
            if let Some(handler) = info.handler_node_id.clone() {
                if self.is_known(&handler) {
                    tree.callees.push(self.callees(&handler, opts, depth + 1, &path));
                }
            }
            return tree;
        }

        // Outgoing call edges. The Python graph deduplicated parallel edges, so
        // repeat calls to the same name are one child.
        for dst in dedupe(self.store.callees(node_id)) {
            if builtins::should_filter(&dst, opts.show_all) {
                continue;
            }
            match self.resolve_callee(&dst, &info.file) {
                Some(target) if path.iter().any(|v| v == &target.id) => {
                    // Recursive / cycle call detected.
                    tree.callees.push(CalleeNode {
                        id: target.id.clone(),
                        name: target.name.clone(),
                        file: target.file.clone(),
                        line: target.line,
                        kind: target.kind.clone(),
                        is_cycle: true,
                        callees: Vec::new(),
                        depth: depth + 1,
                        ..Default::default()
                    });
                }
                Some(target) => {
                    tree.callees.push(self.callees(&target.id, opts, depth + 1, &path));
                }
                None => {
                    // External or unresolvable call.
                    tree.callees.push(CalleeNode {
                        id: dst.clone(),
                        name: dst.clone(),
                        file: String::new(),
                        line: None,
                        kind: KIND_EXTERNAL.to_string(),
                        resolution: Some("unresolved".to_string()),
                        reason: self
                            .by_name
                            .get(&dst)
                            .filter(|defs| defs.len() > 1)
                            .map(|defs| format!("{} candidate definitions", defs.len())),
                        callees: Vec::new(),
                        depth: depth + 1,
                        ..Default::default()
                    });
                }
            }
        }

        // Cross-boundary frontend -> backend HTTP route calls.
        if let Some(bridge) = self.bridge() {
            let mut routes = bridge.routes_for_caller(&info.name, &info.file);
            if routes.is_empty() && info.name.starts_with("use") {
                if let Some(ep) = hook_endpoint_name(&info.name) {
                    routes = bridge.routes_for_caller(&ep, &info.file);
                }
            }
            for (client_call, matched) in routes {
                if self.is_known(node_id)
                    && info.name == client_call.caller_name
                    && client_call.caller_node_id != node_id
                {
                    continue;
                }
                if matched.is_empty() {
                    tree.callees.push(CalleeNode {
                        id: format!("route:EXTERNAL:{}", client_call.normalized_path),
                        name: format!("HTTP {}", client_call.raw_url),
                        file: client_call.file,
                        line: client_call.line,
                        kind: KIND_ROUTE_EXTERNAL.to_string(),
                        callees: Vec::new(),
                        depth: depth + 1,
                        ..Default::default()
                    });
                    continue;
                }
                for route in matched {
                    let route_id = format!("route:{}:{}", route.method, route.normalized_path);
                    if path.iter().any(|v| v == &route_id) {
                        continue;
                    }
                    let mut route_path = path.clone();
                    route_path.push(route_id.clone());
                    let handler_file = route.file.clone();
                    let handler_line = route.line;
                    let handler_name = route.handler_name.clone();
                    let handler_id = route.handler_node_id.clone();
                    let mut route_node = CalleeNode {
                        id: route_id,
                        name: format!("HTTP {} {}", route.method, route.path),
                        file: handler_file.clone(),
                        line: handler_line,
                        kind: KIND_ROUTE.to_string(),
                        callees: Vec::new(),
                        depth: depth + 1,
                        ..Default::default()
                    };
                    if self.is_known(&handler_id) {
                        route_node
                            .callees
                            .push(self.callees(&handler_id, opts, depth + 2, &route_path));
                    } else {
                        route_node.callees.push(CalleeNode {
                            id: handler_id,
                            name: handler_name,
                            file: handler_file,
                            line: handler_line,
                            kind: DEFAULT_KIND.to_string(),
                            callees: Vec::new(),
                            depth: depth + 2,
                            ..Default::default()
                        });
                    }
                    tree.callees.push(route_node);
                }
            }
        }

        tree
    }

    // -- backward ----------------------------------------------------------

    /// Reverse trace / blast radius: who calls this symbol. The target is a
    /// name (optionally scoped by `target_file`).
    pub fn trace_callers(
        &self,
        target_name: &str,
        target_file: Option<&str>,
        opts: &TraceOptions,
    ) -> CallerNode {
        self.callers(target_name, target_file, None, opts, 0, &[])
    }

    /// Reverse trace anchored on a resolved symbol id (Python's `_node_id`,
    /// used for recursion and by the CLI when the name was unambiguous).
    pub fn trace_callers_at(&self, node_id: &str, opts: &TraceOptions) -> CallerNode {
        self.callers(node_id, None, Some(node_id), opts, 0, &[])
    }

    fn callers(
        &self,
        target_name: &str,
        target_file: Option<&str>,
        node_id: Option<&str>,
        opts: &TraceOptions,
        depth: usize,
        visited: &[String],
    ) -> CallerNode {
        let defs: Vec<ResolvedTarget> = match node_id {
            Some(id) if self.is_known(id) => vec![self.node_info(id)],
            _ => self.resolve_target(target_name, target_file),
        };
        // A name matching multiple definitions is not a selected identity.
        let primary = if defs.len() == 1 { defs[0].clone() } else { ResolvedTarget::unknown("") };

        let mut tree = CallerNode {
            id: if defs.len() == 1 {
                primary.id.clone()
            } else {
                format!("call:{target_name}")
            },
            name: if defs.len() == 1 {
                primary.name.clone()
            } else {
                target_name.to_string()
            },
            file: if defs.len() == 1 {
                primary.file.clone()
            } else {
                target_file.unwrap_or_default().to_string()
            },
            line: if defs.len() == 1 { primary.line } else { None },
            kind: if defs.len() == 1 {
                primary.kind.clone()
            } else {
                DEFAULT_KIND.to_string()
            },
            definitions_count: Some(defs.len()),
            callers: Vec::new(),
            depth,
            ..Default::default()
        };

        if depth >= opts.max_depth || defs.len() != 1 {
            return tree;
        }
        let raw_name = primary.name.clone();
        let def_file = primary.file.clone();
        let def_id = primary.id.clone();
        let mut path = visited.to_vec();
        path.push(tree.id.clone());

        // Reverse edges, sorted by caller id.
        for caller_id in {
            let mut v = dedupe(self.store.callers(&raw_name));
            v.sort();
            v
        } {
            let caller = self.node_info(&caller_id);
            // A legacy/name-only edge only contributes when its conservative
            // forward resolution identifies this exact definition.
            if self
                .resolve_callee(&raw_name, &caller.file)
                .map(|d| d.id.as_str())
                != Some(def_id.as_str())
            {
                continue;
            }
            if path.iter().any(|v| v == &caller_id) {
                tree.callers.push(CallerNode {
                    id: caller_id.clone(),
                    name: caller.name.clone(),
                    file: caller.file.clone(),
                    line: caller.line,
                    kind: caller.kind.clone(),
                    is_cycle: true,
                    callers: Vec::new(),
                    depth: depth + 1,
                    ..Default::default()
                });
            } else {
                let mut next = path.clone();
                next.push(caller_id.clone());
                tree.callers.push(self.callers(
                    &caller_id,
                    Some(&caller.file),
                    Some(&caller_id),
                    opts,
                    depth + 1,
                    &next,
                ));
            }
        }

        // Cross-boundary backend -> frontend callers.
        if let Some(bridge) = self.bridge() {
            for (route, client_calls) in bridge.client_callers_for_handler(&raw_name, &def_file) {
                if route.handler_node_id != def_id {
                    continue;
                }
                let route_id = format!("route:{}:{}", route.method, route.normalized_path);
                if path.iter().any(|v| v == &route_id) {
                    continue;
                }
                let mut route_path = path.clone();
                route_path.push(route_id.clone());
                let mut route_node = CallerNode {
                    id: route_id,
                    name: format!("HTTP {} {}", route.method, route.path),
                    file: route.file.clone(),
                    line: route.line,
                    kind: KIND_ROUTE.to_string(),
                    callers: Vec::new(),
                    depth: depth + 1,
                    ..Default::default()
                };
                for client_call in client_calls {
                    if route_path.iter().any(|v| v == &client_call.caller_node_id) {
                        route_node.callers.push(CallerNode {
                            id: client_call.caller_node_id.clone(),
                            name: client_call.caller_name.clone(),
                            file: client_call.file.clone(),
                            line: client_call.line,
                            kind: DEFAULT_KIND.to_string(),
                            is_cycle: true,
                            callers: Vec::new(),
                            depth: depth + 2,
                            ..Default::default()
                        });
                    } else if self.is_known(&client_call.caller_node_id) {
                        let mut next = route_path.clone();
                        next.push(client_call.caller_node_id.clone());
                        route_node.callers.push(self.callers(
                            &client_call.caller_node_id,
                            Some(&client_call.file),
                            Some(&client_call.caller_node_id),
                            opts,
                            depth + 2,
                            &next,
                        ));
                    } else {
                        route_node.callers.push(CallerNode {
                            id: client_call.caller_node_id.clone(),
                            name: client_call.caller_name.clone(),
                            file: client_call.file.clone(),
                            line: client_call.line,
                            kind: DEFAULT_KIND.to_string(),
                            callers: Vec::new(),
                            depth: depth + 2,
                            ..Default::default()
                        });
                    }
                }
                tree.callers.push(route_node);
            }
        }

        tree
    }
}

/// `useCreatePhaseMutation` → `createPhase` (None when the name is not a hook).
fn hook_endpoint_name(name: &str) -> Option<String> {
    if !name.starts_with("use") {
        return None;
    }
    endpoint_name_from_hook(name)
}

/// Symbol ids are `file::qualname`; recover the qualname for `Class.method`
/// matching.
fn symbol_qualname(target: &ResolvedTarget) -> &str {
    match target.id.split_once("::") {
        Some((_, qualname)) => qualname,
        None => &target.name,
    }
}

/// Substring match on the normalized path (Python
/// `file_filter.replace("\\", "/").lower() in data["file"].replace(...)`).
fn matches_file(file: &str, filter: &str) -> bool {
    normalize_path(file).contains(&normalize_path(filter))
}

// ---------------------------------------------------------------------------
// ASCII rendering (`format_ascii_tree`)
// ---------------------------------------------------------------------------

fn node_line(name: &str, file: &str, line: Option<usize>, is_cycle: bool) -> String {
    let location = match line {
        Some(l) if !file.is_empty() => format!(" {file}:{l}"),
        _ => String::new(),
    };
    let suffix = if is_cycle { " (cycle)" } else { "" };
    format!("{name}{location}{suffix}")
}

/// `format_ascii_tree(tree, direction="callees")`. External nodes and nodes
/// without a file are omitted from the rendering, exactly as in Python.
pub fn format_callee_tree(tree: &CalleeNode) -> String {
    let mut lines = vec![node_line(&tree.name, &tree.file, tree.line, tree.is_cycle)];
    walk_callees(tree, "", &mut lines);
    lines.join("\n")
}

fn walk_callees(node: &CalleeNode, prefix: &str, lines: &mut Vec<String>) {
    let children: Vec<&CalleeNode> = node
        .callees
        .iter()
        .filter(|c| c.kind != KIND_EXTERNAL && !c.file.is_empty())
        .collect();
    for (index, child) in children.iter().enumerate() {
        let is_last = index == children.len() - 1;
        let branch = if is_last { "└─ " } else { "├─ " };
        lines.push(format!(
            "{prefix}{branch}📞 {}",
            node_line(&child.name, &child.file, child.line, child.is_cycle)
        ));
        let next = format!("{prefix}{}", if is_last { "   " } else { "│  " });
        walk_callees(child, &next, lines);
    }
}

/// `format_ascii_tree(tree, direction="callers")`.
pub fn format_caller_tree(tree: &CallerNode) -> String {
    let mut lines = vec![node_line(&tree.name, &tree.file, tree.line, tree.is_cycle)];
    walk_callers(tree, "", &mut lines);
    lines.join("\n")
}

fn walk_callers(node: &CallerNode, prefix: &str, lines: &mut Vec<String>) {
    let children: Vec<&CallerNode> = node
        .callers
        .iter()
        .filter(|c| c.kind != KIND_EXTERNAL && !c.file.is_empty())
        .collect();
    for (index, child) in children.iter().enumerate() {
        let is_last = index == children.len() - 1;
        let branch = if is_last { "└─ " } else { "├─ " };
        lines.push(format!(
            "{prefix}{branch}▲── {}",
            node_line(&child.name, &child.file, child.line, child.is_cycle)
        ));
        let next = format!("{prefix}{}", if is_last { "   " } else { "│  " });
        walk_callers(child, &next, lines);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use imc_core::types::{Edge, EdgeKind, Symbol};

    use super::*;

    static SEQ: AtomicU32 = AtomicU32::new(0);

    struct TempDb {
        store: Option<Store>,
        path: PathBuf,
    }

    impl TempDb {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "imc-trace-test-{}-{}-{}",
                std::process::id(),
                tag,
                n
            ));
            let _ = std::fs::remove_dir_all(&path);
            let store = imc_core::store::open(&path).expect("open store");
            Self { store: Some(store), path }
        }

        fn store(&self) -> &Store {
            self.store.as_ref().unwrap()
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            self.store.take();
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn sym(file: &str, qualname: &str, kind: SymbolKind, line: usize) -> Symbol {
        let name = qualname.rsplit("::").next().unwrap_or(qualname).to_string();
        Symbol {
            id: Symbol::make_id(file, qualname),
            name,
            qualname: qualname.to_string(),
            kind,
            file: file.to_string(),
            line,
            end_line: line + 4,
            start_byte: line * 100,
            end_byte: line * 100 + 50,
            parent: None,
        }
    }

    fn call(src: &str, dst: &str) -> Edge {
        Edge {
            src: src.to_string(),
            dst: dst.to_string(),
            kind: EdgeKind::Call,
            file: String::new(),
            line: 1,
        }
    }

    fn id(file: &str, qualname: &str) -> String {
        Symbol::make_id(file, qualname)
    }

    /// a -> b -> c, plus a -> print (builtin) and a -> getName (accessor).
    fn seed_chain(db: &mut TempDb) {
        let file = "app/core.py";
        db.store
            .as_mut()
            .unwrap()
            .upsert_file(file, "h1", Some("python"), 100)
            .unwrap();
        let store = db.store.as_mut().unwrap();
        store
            .insert_symbols(
                file,
                &[
                    sym(file, "a", SymbolKind::Function, 1),
                    sym(file, "b", SymbolKind::Function, 10),
                    sym(file, "c", SymbolKind::Function, 20),
                ],
            )
            .unwrap();
        store
            .insert_edges(
                file,
                &[
                    call(&id(file, "a"), "b"),
                    call(&id(file, "b"), "c"),
                    call(&id(file, "c"), "a"), // cycle back to the root
                    call(&id(file, "a"), "print"),
                    call(&id(file, "a"), "getName"),
                ],
            )
            .unwrap();
    }

    fn names(tree: &CalleeNode) -> Vec<&str> {
        tree.callees.iter().map(|c| c.name.as_str()).collect()
    }

    fn caller_names(tree: &CallerNode) -> Vec<&str> {
        tree.callers.iter().map(|c| c.name.as_str()).collect()
    }

    // -- forward -----------------------------------------------------------

    #[test]
    fn forward_tree_expands_callees_with_depths() {
        let mut db = TempDb::new("fwd");
        seed_chain(&mut db);
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callees(&id("app/core.py", "a"), &TraceOptions::default().with_depth(3));

        assert_eq!(tree.name, "a");
        assert_eq!(tree.file, "app/core.py");
        assert_eq!(tree.line, Some(1));
        assert_eq!(tree.depth, 0);
        // Builtins + accessors filtered by default.
        assert_eq!(names(&tree), vec!["b"]);

        let b = &tree.callees[0];
        assert_eq!(b.name, "b");
        assert_eq!(b.depth, 1);
        assert_eq!(names(b), vec!["c"]);

        let c = &b.callees[0];
        assert_eq!(c.depth, 2);
        assert_eq!(c.name, "c");
        // c calls a again, but a is on the branch → cycle marker.
        assert_eq!(names(c), vec!["a"]);
        assert!(c.callees[0].is_cycle);
        assert_eq!(c.callees[0].depth, 3);
        assert!(c.callees[0].callees.is_empty());
    }

    #[test]
    fn forward_tree_respects_max_depth() {
        let mut db = TempDb::new("depth");
        seed_chain(&mut db);
        let tracer = Tracer::new(db.store());

        let depth1 = tracer.trace_callees(&id("app/core.py", "a"), &TraceOptions::default().with_depth(1));
        assert_eq!(names(&depth1), vec!["b"]);
        assert!(depth1.callees[0].callees.is_empty(), "children at max_depth are leaves");

        let depth0 = tracer.trace_callees(&id("app/core.py", "a"), &TraceOptions::default().with_depth(0));
        assert!(depth0.callees.is_empty());
        assert_eq!(depth0.name, "a");

        let depth2 = tracer.trace_callees(&id("app/core.py", "a"), &TraceOptions::default().with_depth(2));
        assert_eq!(depth2.callees[0].callees[0].depth, 2);
        assert!(
            depth2.callees[0].callees[0].callees.is_empty(),
            "depth 2 == max_depth 2, so c's own callees stop"
        );
    }

    #[test]
    fn forward_tree_default_filters_builtins_show_all_keeps_them() {
        let mut db = TempDb::new("builtins");
        seed_chain(&mut db);
        let tracer = Tracer::new(db.store());
        let root = id("app/core.py", "a");

        let high_signal = tracer.trace_callees(&root, &TraceOptions::default());
        assert_eq!(names(&high_signal), vec!["b"]);

        let all = tracer.trace_callees(
            &root,
            &TraceOptions::default().with_show_all(true),
        );
        let mut got = names(&all);
        got.sort();
        assert_eq!(got, vec!["b", "getName", "print"]);

        // Unresolved names surface as external leaves.
        let print = all.callees.iter().find(|c| c.name == "print").unwrap();
        assert_eq!(print.kind, KIND_EXTERNAL);
        assert_eq!(print.resolution.as_deref(), Some("unresolved"));
        assert_eq!(print.file, "");
        assert_eq!(print.line, None);
        assert_eq!(print.depth, 1);
    }

    #[test]
    fn forward_tree_prefers_definition_in_caller_file() {
        let mut db = TempDb::new("local");
        let f1 = "pkg/one.py";
        let f2 = "pkg/two.py";
        let store = db.store.as_mut().unwrap();
        store.upsert_file(f1, "h1", Some("python"), 10).unwrap();
        store.upsert_file(f2, "h2", Some("python"), 10).unwrap();
        store
            .insert_symbols(
                f1,
                &[sym(f1, "helper", SymbolKind::Function, 1), sym(f1, "entry", SymbolKind::Function, 9)],
            )
            .unwrap();
        store
            .insert_symbols(f2, &[sym(f2, "helper", SymbolKind::Function, 1)])
            .unwrap();
        store
            .insert_edges(f1, &[call(&id(f1, "entry"), "helper")])
            .unwrap();
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callees(&id(f1, "entry"), &TraceOptions::default());
        assert_eq!(names(&tree), vec!["helper"]);
        assert_eq!(tree.callees[0].file, f1, "same-file definition wins");
    }

    #[test]
    fn ambiguous_name_stays_external() {
        let mut db = TempDb::new("ambig");
        let f1 = "pkg/one.py";
        let f2 = "pkg/two.py";
        let f3 = "pkg/three.py";
        let store = db.store.as_mut().unwrap();
        store.upsert_file(f1, "h", Some("python"), 10).unwrap();
        store.upsert_file(f2, "h", Some("python"), 10).unwrap();
        store.upsert_file(f3, "h", Some("python"), 10).unwrap();
        store
            .insert_symbols(f1, &[sym(f1, "helper", SymbolKind::Function, 1)])
            .unwrap();
        store
            .insert_symbols(f2, &[sym(f2, "helper", SymbolKind::Function, 1)])
            .unwrap();
        store
            .insert_symbols(f3, &[sym(f3, "entry", SymbolKind::Function, 5)])
            .unwrap();
        store
            .insert_edges(f3, &[call(&id(f3, "entry"), "helper")])
            .unwrap();
        let tracer = Tracer::new(db.store());
        // Same-file definitions bind; two global candidates do not.
        assert!(tracer.resolve_callee("helper", f1).is_some());
        assert!(tracer.resolve_callee("helper", f2).is_some());
        assert!(tracer.resolve_callee("helper", f3).is_none());
        assert!(tracer.resolve_callee("missing", f1).is_none());

        let tree = tracer.trace_callees(&id(f3, "entry"), &TraceOptions::default());
        assert_eq!(tree.callees[0].kind, KIND_EXTERNAL);
        assert_eq!(tree.callees[0].name, "helper");
    }

    #[test]
    fn unique_global_name_resolves_without_local_match() {
        let mut db = TempDb::new("global");
        let f1 = "pkg/one.py";
        let f2 = "pkg/two.py";
        let store = db.store.as_mut().unwrap();
        store.upsert_file(f1, "h", Some("python"), 10).unwrap();
        store.upsert_file(f2, "h", Some("python"), 10).unwrap();
        store
            .insert_symbols(f1, &[sym(f1, "helper", SymbolKind::Function, 1)])
            .unwrap();
        store
            .insert_symbols(f2, &[sym(f2, "entry", SymbolKind::Function, 5)])
            .unwrap();
        store
            .insert_edges(f2, &[call(&id(f2, "entry"), "helper")])
            .unwrap();
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callees(&id(f2, "entry"), &TraceOptions::default());
        assert_eq!(tree.callees[0].name, "helper");
        assert_eq!(tree.callees[0].file, f1);
    }

    #[test]
    fn unresolved_call_reports_candidate_count() {
        let mut db = TempDb::new("reason");
        let f1 = "pkg/one.py";
        let f2 = "pkg/two.py";
        let f3 = "pkg/three.py";
        // The caller lives in a fourth file with no local `helper`.
        let f4 = "pkg/entry.py";
        let store = db.store.as_mut().unwrap();
        for f in [f1, f2, f3, f4] {
            store.upsert_file(f, "h", Some("python"), 10).unwrap();
        }
        store
            .insert_symbols(f1, &[sym(f1, "helper", SymbolKind::Function, 1)])
            .unwrap();
        store
            .insert_symbols(f2, &[sym(f2, "helper", SymbolKind::Function, 1)])
            .unwrap();
        store
            .insert_symbols(f3, &[sym(f3, "helper", SymbolKind::Function, 1)])
            .unwrap();
        store
            .insert_symbols(f4, &[sym(f4, "entry", SymbolKind::Function, 5)])
            .unwrap();
        store
            .insert_edges(f4, &[call(&id(f4, "entry"), "helper")])
            .unwrap();
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callees(&id(f4, "entry"), &TraceOptions::default());
        assert_eq!(tree.callees[0].kind, KIND_EXTERNAL);
        assert_eq!(tree.callees[0].reason.as_deref(), Some("3 candidate definitions"));
    }

    #[test]
    fn forward_tree_on_unknown_id_uses_id_tail_as_name() {
        let db = TempDb::new("unknown");
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callees("ghost::handler", &TraceOptions::default());
        assert_eq!(tree.name, "handler");
        assert_eq!(tree.file, "");
        assert_eq!(tree.line, None);
        assert!(tree.callees.is_empty());
    }

    // -- backward ----------------------------------------------------------

    #[test]
    fn backward_tree_lists_callers_in_id_order() {
        let mut db = TempDb::new("blast");
        let f = "svc/core.py";
        let store = db.store.as_mut().unwrap();
        store.upsert_file(f, "h", Some("python"), 10).unwrap();
        store
            .insert_symbols(
                f,
                &[
                    sym(f, "target", SymbolKind::Function, 1),
                    sym(f, "zz_caller", SymbolKind::Function, 10),
                    sym(f, "aa_caller", SymbolKind::Function, 20),
                    sym(f, "aa_caller::load_target", SymbolKind::Method, 30),
                ],
            )
            .unwrap();
        store
            .insert_edges(
                f,
                &[
                    call(&id(f, "zz_caller"), "target"),
                    call(&id(f, "aa_caller"), "target"),
                    call(&id(f, "aa_caller::load_target"), "target"),
                ],
            )
            .unwrap();
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callers("target", Some(f), &TraceOptions::default().with_depth(2));

        assert_eq!(tree.name, "target");
        assert_eq!(tree.definitions_count, Some(1));
        assert_eq!(tree.depth, 0);
        assert_eq!(
            caller_names(&tree),
            vec!["aa_caller", "load_target", "zz_caller"],
            "sorted by caller symbol id; display names are symbol names"
        );
        for c in &tree.callers {
            assert_eq!(c.depth, 1);
            assert!(c.callers.is_empty(), "callee direction is empty");
        }
    }

    #[test]
    fn backward_tree_expands_transitively() {
        let mut db = TempDb::new("blast2");
        let f = "svc/core.py";
        let store = db.store.as_mut().unwrap();
        store.upsert_file(f, "h", Some("python"), 10).unwrap();
        store
            .insert_symbols(
                f,
                &[
                    sym(f, "a", SymbolKind::Function, 1),
                    sym(f, "b", SymbolKind::Function, 10),
                    sym(f, "c", SymbolKind::Function, 20),
                ],
            )
            .unwrap();
        store
            .insert_edges(
                f,
                &[call(&id(f, "a"), "b"), call(&id(f, "b"), "c")],
            )
            .unwrap();
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callers("c", Some(f), &TraceOptions::default().with_depth(2));
        assert_eq!(caller_names(&tree), vec!["b"]);
        assert_eq!(tree.callers[0].depth, 1);
        assert_eq!(caller_names(&tree.callers[0]), vec!["a"]);
        assert_eq!(tree.callers[0].callers[0].depth, 2);
    }

    #[test]
    fn backward_tree_respects_max_depth() {
        let mut db = TempDb::new("blast3");
        let f = "svc/core.py";
        let store = db.store.as_mut().unwrap();
        store.upsert_file(f, "h", Some("python"), 10).unwrap();
        store
            .insert_symbols(
                f,
                &[
                    sym(f, "a", SymbolKind::Function, 1),
                    sym(f, "b", SymbolKind::Function, 10),
                    sym(f, "c", SymbolKind::Function, 20),
                ],
            )
            .unwrap();
        store
            .insert_edges(
                f,
                &[call(&id(f, "a"), "b"), call(&id(f, "b"), "c")],
            )
            .unwrap();
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callers("c", Some(f), &TraceOptions::default().with_depth(1));
        assert_eq!(caller_names(&tree), vec!["b"]);
        assert!(tree.callers[0].callers.is_empty());
    }

    #[test]
    fn backward_tree_marks_cycles() {
        let mut db = TempDb::new("cycle");
        let f = "svc/core.py";
        let store = db.store.as_mut().unwrap();
        store.upsert_file(f, "h", Some("python"), 10).unwrap();
        store
            .insert_symbols(
                f,
                &[sym(f, "a", SymbolKind::Function, 1), sym(f, "b", SymbolKind::Function, 10)],
            )
            .unwrap();
        store
            .insert_edges(f, &[call(&id(f, "a"), "b"), call(&id(f, "b"), "a")])
            .unwrap();
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callers("a", Some(f), &TraceOptions::default().with_depth(3));
        assert_eq!(caller_names(&tree), vec!["b"]);
        assert_eq!(caller_names(&tree.callers[0]), vec!["a"]);
        assert!(tree.callers[0].callers[0].is_cycle);
    }

    #[test]
    fn backward_tree_stops_on_ambiguous_name() {
        let mut db = TempDb::new("ambig2");
        let f1 = "pkg/one.py";
        let f2 = "pkg/two.py";
        let store = db.store.as_mut().unwrap();
        for f in [f1, f2] {
            store.upsert_file(f, "h", Some("python"), 10).unwrap();
        }
        store
            .insert_symbols(
                f1,
                &[sym(f1, "helper", SymbolKind::Function, 1), sym(f1, "entry", SymbolKind::Function, 9)],
            )
            .unwrap();
        store
            .insert_symbols(f2, &[sym(f2, "helper", SymbolKind::Function, 1)])
            .unwrap();
        store
            .insert_edges(f1, &[call(&id(f1, "entry"), "helper")])
            .unwrap();
        let tracer = Tracer::new(db.store());

        let ambiguous = tracer.trace_callers("helper", None, &TraceOptions::default().with_depth(3));
        assert_eq!(ambiguous.definitions_count, Some(2));
        assert!(ambiguous.callers.is_empty(), "ambiguous name is not an identity");
        assert_eq!(ambiguous.id, "call:helper");
        assert_eq!(ambiguous.file, "");

        let missing = tracer.trace_callers("nope", None, &TraceOptions::default());
        assert_eq!(missing.definitions_count, Some(0));
        assert!(missing.callers.is_empty());
        assert_eq!(missing.id, "call:nope");
    }

    #[test]
    fn backward_tree_resolves_callers_by_symbol_id() {
        let mut db = TempDb::new("blast4");
        seed_chain(&mut db);
        let tracer = Tracer::new(db.store());
        // b is called by a
        let tree = tracer.trace_callers_at(&id("app/core.py", "b"), &TraceOptions::default().with_depth(3));
        assert_eq!(caller_names(&tree), vec!["a"]);
        // c is called by b
        let tree2 = tracer.trace_callers("c", Some("app/core.py"), &TraceOptions::default().with_depth(3));
        assert_eq!(caller_names(&tree2), vec!["b"]);
    }

    // -- resolve_target ----------------------------------------------------

    #[test]
    fn resolve_target_handles_supported_formats() {
        let mut db = TempDb::new("resolve");
        let f = "main/cli/commands/find.py";
        let store = db.store.as_mut().unwrap();
        store.upsert_file(f, "h", Some("python"), 10).unwrap();
        store
            .insert_symbols(
                f,
                &[
                    sym(f, "run_find", SymbolKind::Function, 1),
                    sym(f, "PhaseController::createPhase", SymbolKind::Method, 10),
                    sym(f, "PhaseController", SymbolKind::Class, 20),
                    sym(f, "find", SymbolKind::Function, 30),
                ],
            )
            .unwrap();
        let tracer = Tracer::new(db.store());

        assert_eq!(tracer.resolve_target("run_find", None)[0].id, id(f, "run_find"));
        assert_eq!(tracer.resolve_target("run_find()", None)[0].name, "run_find");
        assert_eq!(
            tracer.resolve_target("find.py:run_find", None)[0].id,
            id(f, "run_find")
        );
        assert_eq!(
            tracer.resolve_target("main/cli/commands/find.py:run_find", None)[0].id,
            id(f, "run_find")
        );
        assert_eq!(
            tracer.resolve_target(&id(f, "run_find"), None)[0].id,
            id(f, "run_find")
        );
        // Class.method falls back to a qualname match.
        assert_eq!(
            tracer.resolve_target("PhaseController.createPhase", None)[0].id,
            id(f, "PhaseController::createPhase")
        );
        // A bare file path resolves to its definitions, stem first.
        let file_target = tracer.resolve_target(f, None);
        assert_eq!(file_target[0].name, "find", "file stem match wins");
        assert!(file_target.iter().all(|c| c.file == f));
    }

    #[test]
    fn resolve_target_filters_by_file_and_priority() {
        let mut db = TempDb::new("priority");
        let prod = "src/main/java/app/service/PhaseService.java";
        let test = "src/test/java/app/PhaseServiceTest.java";
        let store = db.store.as_mut().unwrap();
        for f in [prod, test] {
            store.upsert_file(f, "h", Some("java"), 10).unwrap();
        }
        store
            .insert_symbols(
                prod,
                &[sym(prod, "PhaseController::createPhase", SymbolKind::Method, 1)],
            )
            .unwrap();
        store
            .insert_symbols(
                test,
                &[sym(test, "PhaseController::createPhase", SymbolKind::Method, 1)],
            )
            .unwrap();
        let tracer = Tracer::new(db.store());

        let all = tracer.resolve_target("createPhase", None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].file, prod, "production code sorts before tests");

        let scoped = tracer.resolve_target("createPhase", Some("app/service/"));
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].file, prod);

        assert!(tracer.resolve_target("createPhase", Some("does/not/match")).is_empty());
        assert!(tracer.resolve_target("unknown_symbol", None).is_empty());
    }

    #[test]
    fn resolve_target_empty_when_id_filtered_by_file() {
        let mut db = TempDb::new("resolve2");
        let f = "app/core.py";
        let store = db.store.as_mut().unwrap();
        store.upsert_file(f, "h", Some("python"), 10).unwrap();
        store
            .insert_symbols(f, &[sym(f, "a", SymbolKind::Function, 1)])
            .unwrap();
        let tracer = Tracer::new(db.store());
        assert_eq!(tracer.resolve_target("a", Some("app/")).len(), 1);
        assert!(tracer.resolve_target("a", Some("other/")).is_empty());
    }

    // -- rendering / serialization ----------------------------------------

    #[test]
    fn ascii_rendering_skips_external_and_nameless_nodes() {
        let mut db = TempDb::new("ascii");
        seed_chain(&mut db);
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callees(
            &id("app/core.py", "a"),
            &TraceOptions::default().with_depth(2).with_show_all(true),
        );
        let text = format_callee_tree(&tree);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "a app/core.py:1");
        assert_eq!(lines[1], "└─ 📞 b app/core.py:10");
        assert_eq!(lines[2], "   └─ 📞 c app/core.py:20");
        // `print`/`getName` are external (no file) and never rendered.
        assert!(!text.contains("print"));
        assert!(!text.contains("getName"));
    }

    #[test]
    fn ascii_rendering_marks_cycles_for_callers() {
        let mut db = TempDb::new("ascii2");
        let f = "svc/core.py";
        let store = db.store.as_mut().unwrap();
        store.upsert_file(f, "h", Some("python"), 10).unwrap();
        store
            .insert_symbols(
                f,
                &[sym(f, "a", SymbolKind::Function, 1), sym(f, "b", SymbolKind::Function, 5)],
            )
            .unwrap();
        store
            .insert_edges(f, &[call(&id(f, "a"), "b"), call(&id(f, "b"), "a")])
            .unwrap();
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callers("a", Some(f), &TraceOptions::default().with_depth(3));
        let text = format_caller_tree(&tree);
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            vec![
                "a svc/core.py:1",
                "└─ ▲── b svc/core.py:5",
                "   └─ ▲── a svc/core.py:1 (cycle)",
            ]
        );
    }

    #[test]
    fn trees_serialize_with_python_json_keys() {
        let mut db = TempDb::new("json");
        seed_chain(&mut db);
        let tracer = Tracer::new(db.store());
        let fwd = tracer.trace_callees(&id("app/core.py", "a"), &TraceOptions::default().with_depth(3));
        let json = serde_json::to_value(&fwd).unwrap();
        assert_eq!(json["name"], "a");
        assert_eq!(json["type"], "function");
        assert_eq!(json["depth"], 0);
        assert_eq!(json["callees"][0]["name"], "b");
        assert!(json["callees"][0].get("is_cycle").is_none());
        assert!(fwd.callees[0].callees[0].callees[0].is_cycle);
        assert_eq!(json["callees"][0]["callees"][0]["callees"][0]["is_cycle"], true);

        let all = tracer.trace_callees(
            &id("app/core.py", "a"),
            &TraceOptions::default().with_show_all(true),
        );
        let json = serde_json::to_value(&all).unwrap();
        let external = json["callees"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "print")
            .unwrap()
            .clone();
        assert_eq!(external["type"], KIND_EXTERNAL);
        assert_eq!(external["resolution"], "unresolved");
        assert!(external["line"].is_null());

        let back = tracer.trace_callers("b", Some("app/core.py"), &TraceOptions::default());
        let json = serde_json::to_value(&back).unwrap();
        assert_eq!(json["definitions_count"], 1);
        assert_eq!(json["callers"][0]["name"], "a");
    }

    // -- bridge -----------------------------------------------------------

    struct FakeBridge {
        routes: Vec<HttpRoute>,
        caller_routes: Vec<(ClientCall, Vec<HttpRoute>)>,
        handler_callers: Vec<(HttpRoute, Vec<ClientCall>)>,
        /// `calls_by_caller`: client call sites keyed by caller symbol name.
        client_calls: HashMap<String, Vec<ClientCall>>,
    }

    impl RouteBridge for FakeBridge {
        fn routes_for_target(&self, _method: &str, _path: &str) -> Vec<HttpRoute> {
            self.routes.clone()
        }
        fn routes_for_caller(
            &self,
            _caller_name: &str,
            _caller_file: &str,
        ) -> Vec<(ClientCall, Vec<HttpRoute>)> {
            self.caller_routes.clone()
        }
        fn client_callers_for_handler(
            &self,
            _handler_name: &str,
            _handler_file: &str,
        ) -> Vec<(HttpRoute, Vec<ClientCall>)> {
            self.handler_callers.clone()
        }
        fn client_calls_by_caller(&self, caller_name: &str) -> Vec<ClientCall> {
            self.client_calls.get(caller_name).cloned().unwrap_or_default()
        }
    }

    fn route(method: &str, path: &str, handler: &str, handler_name: &str) -> HttpRoute {
        HttpRoute {
            method: method.to_string(),
            path: path.to_string(),
            normalized_path: path.to_string(),
            handler_node_id: handler.to_string(),
            handler_name: handler_name.to_string(),
            file: "api/routes.py".to_string(),
            line: Some(7),
        }
    }

    fn client_call(caller: &str, file: &str) -> ClientCall {
        ClientCall {
            caller_name: caller.to_string(),
            caller_node_id: id("web/app.ts", caller),
            file: file.to_string(),
            line: Some(3),
            raw_url: "http://localhost/api/projects".to_string(),
            method: "GET".to_string(),
            normalized_path: "/api/projects".to_string(),
        }
    }

    #[test]
    fn bridge_expands_route_into_handler_in_forward_trace() {
        let mut db = TempDb::new("bridge-fwd");
        let api = "api/routes.py";
        let web = "web/app.ts";
        let store = db.store.as_mut().unwrap();
        for f in [api, web] {
            store.upsert_file(f, "h", Some("python"), 10).unwrap();
        }
        store
            .insert_symbols(
                api,
                &[
                    sym(api, "listProjects", SymbolKind::Function, 7),
                    sym(api, "ProjectRepo::findAll", SymbolKind::Method, 20),
                ],
            )
            .unwrap();
        store
            .insert_symbols(web, &[sym(web, "loadProjects", SymbolKind::Function, 3)])
            .unwrap();
        store
            .insert_edges(api, &[call(&id(api, "listProjects"), "findAll")])
            .unwrap();

        let bridge = FakeBridge {
            routes: vec![route("GET", "/api/projects", &id(api, "listProjects"), "listProjects")],
            caller_routes: vec![(
                client_call("loadProjects", web),
                vec![route("GET", "/api/projects", &id(api, "listProjects"), "listProjects")],
            )],
            handler_callers: Vec::new(),
            client_calls: HashMap::from([(
                "loadProjects".to_string(),
                vec![client_call("loadProjects", web)],
            )]),
        };
        let tracer = Tracer::new(db.store()).with_bridge(&bridge);

        let tree = tracer.trace_callees(&id(web, "loadProjects"), &TraceOptions::default().with_depth(3));
        assert_eq!(tree.callees.len(), 1);
        let route_node = &tree.callees[0];
        assert_eq!(route_node.kind, KIND_ROUTE);
        assert_eq!(route_node.id, "route:GET:/api/projects");
        assert_eq!(route_node.name, "HTTP GET /api/projects");
        assert_eq!(route_node.depth, 1);
        assert_eq!(route_node.callees[0].name, "listProjects");
        assert_eq!(route_node.callees[0].depth, 2, "handler recurses at depth + 2");
        assert_eq!(names(&route_node.callees[0]), vec!["findAll"]);

        // A route target resolves directly and jumps into its handler.
        let resolved = tracer.resolve_target("get /api/projects", None);
        assert_eq!(resolved[0].kind, KIND_ROUTE);
        let from_route = tracer.trace_callees(&resolved[0].id, &TraceOptions::default().with_depth(2));
        assert_eq!(from_route.callees[0].name, "listProjects");
        assert_eq!(from_route.callees[0].depth, 1);
    }

    #[test]
    fn bridge_marks_external_routes_and_reverse_callers() {
        let mut db = TempDb::new("bridge-rev");
        let api = "api/routes.py";
        let web = "web/app.ts";
        let store = db.store.as_mut().unwrap();
        for f in [api, web] {
            store.upsert_file(f, "h", Some("python"), 10).unwrap();
        }
        store
            .insert_symbols(api, &[sym(api, "listProjects", SymbolKind::Function, 7)])
            .unwrap();
        store
            .insert_symbols(web, &[sym(web, "loadProjects", SymbolKind::Function, 3)])
            .unwrap();

        let bridge = FakeBridge {
            routes: Vec::new(),
            caller_routes: vec![(client_call("loadProjects", web), Vec::new())],
            handler_callers: vec![(
                route("GET", "/api/projects", &id(api, "listProjects"), "listProjects"),
                vec![client_call("loadProjects", web)],
            )],
            client_calls: HashMap::from([(
                "loadProjects".to_string(),
                vec![client_call("loadProjects", web)],
            )]),
        };
        let tracer = Tracer::new(db.store()).with_bridge(&bridge);

        let fwd = tracer.trace_callees(&id(web, "loadProjects"), &TraceOptions::default());
        assert_eq!(fwd.callees[0].kind, KIND_ROUTE_EXTERNAL);
        assert_eq!(fwd.callees[0].id, "route:EXTERNAL:/api/projects");
        assert_eq!(fwd.callees[0].name, "HTTP http://localhost/api/projects");

        let back = tracer.trace_callers("listProjects", Some(api), &TraceOptions::default().with_depth(2));
        assert_eq!(back.callers.len(), 1);
        let route_node = &back.callers[0];
        assert_eq!(route_node.kind, KIND_ROUTE);
        assert_eq!(route_node.depth, 1);
        assert_eq!(route_node.callers[0].name, "loadProjects");
        assert_eq!(route_node.callers[0].depth, 2);

        // Client-only hook resolution fallback.
        let hooks = tracer.resolve_target("useLoadProjectsQuery", None);
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].kind, KIND_CLIENT_HOOK);
        assert_eq!(hooks[0].id, "client_call:web/app.ts:useLoadProjectsQuery");
    }

    #[test]
    fn bridge_none_matches_bridge_less_behaviour() {
        let mut db = TempDb::new("nobridge");
        seed_chain(&mut db);
        let tracer = Tracer::new(db.store()).with_bridge(&NoBridge);
        let tree = tracer.trace_callees(&id("app/core.py", "a"), &TraceOptions::default().with_depth(2));
        assert_eq!(names(&tree), vec!["b"]);
        assert!(tracer.resolve_target("GET /api/projects", None).is_empty());
        assert!(tracer.resolve_target("useLoadProjectsQuery", None).is_empty());
    }

    #[test]
    fn hook_endpoint_name_maps_mutation_and_query_hooks() {
        assert_eq!(
            hook_endpoint_name("useCreatePhaseMutation").as_deref(),
            Some("createPhase")
        );
        assert_eq!(
            hook_endpoint_name("useListProjectsQuery").as_deref(),
            Some("listProjects")
        );
        assert_eq!(hook_endpoint_name("loadProjects"), None);
        assert_eq!(hook_endpoint_name("use"), None);
    }

    #[test]
    fn route_regex_matches_supported_methods_only() {
        for t in [
            "GET /api/projects",
            "post /api/projects",
            "PUT /x",
            "DELETE /x",
            "PATCH /x",
            "ALL /x",
        ] {
            assert!(route_re().is_match(t), "{t} should parse as a route");
        }
        assert!(!route_re().is_match("/api/projects"));
        assert!(!route_re().is_match("GET"));
        assert!(!route_re().is_match("GET /a\nsecond"));
    }

    #[test]
    fn path_matching_normalizes_separators_and_case() {
        assert!(matches_file("main\\cli\\find.py", "main/cli"));
        assert!(matches_file("Main/CLI/find.py", "main/cli"));
        assert!(!matches_file("other/find.py", "main/cli"));
        assert!(matches_file("main/cli/find.py", ""));
    }

    #[test]
    fn target_priority_matches_python_scoring() {
        assert_eq!(target_priority("src/test/java/Foo.java"), 100);
        assert_eq!(target_priority("cypress/e2e/a.cy.js"), 100);
        assert_eq!(target_priority("app/service/Foo.java"), -50);
        assert_eq!(target_priority("app/src/main/java/Foo.java"), -30);
        assert_eq!(target_priority("src/main/java/Foo.java"), 0);
        assert_eq!(target_priority("app/router/Foo.py"), -50);
        assert_eq!(target_priority("app/plain.py"), 0);
    }

    #[test]
    fn empty_store_yields_empty_trees() {
        let db = TempDb::new("empty");
        let tracer = Tracer::new(db.store());
        let tree = tracer.trace_callees("a", &TraceOptions::default());
        assert!(tree.callees.is_empty());
        let back = tracer.trace_callers("a", None, &TraceOptions::default());
        assert_eq!(back.definitions_count, Some(0));
        assert!(back.callers.is_empty());
        assert!(tracer.resolve_target("a", None).is_empty());
        assert_eq!(format_callee_tree(&tree), "a");
    }

    #[test]
    fn options_default_to_python_defaults() {
        let o = TraceOptions::default();
        assert_eq!(o.max_depth, 2);
        assert!(!o.show_all);
        assert_eq!(o.with_depth(5).with_show_all(true).max_depth, 5);
        assert!(o.with_depth(5).with_show_all(true).show_all);
    }
}
