use anyhow::Result;
use walkdir::WalkDir;

use crate::hash::hash_file;

pub const DEFAULT_IGNORES: &[&str] = &[
    ".git/",
    ".hg/",
    ".svn/",
    "node_modules/",
    "__pycache__/",
    ".venv/",
    "venv/",
    "target/",
    "dist/",
    "build/",
    ".mypy_cache/",
    ".pytest_cache/",
    ".ruff_cache/",
    ".tox/",
    ".next/",
    "coverage/",
    ".idea/",
    ".vscode/",
    "*.egg-info/",
];

const EXCLUDED_FILENAMES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "composer.lock",
    "cargo.lock",
    "poetry.lock",
    "gemfile.lock",
    "bun.lockb",
    "shrinkwrap.yaml",
];

const INDEXABLE_RESOURCE_EXTS: &[&str] = &[
    "properties", "json", "yaml", "yml", "toml", "ini", "md", "txt", "sql", "graphql", "gql",
    "proto", "cfg", "conf",
];

const CODE_EXTS: &[&str] = &[
    "py", "js", "jsx", "ts", "tsx", "mjs", "cjs", "go", "java", "kt", "kts", "c", "h", "cc",
    "cpp", "hpp", "rs", "rb", "cs", "swift", "php", "scala", "sh", "bash", "zsh", "lua", "vue",
    "svelte", "html", "css", "scss",
];

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: std::path::PathBuf,
    /// Project-relative, forward-slash separated.
    pub rel: String,
    pub hash: String,
    pub size: u64,
}

pub fn is_indexable_file(path: &std::path::Path, max_bytes: u64) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if EXCLUDED_FILENAMES.contains(&name.as_str()) {
        return false;
    }
    // Bundled build output: hashed filenames (index-CuOPrw-O.js) and any
    // `assets/` directory — these are 900 KB+ of minified JS that drown out real code.
    if let Some(parent) = path.parent() {
        if parent.file_name().map(|p| p == "assets").unwrap_or(false) {
            return false;
        }
    }
    if is_hashed_bundle_name(&name) {
        return false;
    }
    if name.ends_with(".lock")
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".min.json")
        || name.ends_with(".map")
        || name.ends_with(".log")
        || name.ends_with(".diff")
        || name.ends_with(".patch")
    {
        return false;
    }
    if matches!(name.as_str(), ".env.example" | ".env.template" | ".env.sample") {
        return true;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if INDEXABLE_RESOURCE_EXTS.contains(&ext.as_str()) || CODE_EXTS.contains(&ext.as_str()) {
        return std::fs::metadata(path).map(|m| m.len() <= max_bytes).unwrap_or(false);
    }
    false
}

/// `index-CuOPrw-O.js`, `logo-BmnHdZ4s.svg`, `main-Dk3x9Pq2.css` — build output.
pub fn is_hashed_bundle_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    if !matches!(ext, "js" | "css" | "mjs" | "map") {
        return false;
    }
    // at least one '-'-separated segment of >=8 base64url chars, no lowercase-only words
    stem.rsplit('-').any(|seg| {
        seg.len() >= 8
            && seg.chars().any(|c| c.is_ascii_digit())
            && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    })
}

pub fn language_of(rel: &str) -> Option<&'static str> {
    let ext = rel.rsplit('.').next()?.to_lowercase();
    Some(match ext.as_str() {
        "py" | "pyi" => "python",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "tsx" => "tsx",
        "go" => "go",
        "rs" => "rust",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "c" | "h" => "c",
        "cc" | "cpp" | "hpp" => "cpp",
        "rb" => "ruby",
        "cs" => "csharp",
        "swift" => "swift",
        "php" => "php",
        "sh" | "bash" | "zsh" => "shell",
        "lua" => "lua",
        "vue" => "vue",
        "svelte" => "svelte",
        "html" => "html",
        "css" | "scss" => "css",
        _ => return None,
    })
}

/// Cheap binary sniff: NUL byte in the first 1 KiB.
pub fn looks_binary(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return true;
    };
    let mut buf = [0u8; 1024];
    let Ok(n) = f.read(&mut buf) else {
        return true;
    };
    buf[..n].contains(&0)
}

pub fn read_text(path: &std::path::Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

/// Walk the project and return `(rel_path, hash)` for every indexable file.
pub fn build_manifest(root: &std::path::Path, max_bytes: u64, extra: &[String]) -> Result<Vec<FileEntry>> {
    let mut ignores: Vec<String> = DEFAULT_IGNORES.iter().map(|s| (*s).to_string()).collect();
    ignores.extend(extra.iter().cloned());

    let mut out = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        if e.depth() == 0 {
            return true;
        }
        if e.file_type().is_dir() {
            let with_slash = format!("{name}/");
            return !ignores.iter().any(|ig| ig == &with_slash || ig == &name);
        }
        true
    }) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !is_indexable_file(path, max_bytes) || looks_binary(path) {
            continue;
        }
        let Ok(hash) = hash_file(path) else { continue };
        let Ok(rel) = path.strip_prefix(root) else { continue };
        out.push(FileEntry {
            path: path.to_path_buf(),
            rel: rel.to_string_lossy().replace('\\', "/"),
            hash,
            size: entry.metadata().map(|m| m.len()).unwrap_or(0),
        });
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}