//! Faithful Rust port of `main/cli/commands/context.py` — the Feature Slice &
//! Context Dossier builder (`imc-context`).
//!
//! Builds a one-shot architectural dossier for a feature, component or
//! controller: resolved subcomponents, styles, hooks, API calls, i18n values
//! and tests.
//!
//! Public functions return `String` / `Option<_>` / `Result<_>` and print
//! nothing; callers render.
//!
//! Divergences from the Python (which is unavoidable in Rust):
//!   * `ProjectManager` project resolution is replaced by an explicit
//!     `root_dir: &Path` argument.
//!   * The semantic-search-index fallback in `locate_target_file` (step 3, which
//!     used `Searcher`) is omitted — it needs an embedder + project DB handle.
//!     The filesystem filename scan and local keyword scoring (steps 2 / 2b) are
//!     ported faithfully.
//!   * The upstream-caller lookup (ASTTracer) uses the optional `&Store`
//!     argument; callers are reported as short symbol names.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use imc_core::store::Store;
use imc_support::endpoints::EndpointBridge;
use regex::Regex;
use serde_json::Value;
use walkdir::WalkDir;

// ─── Feature context ─────────────────────────────────────────────────────────

/// Port of `get_feature_context`. Returns the markdown dossier, or an error
/// string (the Python returned error text, not exceptions).
pub fn get_feature_context(target: &str, root_dir: &Path, store: Option<&Store>) -> String {
    let source_file = match locate_target_file(target, root_dir) {
        Some(f) if f.is_file() => f,
        _ => {
            return format!(
                "Error: Could not locate target file or component for: '{target}'. Please specify an existing file path."
            )
        }
    };

    let content = match std::fs::read_to_string(&source_file) {
        Ok(c) => c,
        Err(e) => return format!("Error reading file '{}': {e}", source_file.display()),
    };

    // Compute a project-relative path, with a marker-based fallback.
    let (rel_path, effective_root) = compute_rel_path(&source_file, root_dir);
    let root_dir = effective_root;

    if is_backend_file(&source_file) {
        return build_backend_dossier(&source_file, &rel_path, &content, &root_dir, store);
    }

    build_ui_dossier(&source_file, &rel_path, &content, &root_dir)
}

fn compute_rel_path(source_file: &Path, root_dir: &Path) -> (String, PathBuf) {
    if let Ok(rel) = source_file.strip_prefix(root_dir) {
        return (rel.display().to_string(), root_dir.to_path_buf());
    }

    // Fallback: walk up looking for a repo marker or a `src` parent.
    let mut cur = source_file.parent().map(|p| p.to_path_buf());
    let mut found_root: Option<PathBuf> = None;
    while let Some(dir) = cur.clone() {
        if dir.join(".git").exists()
            || dir.join("package.json").exists()
            || dir.join("pyproject.toml").exists()
        {
            found_root = Some(dir);
            break;
        }
        if dir.file_name().and_then(|n| n.to_str()) == Some("src") {
            found_root = dir.parent().map(|p| p.to_path_buf());
            break;
        }
        cur = dir.parent().map(|p| p.to_path_buf());
        if cur.is_none() {
            break;
        }
    }
    let new_root = found_root
        .or_else(|| source_file.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| root_dir.to_path_buf());
    let rel = match source_file.strip_prefix(&new_root) {
        Ok(r) => r.display().to_string(),
        Err(_) => source_file.display().to_string(),
    };
    (rel, new_root)
}

/// Port of `is_backend_file` — Java, Python, Go, C#, or server-side TS/JS.
pub fn is_backend_file(source_file: &Path) -> bool {
    let ext = ext_lower(source_file);
    if matches!(
        ext.as_str(),
        ".java" | ".py" | ".go" | ".rs" | ".cs" | ".php" | ".rb" | ".kt" | ".scala"
    ) {
        return true;
    }
    if ext == ".ts" || ext == ".js" {
        let fp_lower = path_lower(source_file);
        if [
            "/service",
            "/controller",
            "/server",
            "/backend",
            "/api/",
            "/routes/",
            "/router/",
            "/workflow/",
            "/dao/",
            "/repository/",
        ]
        .iter()
        .any(|seg| fp_lower.contains(seg))
        {
            return true;
        }
    }
    false
}

// ─── Backend dossier ─────────────────────────────────────────────────────────

/// Port of `build_backend_dossier` for backend classes/services/controllers.
pub fn build_backend_dossier(
    source_file: &Path,
    rel_path: &str,
    content: &str,
    root_dir: &Path,
    store: Option<&Store>,
) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let ext = ext_lower(source_file);

    // 1. Class / structure declaration & annotations.
    let mut class_name = stem_of(source_file);
    let mut class_decl = String::new();
    let mut annotations: Vec<String> = Vec::new();

    if ext == ".java" {
        let ann_re = Regex::new(r"@([A-Za-z0-9_]+(?:\([^)]*\))?)").unwrap();
        let cls_re = Regex::new(
            r"((?:@\w+(?:\([^)]*\))?\s+)*(?:public\s+|protected\s+|private\s+)?(?:class|interface|record|enum)\s+([A-Za-z0-9_]+)(?:<[^>]+>)?(?:\s+extends\s+[A-Za-z0-9_<>,\s]+)?(?:\s+implements\s+[A-Za-z0-9_<>,\s]+)?)",
        )
        .unwrap();
        if let Some(cap) = cls_re.captures(content) {
            let full = cap.get(1).map(|m| m.as_str()).unwrap_or("").trim();
            class_decl = full.split_whitespace().collect::<Vec<_>>().join(" ");
            class_name = cap.get(2).map(|m| m.as_str()).unwrap_or(&class_name).to_string();
        }
        const ANN_NAMES: [&str; 15] = [
            "Component", "Service", "RestController", "Controller", "Repository",
            "Configuration", "Slf4j", "Observed", "RequestMapping", "Entity", "Table",
            "Aspect", "Transactional", "RequiredArgsConstructor", "AllArgsConstructor",
        ];
        for m in ann_re.captures_iter(content) {
            let a = m.get(1).map(|x| x.as_str()).unwrap_or("");
            let base = a.split('(').next().unwrap_or("");
            if ANN_NAMES.contains(&base) {
                annotations.push(format!("@{a}"));
            }
        }
    } else if ext == ".py" {
        let cls_re = Regex::new(r"class\s+([A-Za-z0-9_]+)(?:\(([^)]*)\))?:").unwrap();
        if let Some(cap) = cls_re.captures(content) {
            class_name = cap.get(1).map(|m| m.as_str()).unwrap_or(&class_name).to_string();
            let bases = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            class_decl = if bases.is_empty() {
                format!("class {class_name}")
            } else {
                format!("class {class_name}({bases})")
            };
        }
    } else if ext == ".go" {
        let struct_re = Regex::new(r"type\s+([A-Za-z0-9_]+)\s+struct").unwrap();
        if let Some(cap) = struct_re.captures(content) {
            class_name = cap.get(1).map(|m| m.as_str()).unwrap_or(&class_name).to_string();
            class_decl = format!("type {class_name} struct");
        }
    }

    // 2. Injected dependencies & components.
    let mut injected_deps: Vec<String> = Vec::new();
    if ext == ".java" {
        let ctor_re = Regex::new(&format!(
            r"(?:public|protected)\s+{}\s*\(([^)]+)\)",
            regex::escape(&class_name)
        ))
        .unwrap();
        if let Some(cap) = ctor_re.captures(content) {
            for param in cap.get(1).map(|m| m.as_str()).unwrap_or("").split(',') {
                let clean = param.split_whitespace().collect::<Vec<_>>().join(" ");
                if !clean.is_empty() {
                    injected_deps.push(clean);
                }
            }
        }
        let field_re = Regex::new(
            r"(?:@Autowired\s+)?private\s+final\s+([A-Za-z0-9_<>]+)\s+([A-Za-z0-9_]+)\s*;",
        )
        .unwrap();
        for cap in field_re.captures_iter(content) {
            let item = format!(
                "{} {}",
                cap.get(1).map(|m| m.as_str()).unwrap_or(""),
                cap.get(2).map(|m| m.as_str()).unwrap_or("")
            );
            if !injected_deps.contains(&item) {
                injected_deps.push(item);
            }
        }
    } else if ext == ".py" {
        let init_re = Regex::new(r"def __init__\s*\(\s*self,?\s*([^)]*)\)").unwrap();
        if let Some(cap) = init_re.captures(content) {
            for param in cap.get(1).map(|m| m.as_str()).unwrap_or("").split(',') {
                let clean = param.trim();
                if !clean.is_empty() {
                    injected_deps.push(clean.to_string());
                }
            }
        }
    }

    // 3. HTTP endpoints & canonical routes.
    let mut endpoints: Vec<String> = Vec::new();
    if ext == ".java" {
        let mut base_route = String::new();
        let base_re = Regex::new(r#"@RequestMapping\s*\(\s*(?:value\s*=\s*)?["']([^"']+)["']"#).unwrap();
        if let Some(cap) = base_re.captures(content) {
            base_route = cap.get(1).map(|m| m.as_str()).unwrap_or("").trim().to_string();
            if !base_route.starts_with('/') {
                base_route = format!("/{base_route}");
            }
            base_route = base_route.trim_end_matches('/').to_string();
        }
        let route_re = Regex::new(
            r#"@((?:Get|Post|Put|Delete|Patch)Mapping)\s*(?:\(\s*(?:value\s*=\s*)?["']?([^"'\) ]*)["']?\s*\))?"#,
        )
        .unwrap();
        let fn_re = Regex::new(
            r"\b(?:public|protected|private)\s+[A-Za-z0-9_<>\[\],\s]+\s+([A-Za-z0-9_]+)\s*\(",
        )
        .unwrap();
        for (i, line) in lines.iter().enumerate() {
            if let Some(cap) = route_re.captures(line) {
                let verb = cap
                    .get(1)
                    .map(|m| m.as_str())
                    .unwrap_or("")
                    .replace("Mapping", "")
                    .to_uppercase();
                let sub = cap.get(2).map(|m| m.as_str()).unwrap_or("").trim().trim_matches(['"', '\'']).to_string();
                let full_ep = if sub.is_empty() {
                    base_route.clone()
                } else {
                    format!("{}/{}", base_route, sub.trim_start_matches('/'))
                };
                let display_ep = if full_ep.is_empty() { "/".to_string() } else { full_ep };
                let mut method_name = String::new();
                for next_l in lines.iter().skip(i).take(4) {
                    if let Some(fc) = fn_re.captures(next_l) {
                        method_name = fc.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                        break;
                    }
                }
                endpoints.push(format!(
                    "`{verb} {display_ep}` → `{method_name}()` [line {}]",
                    i + 1
                ));
            }
        }
    } else if ext == ".py" {
        // Route discovery goes through EndpointBridge — the same scanner
        // `imc trace` and frontend->backend linking use — so module prefixes
        // apply: `APIRouter(prefix="/api/git")` + `@router.get("/status")`
        // serves `/api/git/status`, not `/status`. Python's copy of this
        // (main/cli/commands/context.py:488) reads only `@app.` / `@router.`
        // and drops the prefix; that is a bug, not behaviour to match.
        let mut bridge = EndpointBridge::new(root_dir, None);
        for route in bridge.routes_in_file(source_file) {
            let line = decorator_line(&lines, &route.method, &route.path).unwrap_or(route.line);
            endpoints.push(format!("`{} {}` [line {line}]", route.method, route.path));
        }
    }

    // 4. Public methods & key operations.
    let mut public_methods: Vec<String> = Vec::new();
    if ext == ".java" {
        // Note: the Python used a `(?!class|...)` negative lookahead, which the
        // Rust regex crate does not support — filtered in code below instead.
        let fn_re = Regex::new(
            r"^\s*(?:@Override\s+)?(?:public|protected)\s+([A-Za-z0-9_<>\[\]]+)\s+([A-Za-z0-9_]+)\s*\(([^)]*)\)",
        )
        .unwrap();
        for (i, line) in lines.iter().enumerate() {
            if let Some(cap) = fn_re.captures(line) {
                let ret_type = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                let fn_name = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                if matches!(ret_type, "class" | "interface" | "record" | "enum") {
                    continue;
                }
                if fn_name == class_name {
                    continue;
                }
                let params = cap.get(3).map(|m| m.as_str()).unwrap_or("").trim();
                public_methods.push(format!("`{fn_name}({params})`: {ret_type} [line {}]", i + 1));
            }
        }
    } else if ext == ".py" {
        let fn_re = Regex::new(r"^\s*def\s+([a-zA-Z0-9_]+)\s*\(([^)]*)\)").unwrap();
        for (i, line) in lines.iter().enumerate() {
            if let Some(cap) = fn_re.captures(line) {
                let fname = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                if fname.starts_with('_') {
                    continue;
                }
                let params = cap.get(2).map(|m| m.as_str()).unwrap_or("").trim();
                public_methods.push(format!("`{fname}({params})` [line {}]", i + 1));
            }
        }
    }

    // 5. Upstream callers (Store-backed; optional).
    let mut upstream_callers: Vec<String> = Vec::new();
    if let Some(store) = store {
        for c in store.callers(&class_name).into_iter().take(5) {
            let short = c.rsplit("::").next().unwrap_or(&c).to_string();
            upstream_callers.push(format!("`{short}`"));
        }
    }

    // 6. Associated tests.
    let tests = find_related_tests(root_dir, source_file);

    let mut output: Vec<String> = vec![rel_path.to_string()];
    if !class_decl.is_empty() {
        output.push(format!("declaration: {class_decl}"));
    }
    if !annotations.is_empty() {
        output.push(format!("role: {}", annotations.join(", ")));
    }
    if !injected_deps.is_empty() {
        output.push(format!("dependencies: {}", injected_deps.join(", ")));
    }
    if !endpoints.is_empty() {
        output.push("endpoints:".to_string());
        for e in &endpoints {
            output.push(format!("- {e}"));
        }
    }
    if !public_methods.is_empty() {
        output.push("operations:".to_string());
        for m in public_methods.iter().take(12) {
            output.push(format!("- {m}"));
        }
        if public_methods.len() > 12 {
            output.push(format!("- +{} more", public_methods.len() - 12));
        }
    }
    if !upstream_callers.is_empty() {
        output.push(format!("callers: {}", upstream_callers.join(", ")));
    }
    if !tests.is_empty() {
        output.push(format!("tests: {}", tests.join(", ")));
    }

    output.join("\n")
}

/// The dossier reports the *decorator's* line while the bridge reports the
/// handler's `def` line, so re-locate the decorator to keep `[line N]`
/// identical to the Python output. Prefixes are file-wide, so a matching verb
/// plus path suffix identifies the decorator uniquely within one file.
fn decorator_line(lines: &[&str], method: &str, path: &str) -> Option<usize> {
    let re = Regex::new(r#"@\w+\.(get|post|put|delete|patch)\s*\(\s*["']([^"']+)["']"#).ok()?;
    for (i, line) in lines.iter().enumerate() {
        let Some(cap) = re.captures(line) else { continue };
        if !cap[1].eq_ignore_ascii_case(method) {
            continue;
        }
        let lit = cap[2].trim().trim_start_matches('/');
        if lit.is_empty() {
            if path == "/" {
                return Some(i + 1);
            }
            continue;
        }
        if path == lit || path.ends_with(&format!("/{lit}")) {
            return Some(i + 1);
        }
    }
    None
}

// ─── UI / frontend dossier ───────────────────────────────────────────────────

fn build_ui_dossier(
    source_file: &Path,
    rel_path: &str,
    content: &str,
    root_dir: &Path,
) -> String {
    let import_re = Regex::new(
        r#"import\s+(?:([\w*{},$\s]+)\s+from\s+)?['"]([^'"]+)['"];?"#,
    )
    .unwrap();

    let mut resolved_components: Vec<(String, String, Option<String>)> = Vec::new();
    let mut resolved_styles: Vec<String> = Vec::new();
    let mut resolved_data: Vec<(String, String)> = Vec::new(); // (rel, clause)
    let mut external_deps: Vec<String> = Vec::new();

    let t_key_re = Regex::new(r#"\bt\(['"]([A-Za-z0-9_]+)['"]"#).unwrap();
    let ikey_re = Regex::new(r#"i18nKey=['"]([A-Za-z0-9_]+)['"]"#).unwrap();
    let mut all_i18n_keys: BTreeSet<String> = BTreeSet::new();
    collect_keys(content, &t_key_re, &ikey_re, &mut all_i18n_keys);

    for cap in import_re.captures_iter(content) {
        let clause = cap.get(1).map(|m| m.as_str()).unwrap_or("").trim().to_string();
        let module = cap.get(2).map(|m| m.as_str()).unwrap_or("").to_string();
        let label = if clause.is_empty() { module.clone() } else { clause.clone() };

        match resolve_module_path(root_dir, source_file, &module) {
            Some(resolved) if resolved.exists() => {
                let res_rel = rel_to(root_dir, &resolved).unwrap_or_else(|| resolved.display().to_string());
                let ext = ext_lower(&resolved);
                if (ext == ".tsx" || ext == ".jsx") && has_upper_identifier(&clause) {
                    let props = extract_props_interface(&resolved);
                    if let Ok(child_txt) = std::fs::read_to_string(&resolved) {
                        collect_keys(&child_txt, &t_key_re, &ikey_re, &mut all_i18n_keys);
                    }
                    resolved_components.push((res_rel, label, props));
                } else if matches!(ext.as_str(), ".css" | ".scss" | ".less") {
                    resolved_styles.push(res_rel);
                } else {
                    resolved_data.push((res_rel, label));
                }
            }
            _ => {
                if !module.starts_with('.') {
                    external_deps.push(module);
                }
            }
        }
    }

    let hook_re = Regex::new(r"\b(use[A-Z]\w+)\b").unwrap();
    let route_re = Regex::new(r#"["'](/[\w:\-/]+)["']"#).unwrap();
    let api_re = Regex::new(r"\b(\w+Api\.\w+)\b").unwrap();

    let mut hooks: BTreeSet<String> = BTreeSet::new();
    for m in hook_re.captures_iter(content) {
        if let Some(s) = m.get(1) {
            hooks.insert(s.as_str().to_string());
        }
    }
    let mut routes: BTreeSet<String> = BTreeSet::new();
    for m in route_re.captures_iter(content) {
        if let Some(s) = m.get(1) {
            routes.insert(s.as_str().to_string());
        }
    }
    let mut api_calls: BTreeSet<String> = BTreeSet::new();
    for m in api_re.captures_iter(content) {
        if let Some(s) = m.get(1) {
            api_calls.insert(s.as_str().to_string());
        }
    }

    let i18n_files = find_i18n_files(root_dir, source_file);
    let i18n_dict = resolve_i18n_keys(&all_i18n_keys, &i18n_files);
    let tests = find_related_tests(root_dir, source_file);

    let mut lines: Vec<String> = vec![rel_path.to_string()];
    if !resolved_components.is_empty() {
        let items: Vec<String> = resolved_components
            .iter()
            .map(|(path, syms, props)| match props {
                Some(p) => format!("{syms} -> {path} ({p})"),
                None => format!("{syms} -> {path}"),
            })
            .collect();
        lines.push(format!("components: {}", items.join("; ")));
    }
    if !resolved_styles.is_empty() {
        lines.push(format!("styles: {}", resolved_styles.join(", ")));
    }
    if !hooks.is_empty() {
        lines.push(format!("hooks: {}", hooks.into_iter().collect::<Vec<_>>().join(", ")));
    }
    if !api_calls.is_empty() {
        lines.push(format!("api: {}", api_calls.into_iter().collect::<Vec<_>>().join(", ")));
    }
    if !routes.is_empty() {
        lines.push(format!("routes: {}", routes.into_iter().collect::<Vec<_>>().join(", ")));
    }
    if !resolved_data.is_empty() {
        let services: Vec<String> = resolved_data
            .iter()
            .map(|(path, syms)| format!("{syms} -> {path}"))
            .collect();
        lines.push(format!("services: {}", services.join("; ")));
    }
    if !all_i18n_keys.is_empty() {
        let copy: Vec<String> = all_i18n_keys
            .iter()
            .map(|k| format!("{}={}", k, i18n_dict.get(k).map(|v| v.as_str()).unwrap_or("[missing]")))
            .collect();
        lines.push(format!("copy: {}", copy.join("; ")));
    }
    if !tests.is_empty() {
        lines.push(format!("tests: {}", tests.join(", ")));
    }
    if !external_deps.is_empty() {
        let mods: Vec<String> = external_deps.iter().take(6).cloned().collect();
        lines.push(format!("external: {}", mods.join(", ")));
    }

    lines.join("\n")
}

fn collect_keys(text: &str, t_re: &Regex, k_re: &Regex, out: &mut BTreeSet<String>) {
    for re in [t_re, k_re] {
        for m in re.captures_iter(text) {
            if let Some(s) = m.get(1) {
                out.insert(s.as_str().to_string());
            }
        }
    }
}

fn has_upper_identifier(clause: &str) -> bool {
    let id_re = Regex::new(r"\b[A-Za-z_$][\w$]*\b").unwrap();
    let found = id_re.find_iter(clause).any(|m| {
        m.as_str()
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
    });
    found
}

// ─── Module / import resolution ──────────────────────────────────────────────

/// Port of `resolve_module_path` — resolve relative (`./`, `../`), alias (`@/`)
/// or bare imports to a file path.
pub fn resolve_module_path(root_dir: &Path, source_file: &Path, module_name: &str) -> Option<PathBuf> {
    if module_name.is_empty() {
        return None;
    }
    let source_dir = source_file.parent()?;
    let mut candidates: Vec<PathBuf> = Vec::new();

    if module_name.starts_with("./") || module_name.starts_with("../") {
        candidates.push(source_dir.join(module_name));
    } else if let Some(rest) = module_name.strip_prefix("@/") {
        let mut cur = source_dir.to_path_buf();
        let mut src_dir: Option<PathBuf> = None;
        while cur != root_dir && cur.parent().is_some() {
            if cur.file_name().and_then(|n| n.to_str()) == Some("src") {
                src_dir = Some(cur.clone());
                break;
            }
            match cur.parent() {
                Some(p) => cur = p.to_path_buf(),
                None => break,
            }
        }
        if let Some(sd) = src_dir {
            candidates.push(sd.join(rest));
        } else {
            candidates.push(root_dir.join("src").join(rest));
        }
    } else if !module_name.starts_with('.') && !module_name.starts_with('/') {
        candidates.push(root_dir.join("src").join(module_name));
        candidates.push(root_dir.join(module_name));
    }

    for cand in candidates {
        let base = normalize_lexically(&cand);
        if base.is_file() {
            return Some(base);
        }
        for ext in [".tsx", ".ts", ".jsx", ".js", ".css", ".scss", ".less", ".json", ".properties", ".py", ".go", ".rs", ".java"] {
            if let Some(c) = with_suffix_safe(&base, ext) {
                if c.is_file() {
                    return Some(c);
                }
            }
        }
        if base.is_dir() {
            for idx in ["index.tsx", "index.ts", "index.jsx", "index.js", "__init__.py"] {
                let cand = base.join(idx);
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }
    None
}

/// Lexically normalize a path (remove `.`, resolve `..`) without touching the
/// filesystem — equivalent to Python's `Path.resolve()` for these candidates.
fn normalize_lexically(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn with_suffix_safe(base: &Path, ext: &str) -> Option<PathBuf> {
    if base.file_name().is_none() {
        return None;
    }
    Some(base.with_extension(ext.trim_start_matches('.')))
}

/// Port of `extract_props_interface` — interface/type `*Props` snippet.
pub fn extract_props_interface(file_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(file_path).ok()?;
    let re = Regex::new(r"((?:export\s+)?(?:interface|type)\s+\w*Props\b[^{=]*[\{=][^}]+})").ok()?;
    let m = re.captures(&content)?;
    let snippet = m.get(1)?.as_str().trim();
    let joined: Vec<&str> = snippet
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(4)
        .collect();
    Some(joined.join(" "))
}

// ─── i18n ────────────────────────────────────────────────────────────────────

/// Port of `find_i18n_files` — candidate translation files, English first.
pub fn find_i18n_files(root_dir: &Path, source_file: &Path) -> Vec<PathBuf> {
    let mut i18n_files: Vec<PathBuf> = Vec::new();
    let mut checked: HashSet<PathBuf> = HashSet::new();
    let mut cur = match source_file.parent() {
        Some(p) => p.to_path_buf(),
        None => return i18n_files,
    };

    while cur.parent().is_some() {
        let candidates = [
            cur.join("public").join("assets"),
            cur.join("public").join("i18n"),
            cur.join("public").join("locales"),
            cur.join("assets"),
            cur.join("src").join("i18n"),
            cur.join("src").join("locales"),
            cur.join("locales"),
            cur.join("i18n"),
        ];
        for c in candidates {
            if c.is_dir() && checked.insert(c.clone()) {
                if let Ok(entries) = std::fs::read_dir(&c) {
                    for e in entries.flatten() {
                        let p = e.path();
                        let ext = ext_lower(&p);
                        if p.is_file() && (ext == ".properties" || ext == ".json") {
                            i18n_files.push(p);
                        }
                    }
                }
            }
        }
        if cur == root_dir || cur.parent() == Some(root_dir) {
            break;
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => break,
        }
    }

    i18n_files.sort_by_key(|p| {
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        if matches!(
            name.as_str(),
            "i18n.properties" | "messages.properties" | "en.json" | "i18n_en.properties"
        ) {
            (0u8, name)
        } else if name.contains("en") {
            (1u8, name)
        } else {
            (2u8, name)
        }
    });
    i18n_files
}

/// Port of `resolve_i18n_keys` — resolve key strings from `.properties`/`.json`.
pub fn resolve_i18n_keys(
    keys: &BTreeSet<String>,
    i18n_files: &[PathBuf],
) -> BTreeMap<String, String> {
    let mut resolved: BTreeMap<String, String> = BTreeMap::new();
    if keys.is_empty() {
        return resolved;
    }

    for f in i18n_files {
        let ext = ext_lower(f);
        if ext == ".properties" {
            let Ok(txt) = std::fs::read_to_string(f) else { continue };
            for k in keys {
                if resolved.contains_key(k) {
                    continue;
                }
                let re = Regex::new(&format!(r"(?m)^{}\s*=\s*(.+)$", regex::escape(k)));
                let Ok(re) = re else { continue };
                if let Some(cap) = re.captures(&txt) {
                    if let Some(v) = cap.get(1) {
                        resolved.insert(k.clone(), v.as_str().trim().to_string());
                    }
                }
            }
        } else if ext == ".json" {
            let Ok(txt) = std::fs::read_to_string(f) else { continue };
            let Ok(data) = serde_json::from_str::<Value>(&txt) else { continue };
            search_json_keys(&data, keys, &mut resolved);
        }
    }
    resolved
}

fn search_json_keys(d: &Value, keys: &BTreeSet<String>, resolved: &mut BTreeMap<String, String>) {
    let Some(obj) = d.as_object() else { return };
    for (jk, jv) in obj {
        if keys.contains(jk) && !resolved.contains_key(jk) {
            match jv {
                Value::String(s) => {
                    resolved.insert(jk.clone(), s.clone());
                }
                Value::Number(n) => {
                    resolved.insert(jk.clone(), n.to_string());
                }
                // Python stores whatever JSON value it found and later prints
                // `str(value)`, so a boolean resolves as "True"/"False" rather
                // than showing as [missing].
                Value::Bool(b) => {
                    resolved.insert(jk.clone(), if *b { "True" } else { "False" }.to_string());
                }
                _ => {}
            }
        }
        search_json_keys(jv, keys, resolved);
    }
}

// ─── Related tests ───────────────────────────────────────────────────────────

/// Same 15 entries as Python's `valid_suffixes`
/// (main/cli/commands/context.py) — Rust used to carry a 16th, `.cy.js`.
const TEST_SUFFIXES: [&str; 15] = [
    ".cy.tsx", ".cy.ts", ".test.tsx", ".test.ts", ".spec.tsx", ".spec.ts", ".pom.ts",
    "Test.java", "Tests.java", "TestCase.java", "_test.go", "_test.py", "test_*.py",
    "Test.cs", "Tests.cs",
];

/// Port of `find_related_tests` — sibling/mirrored test files (max 5).
pub fn find_related_tests(root_dir: &Path, source_file: &Path) -> Vec<String> {
    let mut tests: Vec<String> = Vec::new();
    let stem = stem_of(source_file);
    let s_str = source_file.display().to_string();

    // 1. Mirrored test dirs for Java (src/main/... -> src/test/...).
    if s_str.contains("src/main/") {
        for suffix in ["Test", "Tests", "TestCase"] {
            let replaced = s_str
                .replace("src/main/", "src/test/")
                .replace(&format!("{stem}.java"), &format!("{stem}{suffix}.java"));
            let mirror = PathBuf::from(&replaced);
            if mirror.is_file() {
                tests.push(rel_to(root_dir, &mirror).unwrap_or_else(|| replaced));
            }
        }
    }

    // 2. Search sibling and parent directories.
    let search_dirs = [
        source_file.parent().map(|p| p.to_path_buf()),
        source_file.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()),
    ];
    'outer: for sd in search_dirs.into_iter().flatten() {
        if !sd.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&sd)
            .into_iter()
            .filter_entry(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| !matches!(n, "node_modules" | ".git" | "venv" | "dist" | "build" | "target"))
                    .unwrap_or(true)
            })
            .flatten()
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let fname = entry.file_name().to_str().unwrap_or("");
            if fname.starts_with(&stem)
                && TEST_SUFFIXES.iter().any(|s| fname.ends_with(s))
            {
                let p = entry.path();
                if let Some(r) = rel_to(root_dir, p) {
                    tests.push(r);
                }
            }
        }
        // Python finishes walking one directory and only then stops, so the
        // five files it reports are the sorted-first five of that directory —
        // stopping mid-walk picks a different set.
        if tests.len() >= 5 {
            break 'outer;
        }
    }

    let mut uniq: Vec<String> = tests.into_iter().collect::<HashSet<_>>().into_iter().collect();
    uniq.sort();
    uniq.truncate(5);
    uniq
}

// ─── Target location ─────────────────────────────────────────────────────────

/// Port of `locate_target_file` (steps 1, 1b, 2 and 2b).
///
/// The Python's step 3 (semantic search index) is not ported; see module docs.
pub fn locate_target_file(target: &str, root_dir: &Path) -> Option<PathBuf> {
    let target_clean = target.trim().trim_matches(['\'', '"']);
    if target_clean.is_empty() {
        return None;
    }

    // 1. Exact file or relative path.
    let direct_path = root_dir.join(target_clean);
    if direct_path.is_file() {
        return Some(direct_path);
    }
    let expanded = {
        // Python: `Path(target_clean).expanduser().resolve()`.
        let p = expand_tilde(target_clean);
        if p.is_absolute() {
            p
        } else {
            std::env::current_dir().map(|c| c.join(&p)).unwrap_or(p)
        }
    };
    if expanded.is_file() {
        return Some(expanded);
    }

    // 1b. Directory → common entry points.
    if direct_path.is_dir() || (expanded.is_dir() && expanded != root_dir) {
        let dir_cand = if direct_path.is_dir() {
            direct_path.clone()
        } else {
            expanded.clone()
        };
        let entry_patterns = [
            dir_cand.join("src").join("pages").join("Landing.tsx"),
            dir_cand.join("src").join("pages").join("Home.tsx"),
            dir_cand.join("src").join("App.tsx"),
            dir_cand.join("src").join("index.tsx"),
            dir_cand.join("src").join("main.tsx"),
            dir_cand.join("App.tsx"),
            dir_cand.join("index.tsx"),
            dir_cand.join("main.py"),
        ];
        for ep in entry_patterns {
            if ep.is_file() {
                return Some(ep);
            }
        }
        for pages_dir in [
            dir_cand.join("src").join("pages"),
            dir_cand.join("pages"),
            dir_cand.join("src"),
        ] {
            if pages_dir.is_dir() {
                let mut tsfx: Vec<PathBuf> = std::fs::read_dir(&pages_dir)
                    .map(|rd| {
                        rd.flatten()
                            .map(|e| e.path())
                            .filter(|p| {
                                p.file_name()
                                    .and_then(|n| n.to_str())
                                    .map(|n| n.ends_with(".tsx"))
                                    .unwrap_or(false)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                tsfx.sort();
                for pf in tsfx {
                    let lname = pf
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if ![".test.", ".spec.", ".cy.", ".pom."].iter().any(|t| lname.contains(t)) {
                        return Some(pf);
                    }
                }
            }
        }
    }

    let ignored = [
        ".git", "node_modules", "venv", ".venv", "__pycache__", "dist", "build",
        "coverage", ".pytest_cache", "data",
    ];

    // 2. Fast scan matching the target filename.
    let target_name = Path::new(target_clean)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mut candidates: Vec<PathBuf> = Vec::new();
    if !target_name.is_empty() {
        for entry in walk_project(root_dir, &ignored) {
            if let Some(fname) = entry.file_name().and_then(|n| n.to_str()) {
                let f_lower = fname.to_lowercase();
                if f_lower == target_name || f_lower.starts_with(&format!("{target_name}.")) {
                    candidates.push(entry);
                    if candidates.len() >= 5 {
                        break;
                    }
                }
            }
        }
    }

    if !candidates.is_empty() {
        let non_test: Vec<&PathBuf> = candidates
            .iter()
            .filter(|c| {
                let lname = c
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                ![".test.", ".spec.", ".cy.", ".pom."].iter().any(|t| lname.contains(t))
            })
            .collect();
        if let Some(first) = non_test.first() {
            return Some((*first).clone());
        }
        return Some(candidates[0].clone());
    }

    // 2b. Fast local query keyword matching.
    let q_words: Vec<String> = split_nonword(&target_clean.to_lowercase())
        .into_iter()
        .filter(|w| w.chars().count() >= 3)
        .filter(|w| !matches!(w.as_str(), "page" | "pages" | "app" | "view" | "component" | "screen"))
        .collect();

    if !q_words.is_empty() {
        let mut keyword_cands: Vec<(i32, PathBuf)> = Vec::new();
        for entry in walk_project(root_dir, &ignored) {
            let ext = ext_lower(&entry);
            if !matches!(ext.as_str(), ".tsx" | ".jsx" | ".ts" | ".js" | ".py" | ".java" | ".go") {
                continue;
            }
            let fp_lower = entry.display().to_string().to_lowercase();
            let mut score = 0i32;
            if [".pom.", ".cy.", ".test.", ".spec.", "/cypress/", "/tests/", "/test/"]
                .iter()
                .any(|t| fp_lower.contains(t))
            {
                score -= 100;
            }
            if ext == ".tsx" || ext == ".jsx" {
                score += 30;
            }
            if ["/pages/", "/views/", "/screens/"].iter().any(|t| fp_lower.contains(t)) {
                score += 30;
            }
            let stem = stem_of(&entry).to_lowercase();
            for qw in &q_words {
                if stem == *qw {
                    score += 100;
                } else if stem.starts_with(qw) || stem.contains(qw.as_str()) {
                    score += 60;
                } else if fp_lower.contains(qw.as_str()) {
                    score += 20;
                }
            }
            if fp_lower.contains("customer")
                && ["consumer", "customer"].iter().any(|w| target_clean.to_lowercase().contains(w))
            {
                score += 40;
            }
            if fp_lower.contains("admin") && target_clean.to_lowercase().contains("admin") {
                score += 40;
            }
            if score >= 50 {
                keyword_cands.push((score, entry));
            }
        }
        if !keyword_cands.is_empty() {
            keyword_cands.sort_by(|a, b| b.0.cmp(&a.0));
            return Some(keyword_cands[0].1.clone());
        }
    }

    None
}

/// `Path.expanduser()` for the leading `~` / `~/` form.
fn expand_tilde(target: &str) -> PathBuf {
    if target == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    } else if let Some(rest) = target.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(target)
}

/// Walk a project directory, skipping ignored and dot-directories.
fn walk_project(root_dir: &Path, ignored: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root_dir)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_str().unwrap_or("");
            if e.depth() == 0 {
                return true;
            }
            if e.file_type().is_dir() {
                !ignored.contains(&name) && !name.starts_with('.')
            } else {
                true
            }
        })
        .flatten()
    {
        if entry.file_type().is_file() {
            files.push(entry.path().to_path_buf());
        }
    }
    files
}

fn split_nonword(s: &str) -> Vec<String> {
    let re = Regex::new(r"\W+").unwrap();
    re.split(s)
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

// ─── Shared path helpers ─────────────────────────────────────────────────────

fn ext_lower(p: &Path) -> String {
    match p.extension().and_then(|e| e.to_str()) {
        Some(e) => format!(".{}", e.to_lowercase()),
        None => String::new(),
    }
}

fn stem_of(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

fn path_lower(p: &Path) -> String {
    p.display().to_string().to_lowercase()
}

fn rel_to(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|r| r.display().to_string())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "imc-ctx-test-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn is_backend_file_classification() {
        assert!(is_backend_file(Path::new("src/main/java/A.java")));
        assert!(is_backend_file(Path::new("api/handler.py")));
        assert!(!is_backend_file(Path::new("src/components/Card.tsx")));
        assert!(is_backend_file(Path::new("src/service/user.ts")));
        assert!(!is_backend_file(Path::new("src/components/ui.tsx")));
    }

    #[test]
    fn resolve_module_relative_and_bare() {
        let root = tmp_dir("mod");
        write(&root, "src/api.ts", "export const x=1;");
        write(&root, "src/pages/Home.tsx", "export default function Home(){return null}");
        let home = root.join("src/pages/Home.tsx");
        // Bare import resolves under src/.
        let r = resolve_module_path(&root, &home, "api").unwrap();
        assert_eq!(r, root.join("src/api.ts"));
        // Relative import (parent) resolves to the normalized absolute path.
        let r2 = resolve_module_path(&root, &home, "../api").unwrap();
        assert_eq!(r2, root.join("src/api.ts"));
    }

    #[test]
    fn resolve_module_alias_at_src() {
        let root = tmp_dir("alias");
        write(&root, "src/utils/date.ts", "export const d=1;");
        let home = root.join("src/pages/Home.tsx");
        let r = resolve_module_path(&root, &home, "@/utils/date").unwrap();
        assert_eq!(r, root.join("src/utils/date.ts"));
    }

    #[test]
    fn extract_props_interface_from_tsx() {
        let root = tmp_dir("props");
        let f = write(
            &root,
            "src/C.tsx",
            "export interface FooProps {\n  a: string;\n  b: number;\n  c: boolean;\n}\n",
        );
        let s = extract_props_interface(&f).unwrap();
        assert!(s.contains("a: string"), "{s}");
        assert!(s.starts_with("export interface FooProps"), "{s}");
    }

    #[test]
    fn extract_props_interface_none_when_absent() {
        let root = tmp_dir("props2");
        let f = write(&root, "src/D.tsx", "export const a = 1;\n");
        assert!(extract_props_interface(&f).is_none());
    }

    #[test]
    fn resolve_i18n_keys_from_properties_and_json() {
        let root = tmp_dir("i18n");
        let f1 = write(&root, "src/i18n/en.properties", "greeting=Hello\nfarewell=Bye\n");
        let f2 = write(&root, "src/i18n/en.json", "{\"title\":\"Title\"}");
        let keys: BTreeSet<String> = ["greeting", "farewell", "title", "missing"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let resolved = resolve_i18n_keys(&keys, &[f1, f2]);
        assert_eq!(resolved.get("greeting").map(|s| s.as_str()), Some("Hello"));
        assert_eq!(resolved.get("farewell").map(|s| s.as_str()), Some("Bye"));
        assert_eq!(resolved.get("title").map(|s| s.as_str()), Some("Title"));
        assert!(!resolved.contains_key("missing"));
    }

    #[test]
    fn resolve_i18n_keys_resolves_json_booleans() {
        // Python stores the JSON value and prints str(value), so a boolean key
        // resolves as "True"/"False" instead of showing as [missing].
        let root = tmp_dir("i18n-bool");
        let f = write(&root, "src/i18n/en.json", "{\"flag\":true,\"off\":false}");
        let keys: BTreeSet<String> = ["flag", "off"].iter().map(|s| s.to_string()).collect();
        let resolved = resolve_i18n_keys(&keys, &[f]);
        assert_eq!(resolved.get("flag").map(|s| s.as_str()), Some("True"));
        assert_eq!(resolved.get("off").map(|s| s.as_str()), Some("False"));
    }

    #[test]
    fn resolve_i18n_keys_follows_document_order() {
        // Python walks the JSON in document order and keeps the first hit, so a
        // top-level key wins over a later nested one. Rust needs serde_json's
        // `preserve_order` for this: a sorted map visits "outer" first and the
        // nested value shadows the top-level key.
        let root = tmp_dir("i18n-order");
        let f = write(
            &root,
            "src/i18n/en.json",
            r#"{"title":"Outer","outer":{"title":"Inner"}}"#,
        );
        let keys: BTreeSet<String> = ["title"].iter().map(|s| s.to_string()).collect();
        let resolved = resolve_i18n_keys(&keys, &[f]);
        assert_eq!(resolved.get("title").map(|s| s.as_str()), Some("Outer"));
    }

    #[test]
    fn expand_tilde_uses_home() {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", "/tmp/imc-home-probe");
        assert_eq!(expand_tilde("~/src/a.py"), PathBuf::from("/tmp/imc-home-probe/src/a.py"));
        assert_eq!(expand_tilde("~"), PathBuf::from("/tmp/imc-home-probe"));
        assert_eq!(expand_tilde("src/a.py"), PathBuf::from("src/a.py"));
        assert_eq!(expand_tilde("/abs/a.py"), PathBuf::from("/abs/a.py"));
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn find_i18n_files_prefers_english() {
        let root = tmp_dir("i18nloc");
        write(&root, "src/i18n/zz.json", "{}");
        write(&root, "src/i18n/en.json", "{}");
        let src = write(&root, "src/pages/Home.tsx", "export default ()=>null");
        let files = find_i18n_files(&root, &src);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names[0], "en.json", "{names:?}");
    }

    #[test]
    fn find_related_tests_detects_sibling_test() {
        let root = tmp_dir("tests");
        let src = write(&root, "src/components/Button.tsx", "export const Button=()=>null");
        write(&root, "src/components/Button.test.tsx", "// test");
        let t = find_related_tests(&root, &src);
        assert_eq!(t, vec!["src/components/Button.test.tsx".to_string()]);
    }

    #[test]
    fn find_related_tests_empty_when_none() {
        let root = tmp_dir("tests2");
        let src = write(&root, "src/components/Gauge.tsx", "export const G=()=>null");
        assert!(find_related_tests(&root, &src).is_empty());
    }

    #[test]
    fn locate_target_exact_file() {
        let root = tmp_dir("loc");
        let f = write(&root, "src/App.tsx", "export default ()=>null");
        let r = locate_target_file("src/App.tsx", &root).unwrap();
        assert_eq!(r, f);
    }

    #[test]
    fn locate_target_by_filename_scan() {
        let root = tmp_dir("loc2");
        let f = write(&root, "deep/nested/Widget.tsx", "export const W=()=>null");
        let r = locate_target_file("Widget", &root).unwrap();
        assert_eq!(r, f);
    }

    #[test]
    fn locate_target_missing_returns_none() {
        let root = tmp_dir("loc3");
        assert!(locate_target_file("NoSuchThing", &root).is_none());
    }

    #[test]
    fn locate_target_keyword_scoring() {
        let root = tmp_dir("loc4");
        let f = write(&root, "src/pages/CustomerPage.tsx", "export const C=()=>null");
        let r = locate_target_file("customer page", &root).unwrap();
        assert_eq!(r, f);
    }

    #[test]
    fn backend_dossier_python() {
        let root = tmp_dir("be-py");
        let f = write(
            &root,
            "api/users.py",
            "class UserService:\n    def __init__(self, repo, logger):\n        self.repo = repo\n\n    def get_user(self, uid):\n        return self.repo.get(uid)\n\n    @app.get('/users/{uid}')\n    def show(self, uid):\n        pass\n",
        );
        let content = std::fs::read_to_string(&f).unwrap();
        let d = build_backend_dossier(&f, "api/users.py", &content, &root, None);
        assert!(d.contains("declaration: class UserService"), "{d}");
        assert!(d.contains("dependencies: repo, logger"), "{d}");
        assert!(d.contains("`get_user(self, uid)`"), "{d}");
        assert!(d.contains("GET /users/{uid}"), "{d}");
    }

    #[test]
    fn backend_dossier_java() {
        let root = tmp_dir("be-java");
        let f = write(
            &root,
            "src/main/java/Ctrl.java",
            "package a;\nimport x.Y;\n@Service\npublic class Ctrl {\n    private final Repo repo;\n    @RequestMapping(\"/api\")\n    @GetMapping(\"/hello\")\n    public String hello(String n) { return \"hi\"; }\n}\n",
        );
        let content = std::fs::read_to_string(&f).unwrap();
        let d = build_backend_dossier(&f, "Ctrl.java", &content, &root, None);
        // The class-decl regex includes preceding annotations (as in Python).
        assert!(d.contains("declaration: @Service public class Ctrl"), "{d}");
        assert!(d.contains("role: @Service"), "{d}");
        assert!(d.contains("dependencies: Repo repo"), "{d}");
        assert!(d.contains("GET /api/hello"), "{d}");
        assert!(d.contains("`hello(String n)`"), "{d}");
    }

    #[test]
    fn ui_dossier_components_styles_hooks() {
        let root = tmp_dir("ui");
        write(&root, "src/components/Card.tsx", "export interface CardProps { title: string; }\nexport const Card=()=>null");
        write(&root, "src/styles/main.css", "body{}");
        let f = write(
            &root,
            "src/pages/Home.tsx",
            "import React from 'react';\nimport { Card } from '../components/Card';\nimport '../styles/main.css';\nimport { Card as C2 } from '@/components/Card';\nimport axios from 'axios';\nexport default function Home(){ const h = useState(); return <div>{t('title')}</div>; }\n",
        );
        let content = std::fs::read_to_string(&f).unwrap();
        let d = build_ui_dossier(&f, "src/pages/Home.tsx", &content, &root);
        assert!(d.contains("components:"), "{d}");
        assert!(d.contains("styles: src/styles/main.css"), "{d}");
        // Both 'react' and 'axios' are external (non-relative) deps.
        assert!(d.contains("external: react, axios"), "{d}");
        assert!(d.contains("hooks: useState"), "{d}");
        assert!(d.contains("copy: title=[missing]"), "{d}");
    }

    #[test]
    fn get_feature_context_backend_routes_to_backend() {
        let root = tmp_dir("gfc-be");
        write(&root, "api/users.py", "class S:\n    def m(self):\n        pass\n");
        let d = get_feature_context("api/users.py", &root, None);
        assert!(d.starts_with("api/users.py"), "{d}");
        assert!(d.contains("declaration: class S"), "{d}");
    }

    #[test]
    fn get_feature_context_missing_target_error() {
        let root = tmp_dir("gfc-missing");
        let d = get_feature_context("Nope/NotThere.tsx", &root, None);
        assert!(d.starts_with("Error: Could not locate target file"), "{d}");
    }
}
