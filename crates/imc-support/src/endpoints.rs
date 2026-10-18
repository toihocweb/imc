//! Universal Full-Stack Endpoint Bridge for IndexMyCode.
//!
//! Bridges client-side API requests (React, RTK Query, Axios, fetch, Vue,
//! Angular) to server-side route handlers (Spring Boot, FastAPI, Express,
//! NestJS, Go Gin, .NET) via universal URL pattern normalization and static
//! AST correlation.
//!
//! Faithful port of `main/core/endpoint_bridge.py`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;
use walkdir::WalkDir;

use crate::context::splitlines;
use imc_core::store::Store;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(|| Regex::new($pat).expect(concat!("valid regex: ", $pat)))
        }
    };
}

re!(re_scheme, r"^https?://[^/]+");
re!(
    re_env_prefix,
    r"^\$\{(?:API_URL|BASE_URL|baseUrl|apiUrl|VITE_API_URL|REACT_APP_API_URL)\}"
);
re!(re_js_template, r"\$\{[^}]+\}");
re!(re_spring_var, r"\{[a-zA-Z0-9_]+\}");
re!(re_angle_var, r"<[^>]+>");
re!(re_colon_var, r":[a-zA-Z0-9_]+");
re!(re_slashes, r"/+");

/// Standardize any API path or template URL to a universal wildcard path.
///
/// Examples:
///   - `http://localhost:12345/api/git/status?project_id=123` -> `/api/git/status`
///   - `${API_URL}/claude/sessions` -> `/api/claude/sessions`
///   - `/api/v1/users/{id}` (Spring) -> `/api/v1/users/*`
///   - `/api/v1/users/:id` (Express/Nest/Gin) -> `/api/v1/users/*`
///   - `/api/v1/users/${userId}` (React template) -> `/api/v1/users/*`
///   - `/api/v1/users/<int:id>/` (Django/Flask) -> `/api/v1/users/*`
pub fn normalize_url_path(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }

    // 1. Strip query strings and URL fragments
    let url = url.split('?').next().unwrap_or("").split('#').next().unwrap_or("").trim();

    // 2. Strip protocol and host (http://localhost:12345/api/...)
    let url = re_scheme().replace(url, "").to_string();

    // 3. Handle common frontend environment prefixes
    let url = re_env_prefix().replace(&url, "/api").to_string();

    // 4. Standardize template variables to universal '*' wildcard.
    // JS template strings ${...} first, to avoid leaving a dangling $.
    let url = re_js_template().replace_all(&url, "*").to_string();
    let url = re_spring_var().replace_all(&url, "*").to_string();
    let url = re_angle_var().replace_all(&url, "*").to_string();
    let url = re_colon_var().replace_all(&url, "*").to_string();

    // 5. Collapse multiple slashes and trim trailing slash
    let url = re_slashes().replace_all(&url, "/").to_string();
    let url = url.trim_end_matches('/').to_string();
    let url = if url.starts_with('/') {
        url
    } else {
        format!("/{url}")
    };

    url.to_lowercase()
}

#[cfg(test)]
mod normalize_tests {
    use super::normalize_url_path as n;

    #[test]
    fn docstring_examples() {
        assert_eq!(n("http://localhost:12345/api/git/status?project_id=123"), "/api/git/status");
        assert_eq!(n("${API_URL}/claude/sessions"), "/api/claude/sessions");
        assert_eq!(n("/api/v1/users/{id}"), "/api/v1/users/*");
        assert_eq!(n("/api/v1/users/:id"), "/api/v1/users/*");
        assert_eq!(n("/api/v1/users/${userId}"), "/api/v1/users/*");
        assert_eq!(n("/api/v1/users/<int:id>/"), "/api/v1/users/*");
    }

    #[test]
    fn empty_and_trivial() {
        assert_eq!(n(""), "");
        assert_eq!(n("/"), "/");
        assert_eq!(n("/api"), "/api");
    }

    #[test]
    fn fragments_and_query_strings() {
        assert_eq!(n("/api/x#frag"), "/api/x");
        assert_eq!(n("/api/x?a=1&b=2"), "/api/x");
        assert_eq!(n("/api/x?a=1#frag"), "/api/x");
    }

    #[test]
    fn scheme_and_host_stripped() {
        assert_eq!(n("https://example.com/api/users"), "/api/users");
        assert_eq!(n("http://api.example.com/v1/x"), "/v1/x");
    }

    #[test]
    fn env_prefix_variants() {
        assert_eq!(n("${BASE_URL}/x"), "/api/x");
        assert_eq!(n("${baseUrl}/x"), "/api/x");
        assert_eq!(n("${apiUrl}/x"), "/api/x");
        assert_eq!(n("${VITE_API_URL}/x"), "/api/x");
        assert_eq!(n("${REACT_APP_API_URL}/x"), "/api/x");
    }

    #[test]
    fn slash_collapsing_and_trailing_trim() {
        assert_eq!(n("//api//x//"), "/api/x");
        assert_eq!(n("api/x"), "/api/x");
        assert_eq!(n("/api/x/"), "/api/x");
    }

    #[test]
    fn lowercased_output() {
        assert_eq!(n("/API/Users/ID"), "/api/users/id");
        assert_eq!(n("http://EXAMPLE.com/API"), "/api");
        // Uppercase scheme is not stripped (Python's `^https?` is lowercase-only).
        assert_eq!(n("HTTP://EXAMPLE.COM/API"), "/http:/example.com/api");
    }

    #[test]
    fn wildcard_forms_collapse_to_star() {
        assert_eq!(n("/api/users/{userId}/posts"), "/api/users/*/posts");
        assert_eq!(n("/api/users/<uuid:uid>/x"), "/api/users/*/x");
        assert_eq!(n("/api/:module/:action"), "/api/*/*");
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteDefinition {
    /// GET, POST, PUT, DELETE, PATCH, ALL
    pub method: String,
    /// Raw path e.g. /api/v1/users/{id}
    pub path: String,
    /// Normalized e.g. /api/v1/users/*
    pub normalized_path: String,
    /// Relative file path
    pub file: String,
    /// Line number
    pub line: usize,
    /// Function / method name
    pub handler_name: String,
    /// Graph node_id if available
    pub handler_node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientApiCall {
    /// GET, POST, or ALL
    pub method: String,
    /// Original string literal
    pub raw_url: String,
    /// Normalized e.g. /api/v1/users/*
    pub normalized_path: String,
    /// Relative file path
    pub file: String,
    /// Line number
    pub line: usize,
    /// Enclosing function / component name
    pub caller_name: String,
    /// Graph node_id if available
    pub caller_node_id: String,
}

pub const BACKEND_EXTENSIONS: &[&str] = &[".java", ".py", ".go", ".cs", ".rb", ".php", ".ts", ".js"];
pub const FRONTEND_EXTENSIONS: &[&str] = &[".jsx", ".tsx", ".js", ".ts", ".vue", ".svelte", ".html"];

const SKIP_DIRS: &[&str] = &[
    "node_modules", "dist", "build", "target", "vendor", "venv", ".venv", "__pycache__", "bin",
    "obj", ".git", "coverage", "assets",
];

/// Universal bridge correlating frontend API calls with backend route handlers.
///
/// The store is borrowed, not owned: the tracer and the CLI both hold a
/// `&Store` for the same index, and `Connection` is neither `Clone` nor `Sync`.
pub struct EndpointBridge<'a> {
    root_dir: std::path::PathBuf,
    store: Option<&'a Store>,
    routes_by_path: HashMap<String, Vec<RouteDefinition>>,
    routes_by_handler: HashMap<String, Vec<RouteDefinition>>,
    calls_by_caller: HashMap<String, Vec<ClientApiCall>>,
    calls_by_path: HashMap<String, Vec<ClientApiCall>>,
    hook_aliases: HashMap<String, Vec<String>>,
    scanned: bool,
}

impl<'a> EndpointBridge<'a> {
    pub fn new(root_dir: &Path, store: Option<&'a Store>) -> Self {
        Self {
            root_dir: root_dir.to_path_buf(),
            store,
            routes_by_path: HashMap::new(),
            routes_by_handler: HashMap::new(),
            calls_by_caller: HashMap::new(),
            calls_by_path: HashMap::new(),
            hook_aliases: HashMap::new(),
            scanned: false,
        }
    }

    /// Map a discovered symbol to its persisted identity without guessing.
    fn symbol_node_id(&self, name: &str, file: &str, line: usize, require_containment: bool) -> String {
        let Some(store) = &self.store else {
            return format!("unresolved:{file}:{name}:{line}");
        };
        let file_n = file.replace('\\', "/");
        let candidates: Vec<_> = store
            .find_symbols_by_name(name, 1000)
            .into_iter()
            .filter(|s| s.file.replace('\\', "/") == file_n)
            .collect();

        if candidates.len() == 1 {
            let c = &candidates[0];
            if !require_containment || (c.line <= line && line <= c.end_line) {
                return c.id.clone();
            }
            return format!("unresolved:{file}:{name}:{line}");
        }

        let containing: Vec<_> = candidates
            .iter()
            .filter(|c| c.line <= line && line <= c.end_line)
            .collect();
        if containing.len() == 1 {
            return containing[0].id.clone();
        }
        // Keep a non-definition identity for missing or ambiguous metadata.
        format!("unresolved:{file}:{name}:{line}")
    }

    fn register_route(&mut self, mut route: RouteDefinition) {
        route.handler_node_id =
            self.symbol_node_id(&route.handler_name, &route.file, route.line, false);
        // Idempotent: `routes_in_file` scans a single file and a later full
        // `scan()` would otherwise register the same route twice.
        if let Some(existing) = self.routes_by_path.get(&route.normalized_path) {
            if existing
                .iter()
                .any(|r| r.method == route.method && r.file == route.file && r.line == route.line)
            {
                return;
            }
        }
        self.routes_by_path
            .entry(route.normalized_path.clone())
            .or_default()
            .push(route.clone());
        self.routes_by_handler
            .entry(route.handler_name.clone())
            .or_default()
            .push(route);
    }

    fn register_call(&mut self, call: ClientApiCall) {
        self.calls_by_caller
            .entry(call.caller_name.clone())
            .or_default()
            .push(call.clone());
        self.calls_by_path
            .entry(call.normalized_path.clone())
            .or_default()
            .push(call);
    }

    /// Perform full-stack route and API call discovery across project files.
    pub fn scan(&mut self) {
        if self.scanned || !self.root_dir.exists() {
            return;
        }

        let root = self.root_dir.clone();
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                if e.file_type().is_dir() {
                    let name = e.file_name().to_string_lossy().to_string();
                    return !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str());
                }
                true
            })
        {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let file = entry.file_name().to_string_lossy().to_string();
            if file.ends_with(".min.js") || file.ends_with(".min.css") || file.contains("bundle") {
                continue;
            }
            let full_path = entry.path();
            let rel_path = full_path
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| full_path.to_string_lossy().into_owned());

            // Skip compiled assets inside desktop or main/desktop.
            if rel_path.contains("desktop/ui/assets") || rel_path.contains("desktop/dist") {
                continue;
            }

            let ext = Path::new(&file)
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                .unwrap_or_default();

            if BACKEND_EXTENSIONS.contains(&ext.as_str()) {
                self.scan_backend_file(full_path, &rel_path, &ext);
            }
            if FRONTEND_EXTENSIONS.contains(&ext.as_str()) {
                self.scan_frontend_file(full_path, &rel_path);
            }
        }

        // Post-scan: resolve all hook aliases and caller links across files.
        let aliases_snapshot: Vec<(String, Vec<String>)> = self
            .hook_aliases
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (hook_name, aliases) in aliases_snapshot {
            let mut calls = self.calls_by_caller.get(&hook_name).cloned().unwrap_or_default();
            if calls.is_empty() {
                if let Some(m) = re_use_hook().captures(&hook_name) {
                    let ep_cap = m.get(1).unwrap().as_str();
                    let ep_name = format!("{}{}", ep_cap[..1].to_lowercase(), &ep_cap[1..]);
                    calls = self.calls_by_caller.get(&ep_name).cloned().unwrap_or_default();
                    if !calls.is_empty() {
                        self.calls_by_caller.insert(hook_name.clone(), calls.clone());
                    }
                }
            }
            if !calls.is_empty() {
                for alias in &aliases {
                    let slot = self.calls_by_caller.entry(alias.clone()).or_default();
                    for c in &calls {
                        if !slot.contains(c) {
                            slot.push(c.clone());
                        }
                    }
                }
            }
        }

        self.scanned = true;
    }

    /// Find backend routes matching a normalized client path using exact,
    /// wildcard, or suffix matching.
    pub fn match_routes(&mut self, normalized_client_path: &str) -> Vec<RouteDefinition> {
        self.scan();

        // 1. Exact match
        if let Some(routes) = self.routes_by_path.get(normalized_client_path) {
            return routes.clone();
        }

        // 2. Wildcard prefix match: /api/users/* matches /api/users/123
        let client_parts = split_parts(normalized_client_path);
        let mut matched: Vec<RouteDefinition> = Vec::new();
        for (route_path, routes) in &self.routes_by_path {
            let route_parts = split_parts(route_path);
            if client_parts.len() == route_parts.len() {
                let ok = client_parts
                    .iter()
                    .zip(route_parts.iter())
                    .all(|(&cp, &rp)| rp == "*" || cp == "*" || rp == cp);
                if ok {
                    matched.extend(routes.iter().cloned());
                }
            }
        }
        if !matched.is_empty() {
            return matched;
        }

        // 3. Suffix match (e.g. client '/phases' matches route '/dhp_api/v1/phases' or vice versa)
        for (route_path, routes) in &self.routes_by_path {
            let route_parts = split_parts(route_path);
            if client_parts.is_empty() || route_parts.is_empty() {
                continue;
            }
            let n = client_parts.len();
            let m = route_parts.len();
            if m > n && route_parts[m - n..] == client_parts[..] {
                matched.extend(routes.iter().cloned());
            } else if n > m && client_parts[n - m..] == route_parts[..] {
                matched.extend(routes.iter().cloned());
            }
        }
        matched
    }

    /// Find routes for an explicit `METHOD /path` trace target.
    pub fn get_routes_for_target(&mut self, method: &str, path: &str) -> Vec<RouteDefinition> {
        let routes = self.match_routes(&normalize_url_path(path));
        let method = method.to_uppercase();
        routes
            .into_iter()
            .filter(|r| r.method == method || r.method == "ALL")
            .collect()
    }

    /// Given a frontend component/function name, find all backend routes it triggers.
    pub fn get_routes_for_caller(
        &mut self,
        caller_name: &str,
        caller_file: Option<&str>,
    ) -> Vec<(ClientApiCall, Vec<RouteDefinition>)> {
        self.scan();
        let file_n = caller_file.map(|f| f.replace('\\', "/"));
        let client_calls: Vec<ClientApiCall> = self
            .calls_by_caller
            .get(caller_name)
            .map(|calls| {
                calls
                    .iter()
                    .filter(|c| file_n.as_deref().map(|f| f == c.file.replace('\\', "/")).unwrap_or(true))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let mut results = Vec::new();
        for call in client_calls {
            let matched_routes = self.match_routes(&call.normalized_path);
            results.push((call, matched_routes));
        }
        results
    }

    /// Given a backend handler name, find all frontend components calling it.
    pub fn get_client_callers_for_handler(
        &mut self,
        handler_name: &str,
        handler_file: Option<&str>,
    ) -> Vec<(RouteDefinition, Vec<ClientApiCall>)> {
        self.scan();
        let file_n = handler_file.map(|f| f.replace('\\', "/"));
        let routes: Vec<RouteDefinition> = self
            .routes_by_handler
            .get(handler_name)
            .map(|routes| {
                routes
                    .iter()
                    .filter(|r| file_n.as_deref().map(|f| f == r.file.replace('\\', "/")).unwrap_or(true))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let mut results = Vec::new();
        for r in routes {
            let mut client_calls = self
                .calls_by_path
                .get(&r.normalized_path)
                .cloned()
                .unwrap_or_default();
            if client_calls.is_empty() {
                // Try wildcard match against client paths.
                let r_parts = split_parts(&r.normalized_path);
                for (c_path, calls) in &self.calls_by_path {
                    let c_parts = split_parts(c_path);
                    if r_parts.len() == c_parts.len() {
                        let ok = r_parts
                            .iter()
                            .zip(c_parts.iter())
                            .all(|(&rp, &cp)| rp == "*" || cp == "*" || rp == cp);
                        if ok {
                            client_calls.extend(calls.iter().cloned());
                        }
                    }
                }
            }
            if !client_calls.is_empty() {
                results.push((r, client_calls));
            }
        }
        results
    }

    /// Routes defined inside one backend file, with module/class prefixes
    /// applied, in source order. Scans only that file — no project-wide walk —
    /// so `imc context` on one file does not pay for a full traversal.
    pub fn routes_in_file(&mut self, full_path: &Path) -> Vec<RouteDefinition> {
        let Ok(rel) = full_path.strip_prefix(&self.root_dir) else {
            return Vec::new();
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let ext = full_path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        if !BACKEND_EXTENSIONS.contains(&ext.as_str()) {
            return Vec::new();
        }
        self.scan_backend_file(full_path, &rel, &ext);
        let mut routes: Vec<RouteDefinition> = self
            .routes_by_path
            .values()
            .flatten()
            .filter(|r| r.file == rel)
            .cloned()
            .collect();
        routes.sort_by_key(|r| r.line);
        routes
    }

    /// `calls_by_caller` — client call sites for one frontend symbol name,
    /// scanning the project on first use (Python calls `bridge.scan()` here).
    pub fn client_calls(&mut self, caller_name: &str) -> Vec<ClientApiCall> {
        self.scan();
        self.calls_by_caller
            .get(caller_name)
            .cloned()
            .unwrap_or_default()
    }
}

fn split_parts(path: &str) -> Vec<&str> {
    path.split('/').filter(|p| !p.is_empty()).collect()
}

re!(re_use_hook, r"^use([A-Z0-9_][a-zA-Z0-9_]+?)(Mutation|Query)$");
re!(re_class_split, r"\b(?:public\s+)?class\s+\w+");
re!(
    re_spring_class_mapping,
    r#"@(?:RequestMapping)\s*\(\s*(?:(?:value|path)\s*=\s*)?["']([^"']+)["']"#
);
re!(
    re_spring_method,
    r#"@((?:Get|Post|Put|Delete|Patch|Request)Mapping)\s*(?:\(\s*(?:(?:value|path)\s*=\s*)?["']?([^"') \s]*)["']?\s*\))?"#
);
re!(
    re_java_fn,
    r#"(?:@\w+(?:\([^)]*\))?\s+)*(?:public|protected|private|static|final|abstract)?\s*[\w<>\[\],?]+\s+(\w+)\s*\("#
);
re!(re_py_prefix, r#"APIRouter\s*\([^)]*prefix\s*=\s*["']([^"']+)["']"#);
re!(re_py_route, r#"@\w+\.(get|post|put|delete|patch)\s*\(\s*["']([^"']*)["']"#);
re!(re_py_def, r"\b(?:async\s+)?def\s+([a-zA-Z0-9_]+)\s*\(");
re!(re_ctrl_prefix, r#"@Controller\s*\(\s*["']([^"']*)["']"#);
re!(
    re_express,
    r#"\b(?:router|app)\.(get|post|put|delete|patch)\s*\(\s*["']([^"']+)["']\s*,\s*([a-zA-Z0-9_]+)"#
);
re!(
    re_nest,
    r#"@((?:Get|Post|Put|Delete|Patch))\s*\(\s*["']?([^"') \s]*)["']?\s*\)"#
);
re!(re_ts_fn, r"\b(?:async\s+)?([a-zA-Z0-9_]+)\s*\(");
re!(
    re_go_route,
    r#"\.(GET|POST|PUT|DELETE|PATCH)\s*\(\s*["']([^"']+)["']\s*,\s*([a-zA-Z0-9_]+)"#
);
re!(
    re_dotnet,
    r#"\[(HttpGet|HttpPost|HttpPut|HttpDelete|Route)\s*\(\s*["']([^"']+)["']"#
);
re!(
    re_dotnet_fn,
    r#"\b(?:public|protected|private)\s+[\w<>\[\],\s]+\s+(\w+)\s*\("#
);

impl<'a> EndpointBridge<'a> {
    /// Scan a file for backend route definitions.
    fn scan_backend_file(&mut self, full_path: &Path, rel_path: &str, ext: &str) {
        let Ok(content) = std::fs::read_to_string(full_path) else { return };
        match ext {
            ".java" => self.scan_spring_routes(&content, rel_path),
            ".py" => self.scan_python_routes(&content, rel_path),
            ".js" | ".ts" => self.scan_node_routes(&content, rel_path),
            ".go" => self.scan_go_routes(&content, rel_path),
            ".cs" => self.scan_dotnet_routes(&content, rel_path),
            _ => {}
        }
    }

    /// Spring Boot route extraction (@RequestMapping + @GetMapping/@PostMapping).
    fn scan_spring_routes(&mut self, code: &str, rel_path: &str) {
        if !code.contains("@RestController") && !code.contains("@Controller") && !code.contains("@RequestMapping") {
            return;
        }

        // Split at class declaration to separate class-level from method-level.
        let (class_header, class_body) = match re_class_split().find(code) {
            Some(m) => (&code[..m.start()], &code[m.end()..]),
            None => ("", code),
        };

        let mut class_prefix = String::new();
        if let Some(m) = re_spring_class_mapping().captures(class_header) {
            class_prefix = m.get(1).unwrap().as_str().trim().to_string();
            if !class_prefix.starts_with('/') {
                class_prefix = format!("/{class_prefix}");
            }
        }

        let body_start_line = class_header.matches('\n').count();
        let lines = splitlines(class_body);
        for (i, line) in lines.iter().enumerate() {
            let Some(m) = re_spring_method().captures(line) else { continue };
            let anno = m.get(1).unwrap().as_str();
            let sub_path = m.get(2).map(|x| x.as_str().trim()).unwrap_or("").to_string();
            let http_method = if anno != "RequestMapping" {
                anno.replace("Mapping", "").to_uppercase()
            } else {
                "ALL".to_string()
            };

            // Find the method name in the lines that follow. Python walks
            // `range(i, min(i + 6, len(lines)))`: skipping `i` lines first is
            // what makes the search start *at the annotation*, not at line 1.
            let mut handler_name: Option<String> = None;
            let mut handler_index = i;
            for (hi, next_line) in lines.iter().enumerate().skip(i).take(6) {
                let Some(mm) = re_java_fn().captures(next_line) else { continue };
                let name = mm.get(1).unwrap().as_str().to_string();
                if !["public", "protected", "private", "return", "if", "for", "while"].contains(&name.as_str()) {
                    handler_name = Some(name);
                    handler_index = hi;
                    break;
                }
            }

            let Some(handler_name) = handler_name else { continue };

            let full_path = if !sub_path.is_empty() {
                let sp = if sub_path.starts_with('/') { sub_path } else { format!("/{sub_path}") };
                format!("{}{}", class_prefix.trim_end_matches('/'), sp)
            } else {
                class_prefix.clone()
            };
            let full_path = if full_path.is_empty() { "/".to_string() } else { full_path };

            let norm_path = normalize_url_path(&full_path);
            self.register_route(RouteDefinition {
                method: http_method,
                path: full_path,
                normalized_path: norm_path,
                file: rel_path.to_string(),
                line: body_start_line + handler_index + 1,
                handler_name,
                handler_node_id: String::new(),
            });
        }
    }

    /// FastAPI / Flask / Bottle route extraction.
    fn scan_python_routes(&mut self, code: &str, rel_path: &str) {
        let prefix = re_py_prefix()
            .captures(code)
            .map(|m| m.get(1).unwrap().as_str().trim().to_string())
            .unwrap_or_default();
        let prefix = if !prefix.is_empty() && !prefix.starts_with('/') {
            format!("/{prefix}")
        } else {
            prefix
        };

        let lines = splitlines(code);
        for (i, line) in lines.iter().enumerate() {
            let Some(m) = re_py_route().captures(line) else { continue };
            let http_method = m.get(1).unwrap().as_str().to_uppercase();
            let sub_path = m.get(2).unwrap().as_str().trim().to_string();

            let full_path = if !sub_path.is_empty() {
                let sp = if sub_path.starts_with('/') { sub_path } else { format!("/{sub_path}") };
                format!("{}{}", prefix.trim_end_matches('/'), sp)
            } else {
                prefix.clone()
            };
            let full_path = if full_path.is_empty() { "/".to_string() } else { full_path };

            let mut handler_name: Option<String> = None;
            let mut handler_index = i;
            for (hi, next_line) in lines.iter().enumerate().skip(i).take(5) {
                if let Some(fm) = re_py_def().captures(next_line) {
                    handler_name = Some(fm.get(1).unwrap().as_str().to_string());
                    handler_index = hi;
                    break;
                }
            }

            let Some(handler_name) = handler_name else { continue };

            let norm_path = normalize_url_path(&full_path);
            self.register_route(RouteDefinition {
                method: http_method,
                path: full_path,
                normalized_path: norm_path,
                file: rel_path.to_string(),
                line: handler_index + 1,
                handler_name,
                handler_node_id: String::new(),
            });
        }
    }

    /// Express / NestJS / Koa route extraction.
    fn scan_node_routes(&mut self, code: &str, rel_path: &str) {
        let lines = splitlines(code);
        // NestJS controller prefix.
        let prefix = re_ctrl_prefix()
            .captures(code)
            .map(|m| m.get(1).unwrap().as_str().trim().to_string())
            .unwrap_or_default();
        let prefix = if !prefix.is_empty() && !prefix.starts_with('/') {
            format!("/{prefix}")
        } else {
            prefix
        };

        for (i, line) in lines.iter().enumerate() {
            // Express: router.get('/path', handler) or app.get('/path', handler)
            if let Some(m) = re_express().captures(line) {
                let http_method = m.get(1).unwrap().as_str().to_uppercase();
                let sub_path = m.get(2).unwrap().as_str().trim().to_string();
                let handler_name = m.get(3).unwrap().as_str().to_string();
                let norm_path = normalize_url_path(&sub_path);
                self.register_route(RouteDefinition {
                    method: http_method,
                    path: sub_path,
                    normalized_path: norm_path,
                    file: rel_path.to_string(),
                    line: i + 1,
                    handler_name,
                    handler_node_id: String::new(),
                });
                continue;
            }

            // NestJS: @Get('/path') methodName()
            if let Some(m) = re_nest().captures(line) {
                let http_method = m.get(1).unwrap().as_str().to_uppercase();
                let sub_path = m.get(2).unwrap().as_str().trim().to_string();
                let full_path = format!(
                    "{}{}",
                    prefix.trim_end_matches('/'),
                    if sub_path.is_empty() {
                        String::new()
                    } else {
                        format!("/{}", sub_path.trim_start_matches('/'))
                    }
                );

                let mut handler_name: Option<String> = None;
                let mut handler_index = i;
                for (hi, next_line) in lines.iter().enumerate().skip(i).take(4) {
                    if let Some(fm) = re_ts_fn().captures(next_line) {
                        let name = fm.get(1).unwrap().as_str().to_string();
                        if !["constructor", "if", "for", "while"].contains(&name.as_str()) {
                            handler_name = Some(name);
                            handler_index = hi;
                            break;
                        }
                    }
                }

                if let Some(handler_name) = handler_name {
                    let full_path = if full_path.is_empty() { "/".to_string() } else { full_path };
                    let norm_path = normalize_url_path(&full_path);
                    self.register_route(RouteDefinition {
                        method: http_method,
                        path: full_path,
                        normalized_path: norm_path,
                        file: rel_path.to_string(),
                        line: handler_index + 1,
                        handler_name,
                        handler_node_id: String::new(),
                    });
                }
            }
        }
    }

    /// Go Gin / Echo / Chi route extraction.
    fn scan_go_routes(&mut self, code: &str, rel_path: &str) {
        let lines = splitlines(code);
        for (i, line) in lines.iter().enumerate() {
            if let Some(m) = re_go_route().captures(line) {
                let http_method = m.get(1).unwrap().as_str().to_uppercase();
                let raw_path = m.get(2).unwrap().as_str().trim().to_string();
                let handler_name = m.get(3).unwrap().as_str().to_string();
                let norm_path = normalize_url_path(&raw_path);
                self.register_route(RouteDefinition {
                    method: http_method,
                    path: raw_path,
                    normalized_path: norm_path,
                    file: rel_path.to_string(),
                    line: i + 1,
                    handler_name,
                    handler_node_id: String::new(),
                });
            }
        }
    }

    /// ASP.NET Core route extraction.
    fn scan_dotnet_routes(&mut self, code: &str, rel_path: &str) {
        let lines = splitlines(code);
        for (i, line) in lines.iter().enumerate() {
            if let Some(m) = re_dotnet().captures(line) {
                let anno = m.get(1).unwrap().as_str();
                let raw_path = m.get(2).unwrap().as_str().trim().to_string();
                let http_method = if anno != "Route" {
                    anno.replace("Http", "").to_uppercase()
                } else {
                    "ALL".to_string()
                };

                let mut handler_name: Option<String> = None;
                let mut handler_index = i;
                for (hi, next_line) in lines.iter().enumerate().skip(i).take(5) {
                    if let Some(fm) = re_dotnet_fn().captures(next_line) {
                        handler_name = Some(fm.get(1).unwrap().as_str().to_string());
                        handler_index = hi;
                        break;
                    }
                }

                if let Some(handler_name) = handler_name {
                    let norm_path = normalize_url_path(&raw_path);
                    self.register_route(RouteDefinition {
                        method: http_method,
                        path: raw_path,
                        normalized_path: norm_path,
                        file: rel_path.to_string(),
                        line: handler_index + 1,
                        handler_name,
                        handler_node_id: String::new(),
                    });
                }
            }
        }
    }

    /// Find the nearest enclosing function or component name above `target_line`.
    fn find_enclosing_symbol(&self, code_lines: &[String], target_line: usize, file_path: &str) -> String {
        // Search starting from target line itself backwards.
        if code_lines.is_empty() {
            return Path::new(file_path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| file_path.to_string());
        }
        let start = std::cmp::min(target_line.saturating_sub(1), code_lines.len() - 1);
        for line_idx in (0..=start).rev() {
            let line = &code_lines[line_idx];

            // 1. RTK Query endpoint: getUserById: builder.query(...)
            if let Some(m) = re_rtk_endpoint().captures(line) {
                return m.get(1).unwrap().as_str().to_string();
            }

            // 2. Function declaration: function Foo(...) / export default function Foo(...)
            if let Some(m) = re_fn_decl().captures(line) {
                return m.get(1).unwrap().as_str().to_string();
            }

            // 3. Arrow functions, incl. React wrappers such as
            //    `const fetchProjects = useCallback(async () => ...)`.
            if let Some(m) = re_arrow_fn().captures(line) {
                return m.get(1).unwrap().as_str().to_string();
            }

            // 4. Method definition in class or object: methodName(...) {
            if let Some(m) = re_method_def().captures(line) {
                let name = m.get(1).unwrap().as_str();
                if !["if", "for", "while", "switch", "catch"].contains(&name) {
                    return name.to_string();
                }
            }
        }

        // Fallback to file base component name (e.g. ProjectWorkspace).
        Path::new(file_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_path.to_string())
    }

    /// Scan a frontend file for fetch, axios, api calls, and RTK Query endpoints.
    fn scan_frontend_file(&mut self, full_path: &Path, rel_path: &str) {
        let Ok(content) = std::fs::read_to_string(full_path) else { return };
        let lines = splitlines(&content);

        // Base URL if defined in file (e.g. baseUrl: '/dhp_api/v1/' or baseURL: '/api').
        let file_base_url = re_base_url()
            .captures(&content)
            .map(|m| m.get(1).unwrap().as_str().trim().to_string())
            .unwrap_or_default();

        for (i, line) in lines.iter().enumerate() {
            // 1. Calls via fetch / axios / api. Capture template URLs with a
            //    dynamic base, e.g. `${API_URL}/projects`, not only literal paths.
            for raw_url in fetch_like_urls(line) {
                if raw_url.is_empty() {
                    continue;
                }
                let is_api_url = raw_url.contains("/api")
                    || raw_url.contains("/v")
                    || raw_url.starts_with('/')
                    || !file_base_url.is_empty()
                    || re_template_start().is_match(&raw_url);
                if !is_api_url {
                    continue;
                }

                let full_url = if !file_base_url.is_empty()
                    && !raw_url.starts_with("http")
                    && !raw_url.starts_with(&file_base_url)
                {
                    format!(
                        "{}/{}",
                        file_base_url.trim_end_matches('/'),
                        raw_url.trim_start_matches('/')
                    )
                } else {
                    raw_url.clone()
                };
                let norm_path = normalize_url_path(&full_url);
                let caller_name = self.find_enclosing_symbol(&lines, i + 1, rel_path);
                let method = re_axios_method()
                    .captures(line)
                    .map(|m| m.get(1).unwrap().as_str().to_uppercase())
                    .unwrap_or_else(|| "GET".to_string());
                let caller_node_id =
                    self.symbol_node_id(&caller_name, rel_path, i + 1, true);
                self.register_call(ClientApiCall {
                    method,
                    raw_url,
                    normalized_path: norm_path,
                    file: rel_path.to_string(),
                    line: i + 1,
                    caller_name,
                    caller_node_id,
                });
            }

            // 2. RTK Query endpoint definitions:
            //    query: () => '/api/users'
            //    query: (arg) => ({ url: 'phases/', method: 'POST', data: arg })
            //    url: 'phases/'
            let rtk = re_rtk_url()
                .captures(line)
                .or_else(|| re_rtk_query().captures(line));
            if let Some(m) = rtk {
                let raw_url = m.get(1).unwrap().as_str().trim().to_string();
                let caller_name = self.find_enclosing_symbol(&lines, i + 1, rel_path);

                let full_url = if !file_base_url.is_empty()
                    && !raw_url.starts_with("http")
                    && !raw_url.starts_with(&file_base_url)
                {
                    format!(
                        "{}/{}",
                        file_base_url.trim_end_matches('/'),
                        raw_url.trim_start_matches('/')
                    )
                } else if !raw_url.starts_with('/') {
                    format!("/{raw_url}")
                } else {
                    raw_url.clone()
                };

                let norm_path = normalize_url_path(&full_url);

                let mut method = "ALL".to_string();
                if let Some(mm) = re_rtk_method().captures(line) {
                    method = mm.get(1).unwrap().as_str().to_uppercase();
                } else {
                    let lo = i.saturating_sub(2);
                    let hi = std::cmp::min(lines.len(), i + 4);
                    for nearby in lines.iter().take(hi).skip(lo) {
                        if let Some(mn) = re_rtk_method().captures(nearby) {
                            method = mn.get(1).unwrap().as_str().to_uppercase();
                            break;
                        }
                    }
                }
                if method == "ALL" {
                    let lo = i.saturating_sub(5);
                    let hi = std::cmp::min(lines.len(), i + 5);
                    let context_chunk = lines[lo..hi].join("\n");
                    if context_chunk.contains("builder.mutation") {
                        method = "POST".to_string();
                    } else if context_chunk.contains("builder.query") {
                        method = "GET".to_string();
                    }
                }

                let caller_node_id =
                    self.symbol_node_id(&caller_name, rel_path, i + 1, true);
                let call = ClientApiCall {
                    method,
                    raw_url: raw_url.clone(),
                    normalized_path: norm_path.clone(),
                    file: rel_path.to_string(),
                    line: i + 1,
                    caller_name: caller_name.clone(),
                    caller_node_id,
                };
                self.register_call(call.clone());

                // Also register relative path if baseUrl was used.
                let short_input = if raw_url.starts_with('/') {
                    raw_url.clone()
                } else {
                    format!("/{raw_url}")
                };
                let short_norm = normalize_url_path(&short_input);
                if short_norm != norm_path {
                    self.calls_by_path.entry(short_norm).or_default().push(call.clone());
                }

                // Also link auto-generated RTK hooks: use<Endpoint>Query/Mutation.
                let mut cap_iter = caller_name.chars();
                let cap_name = match cap_iter.next() {
                    Some(first) => {
                        format!("{}{}", first.to_uppercase(), cap_iter.as_str())
                    }
                    None => caller_name.clone(),
                };
                for hook_name in [format!("use{cap_name}Query"), format!("use{cap_name}Mutation")] {
                    self.calls_by_caller.entry(hook_name.clone()).or_default().push(call.clone());
                    let aliases = self.hook_aliases.get(&hook_name).cloned().unwrap_or_default();
                    for alias in aliases {
                        self.calls_by_caller.entry(alias).or_default().push(call.clone());
                    }
                }
            }

            // 3. Detect destructured hook aliases in components, e.g.
            //    const [createPhase, { isLoading }] = useCreatePhaseMutation();
            if let Some(m) = re_hook_array().captures(line) {
                let alias_name = m.get(1).unwrap().as_str().to_string();
                let hook_name = m.get(2).unwrap().as_str().to_string();
                self.hook_aliases.entry(hook_name.clone()).or_default().push(alias_name.clone());
                let calls = self.calls_by_caller.get(&hook_name).cloned().unwrap_or_default();
                for c in calls {
                    self.calls_by_caller.entry(alias_name.clone()).or_default().push(c);
                }
            }

            // e.g.: const { data: phases } = useGetPhasesQuery();
            if let Some(m) = re_hook_object().captures(line) {
                let hook_name = m.get(1).unwrap().as_str().to_string();
                let caller_name = self.find_enclosing_symbol(&lines, i + 1, rel_path);
                let calls = self.calls_by_caller.get(&hook_name).cloned().unwrap_or_default();
                for c in calls {
                    self.calls_by_caller.entry(caller_name.clone()).or_default().push(c);
                }
            }
        }

        // 4. Detect exported RTK Query hooks in apis definition file, e.g.
        //    export const { useCreatePhaseMutation, useGetPhaseByFlowIdQuery } = apis;
        for m in re_exported_destr().captures_iter(&content) {
            let inner = m.get(1).unwrap().as_str();
            for hook in inner.split(',').map(|h| h.trim()).filter(|h| !h.is_empty()) {
                let Some(mh) = re_use_hook().captures(hook) else { continue };
                let ep_cap = mh.get(1).unwrap().as_str();
                let ep_name = format!("{}{}", &ep_cap[..1].to_lowercase(), &ep_cap[1..]);
                if let Some(calls) = self.calls_by_caller.get(&ep_name).cloned() {
                    for call in calls {
                        let slot = self.calls_by_caller.entry(hook.to_string()).or_default();
                        if !slot.contains(&call) {
                            slot.push(call);
                        }
                    }
                }
            }
        }
    }
}

re!(re_rtk_endpoint, r"\b([a-zA-Z0-9_]+)\s*:\s*builder\.(?:query|mutation)\b");
re!(re_fn_decl, r"\bfunction\s+([a-zA-Z0-9_]+)\s*\(");
re!(
    re_arrow_fn,
    r"\b(?:export\s+)?(?:const|let|var)\s+([a-zA-Z0-9_]+)\s*=\s*(?:(?:useCallback|useMemo)\s*\(\s*)?(?:async\s*)?(?:\([^)]*\)|[a-zA-Z0-9_]+)\s*=>"
);
re!(
    re_method_def,
    r"^\s*(?:async\s+)?([a-zA-Z0-9_]+)\s*\([^)]*\)\s*\{"
);
re!(re_base_url, r#"\b(?:baseUrl|baseURL)\s*:\s*[`"']([^`"']+)['"`]"#);
re!(re_template_start, r"^\$\{[^}]+\}/");
re!(re_axios_method, r"(?i)\b(?:axios|api)\.(get|post|put|delete|patch)\s*\(");
re!(re_rtk_url, r#"\burl\s*:\s*[`"']([^`"'\s?]+)"#);
re!(
    re_rtk_query,
    r#"\bquery\s*:\s*(?:\([^)]*\)\s*=>\s*)?[`"']([^`"'\s?]+)"#
);
re!(re_rtk_method, r#"\bmethod\s*:\s*[`"']([A-Za-z]+)["'`]"#);
re!(
    re_hook_array,
    r"(?:const|let|var)\s*\[\s*([a-zA-Z0-9_]+)[^\]]*\]\s*=\s*(use[A-Z0-9_]+(?:Mutation|Query))\b"
);
re!(
    re_hook_object,
    r"(?:const|let|var)\s*\{[^}]*\}\s*=\s*(use[A-Z0-9_]+(?:Mutation|Query))\b"
);
re!(
    re_exported_destr,
    r"(?:export\s+)?(?:const|let|var)\s*\{([^}]+)\}\s*=\s*([a-zA-Z0-9_]+)\b"
);

/// Collect `fetch(...)`, `axios.<verb>(...)` and `api.<verb>(...)` string
/// literals on a line, ordered by their position in the source.
///
/// The Python pattern `([\`"'])(.*?)\1` uses a back-reference, which the Rust
/// regex engine does not support. The three quote flavours are mutually
/// exclusive on the opening character, so running them separately and merging
/// by match offset reproduces the original left-to-right `finditer` order.
fn fetch_like_urls(line: &str) -> Vec<String> {
    let base = r"(?:fetch|axios\.(?:get|post|put|delete|patch)|api\.(?:get|post|put|delete|patch))\s*\(\s*";
    let mut hits: Vec<(usize, String)> = Vec::new();
    for q in ["`", "\"", "'"] {
        let pattern = format!("{base}{q}(.*?){q}");
        let Ok(re) = Regex::new(&pattern) else { continue };
        for caps in re.captures_iter(line) {
            if let Some(m) = caps.get(1) {
                hits.push((m.start(), m.as_str().to_string()));
            }
        }
    }
    hits.sort_by_key(|(start, _)| *start);
    hits.into_iter().map(|(_, url)| url).collect()
}

#[cfg(test)]
mod bridge_tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("imc-endpoint-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn python_fastapi_route() {
        let root = tmpdir("fastapi");
        write(
            &root,
            "api/routes.py",
            "router = APIRouter(prefix='/api/v1/users')\n\n@router.get('/{user_id}')\nasync def get_user(user_id: str):\n    return {}\n",
        );
        let mut b = EndpointBridge::new(&root, None);
        b.scan();
        let routes = b.get_routes_for_target("GET", "/api/v1/users/42");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].handler_name, "get_user");
        assert_eq!(routes[0].normalized_path, "/api/v1/users/*");
        assert_eq!(routes[0].method, "GET");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn express_route_and_frontend_call_correlate() {
        let root = tmpdir("express");
        write(&root, "server/routes.js", "router.get('/api/items/:id', getItem);\n");
        write(
            &root,
            "web/api.js",
            "export function loadItem() {\n  return axios.get('/api/items/7');\n}\n",
        );
        let mut b = EndpointBridge::new(&root, None);
        b.scan();

        let routes = b.get_routes_for_target("GET", "/api/items/7");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].handler_name, "getItem");

        let by_caller = b.get_routes_for_caller("loadItem", None);
        assert_eq!(by_caller.len(), 1);
        assert_eq!(by_caller[0].1.len(), 1);

        let by_handler = b.get_client_callers_for_handler("getItem", None);
        assert_eq!(by_handler.len(), 1);
        assert_eq!(by_handler[0].1.len(), 1);
        assert_eq!(by_handler[0].1[0].caller_name, "loadItem");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn go_gin_route() {
        let root = tmpdir("gin");
        write(&root, "main.go", "func setup() {\n\trouter.GET(\"/api/health\", healthHandler)\n}\n");
        let mut b = EndpointBridge::new(&root, None);
        b.scan();
        let routes = b.get_routes_for_target("GET", "/api/health");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].handler_name, "healthHandler");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dotnet_route() {
        let root = tmpdir("dotnet");
        write(
            &root,
            "C.cs",
            "public class C {\n  [HttpGet(\"/api/orders\")]\n  public IActionResult GetOrders() { return null; }\n}\n",
        );
        let mut b = EndpointBridge::new(&root, None);
        b.scan();
        let routes = b.get_routes_for_target("GET", "/api/orders");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].handler_name, "GetOrders");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn spring_controller_prefix_and_method_mapping() {
        let root = tmpdir("spring");
        write(
            &root,
            "UserController.java",
            "@RestController\n@RequestMapping(\"/api/v1\")\npublic class UserController {\n  @GetMapping(\"/users\")\n  public List<User> list() { return null; }\n}\n",
        );
        let mut b = EndpointBridge::new(&root, None);
        b.scan();
        let routes = b.get_routes_for_target("GET", "/api/v1/users");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].handler_name, "list");
        assert_eq!(routes[0].method, "GET");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rtk_query_endpoint_detected() {
        let root = tmpdir("rtk");
        write(
            &root,
            "web/store.js",
            "export const apis = {\n  getProjects: builder.query({ query: () => '/api/projects' }),\n}\n",
        );
        let mut b = EndpointBridge::new(&root, None);
        b.scan();
        let calls = b.get_routes_for_caller("getProjects", None);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0.method, "GET");
        assert_eq!(calls[0].0.normalized_path, "/api/projects");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skipped_dirs_and_bundles_ignored() {
        let root = tmpdir("skip");
        write(&root, "node_modules/pkg/routes.js", "router.get('/api/should-not-appear', h);\n");
        write(&root, "web/main.bundle.js", "router.get('/api/bundled', h2);\n");
        write(&root, "web/app.min.js", "router.get('/api/minified', h3);\n");
        let mut b = EndpointBridge::new(&root, None);
        b.scan();
        assert!(b.get_routes_for_target("GET", "/api/should-not-appear").is_empty());
        assert!(b.get_routes_for_target("GET", "/api/minified").is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fetch_like_urls_preserves_order_and_quotes() {
        let line = "axios.get('/api/a'); fetch(`/api/b`); api.post(\"/api/c\");";
        assert_eq!(fetch_like_urls(line), vec!["/api/a", "/api/b", "/api/c"]);
    }

    #[test]
    fn find_enclosing_symbol_variants() {
        let lines: Vec<String> = [
            "function outer() {",
            "  const load = async () => {",
            "    return fetch('/api/x');",
            "  };",
            "}",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let b = EndpointBridge::new(Path::new("/nonexistent"), None);
        assert_eq!(b.find_enclosing_symbol(&lines, 3, "web/f.ts"), "load");
        assert_eq!(b.find_enclosing_symbol(&lines, 2, "web/f.ts"), "load");
        // No enclosing symbol found -> file stem fallback.
        assert_eq!(b.find_enclosing_symbol(&lines, 1, "web/f.ts"), "outer");
        assert_eq!(b.find_enclosing_symbol(&[], 1, "web/Thing.tsx"), "Thing");
    }
}
