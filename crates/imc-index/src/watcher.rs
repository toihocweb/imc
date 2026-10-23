//! Event-driven filesystem watcher for git branch and commit changes.
//!
//! Watches repository `.git` directories and triggers incremental re-indexing
//! only when the actual branch or commit hash changes on disk.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};

use imc_core::config::{load_registry, registry_path, upsert_project, Config, ProjectMeta};
use imc_core::hash::md5_hex;
use imc_core::store::Store;

use crate::{run, IndexStats};

/// Query the current git commit hash and branch name if `root` is inside a git repository.
pub fn git_head_state(root: &Path) -> Option<(String, String)> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD", "--abbrev-ref", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let commit = lines.next()?.trim().to_string();
    let branch = lines.next()?.trim().to_string();
    if commit.is_empty() {
        return None;
    }
    Some((commit, branch))
}

/// Locate the real `.git` directory for a repository root, handling git worktrees and submodules.
pub fn resolve_git_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    if dot_git.is_file() {
        // Submodule or git worktree: contains "gitdir: <path>"
        if let Ok(content) = std::fs::read_to_string(&dot_git) {
            let line = content.lines().next()?.trim();
            if let Some(rest) = line.strip_prefix("gitdir:") {
                let gitdir_path = rest.trim();
                let candidate = root.join(gitdir_path);
                if let Ok(canon) = std::fs::canonicalize(&candidate) {
                    return Some(canon);
                }
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Check if the stored index's git state differs from the current git repository state.
pub fn is_git_stale(store: &Store, root: &Path) -> bool {
    if let Some((curr_commit, curr_branch)) = git_head_state(root) {
        let stored_commit = store.get_meta("git_commit").ok().flatten().unwrap_or_default();
        let stored_branch = store.get_meta("git_branch").ok().flatten().unwrap_or_default();
        if stored_commit.is_empty() {
            let _ = store.set_meta("git_commit", &curr_commit);
            let _ = store.set_meta("git_branch", &curr_branch);
            return false;
        }
        if stored_commit != curr_commit || stored_branch != curr_branch {
            return true;
        }
    }
    false
}

/// Record current git commit and branch into project metadata table.
pub fn record_git_state(store: &Store, root: &Path) -> Result<()> {
    if let Some((commit, branch)) = git_head_state(root) {
        store.set_meta("git_commit", &commit)?;
        store.set_meta("git_branch", &branch)?;
    }
    Ok(())
}

/// Check and incrementally update project index if git state has changed.
pub fn ensure_fresh_index(cfg: &Config, root: &Path) -> Result<Option<IndexStats>> {
    let db = cfg.project_db(root);
    if !db.exists() {
        return Ok(None);
    }
    let store = imc_core::store::open(&db)?;
    if is_git_stale(&store, root) {
        drop(store);
        eprintln!("[imc] Git branch or commit changed — updating index incrementally…");
        let stats = run(cfg, root, &[], false)?;
        eprintln!(
            "[imc] Index updated: +{} ~{} -{} files in {}ms",
            stats.new_files, stats.modified_files, stats.deleted_files, stats.elapsed_ms
        );
        let now_sec = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let meta = ProjectMeta {
            id: md5_hex(root.to_string_lossy().as_bytes())[..16].to_string(),
            name: root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "root".into()),
            path: root.to_string_lossy().to_string(),
            indexed_at: Some(now_sec),
            file_count: Some(stats.total_files as u64),
        };
        let _ = upsert_project(cfg, meta);
        return Ok(Some(stats));
    }
    Ok(None)
}

/// Check and incrementally update project index when working tree files (uncommitted edits) change.
pub fn ensure_fresh_working_tree(cfg: &Config, root: &Path) -> Result<Option<IndexStats>> {
    let db = cfg.project_db(root);
    if !db.exists() {
        return Ok(None);
    }
    let stats = run(cfg, root, &[], false)?;
    if stats.new_files > 0 || stats.modified_files > 0 || stats.deleted_files > 0 {
        eprintln!(
            "[imc] Working tree updated: +{} ~{} -{} files in {}ms",
            stats.new_files, stats.modified_files, stats.deleted_files, stats.elapsed_ms
        );
        let now_sec = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let meta = ProjectMeta {
            id: md5_hex(root.to_string_lossy().as_bytes())[..16].to_string(),
            name: root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "root".into()),
            path: root.to_string_lossy().to_string(),
            indexed_at: Some(now_sec),
            file_count: Some(stats.total_files as u64),
        };
        let _ = upsert_project(cfg, meta);
        return Ok(Some(stats));
    }
    Ok(None)
}

pub fn is_source_or_resource_file(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if is_ignored_path(&s) {
        return false;
    }
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_lower = ext.to_lowercase();
        imc_core::ingest::CODE_EXTS.contains(&ext_lower.as_str())
            || imc_core::ingest::INDEXABLE_RESOURCE_EXTS.contains(&ext_lower.as_str())
    } else {
        false
    }
}

pub fn is_ignored_path(s: &str) -> bool {
    s.contains("/.git/")
        || s.contains("/node_modules/")
        || s.contains("/target/")
        || s.contains("/dist/")
        || s.contains("/build/")
        || s.contains("/.next/")
        || s.contains("/__pycache__/")
        || s.contains("/.venv/")
        || s.contains("/venv/")
        || s.contains("/cypress/fixtures/")
        || s.contains("/playwright-report/")
        || s.contains("/test-results/")
}

pub fn is_ignored_dir_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        ".git"
            | ".hg"
            | ".svn"
            | "node_modules"
            | "__pycache__"
            | ".venv"
            | "venv"
            | "target"
            | "dist"
            | "build"
            | ".mypy_cache"
            | ".pytest_cache"
            | ".ruff_cache"
            | ".tox"
            | ".next"
            | "coverage"
            | ".idea"
            | ".vscode"
            | "cypress"
            | "playwright-report"
            | "test-results"
    )
}

/// Load and parse gitignore rules for a project root, including `.gitignore` and `.git/info/exclude`.
pub fn load_project_gitignore(root: &Path) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(root);
    let gitignore = root.join(".gitignore");
    if gitignore.exists() {
        let _ = builder.add(&gitignore);
    }
    let git_exclude = root.join(".git").join("info").join("exclude");
    if git_exclude.exists() {
        let _ = builder.add(&git_exclude);
    }
    builder.build().ok()
}

/// Checks if a path should be ignored, combining hardcoded safety list and project `.gitignore`.
pub fn is_path_ignored(
    root: &Path,
    path: &Path,
    is_dir: bool,
    gitignore: Option<&Gitignore>,
) -> bool {
    let s = path.to_string_lossy();
    if is_ignored_path(&s) {
        return true;
    }
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if is_dir && is_ignored_dir_name(name) {
            return true;
        }
    }
    if let Some(gi) = gitignore {
        if let Ok(rel) = path.strip_prefix(root) {
            if gi.matched_path_or_any_parents(rel, is_dir).is_ignore() {
                return true;
            }
        }
    }
    false
}

/// Handle to a running background git watcher thread.
pub struct GitWatcherHandle {
    stop: Arc<AtomicBool>,
    tx_stop: Sender<PathBuf>,
    thread: Option<JoinHandle<()>>,
}

impl GitWatcherHandle {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.tx_stop.send(PathBuf::new());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for GitWatcherHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Spawn an event-driven filesystem watcher thread that watches git refs/HEAD for registered projects.
///
/// Sleeps until notified by OS inotify/FSEvents, debounces events, and triggers incremental re-indexing
/// when branch or commit actually changes.
pub fn spawn_git_watcher(cfg: Config) -> Result<GitWatcherHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    let (tx, rx) = channel::<PathBuf>();
    let tx_stop = tx.clone();

    let thread = std::thread::Builder::new()
        .name("imc-git-watcher".to_string())
        .spawn(move || {
            let mut watcher: RecommendedWatcher = match RecommendedWatcher::new(
                move |res: Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        for path in event.paths {
                            let _ = tx.send(path);
                        }
                    }
                },
                NotifyConfig::default(),
            ) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("[imc watcher] could not initialize filesystem watcher: {e}");
                    return;
                }
            };

            // Watch registry file (or its parent directory) to catch newly registered projects
            let reg_path = registry_path(&cfg);
            if let Some(parent) = reg_path.parent() {
                let _ = std::fs::create_dir_all(parent);
                let _ = watcher.watch(parent, RecursiveMode::NonRecursive);
            }

            // Track currently watched directories -> project_root
            let mut watched_dirs: HashMap<PathBuf, PathBuf> = HashMap::new();
            // Track loaded gitignore per project_root
            let mut project_gitignores: HashMap<PathBuf, Option<Gitignore>> = HashMap::new();

            fn sync_watches(
                cfg: &Config,
                watcher: &mut RecommendedWatcher,
                watched: &mut HashMap<PathBuf, PathBuf>,
                gitignores: &mut HashMap<PathBuf, Option<Gitignore>>,
            ) {
                let current_registry = load_registry(cfg);
                let current_roots: HashSet<PathBuf> = current_registry
                    .into_iter()
                    .map(|p| PathBuf::from(p.path))
                    .filter(|p| p.exists())
                    .collect();

                // Unwatch directories of removed projects
                watched.retain(|dir, proj_root| {
                    if !current_roots.contains(proj_root) || !proj_root.exists() {
                        let _ = watcher.unwatch(dir);
                        false
                    } else {
                        true
                    }
                });
                gitignores.retain(|proj_root, _| current_roots.contains(proj_root));

                // Add watches for projects
                for root in current_roots {
                    let gi = load_project_gitignore(&root);

                    // 1. Watch root non-recursively (main.rs, Cargo.toml, etc.)
                    if !watched.contains_key(&root) {
                        if watcher.watch(&root, RecursiveMode::NonRecursive).is_ok() {
                            watched.insert(root.clone(), root.clone());
                        }
                    }

                    // 2. Watch non-ignored subdirectories recursively (src, crates, cmd, etc.)
                    if let Ok(entries) = std::fs::read_dir(&root) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_dir() {
                                if !is_path_ignored(&root, &path, true, gi.as_ref())
                                    && !watched.contains_key(&path)
                                {
                                    if watcher.watch(&path, RecursiveMode::Recursive).is_ok() {
                                        watched.insert(path, root.clone());
                                    }
                                }
                            }
                        }
                    }

                    // 3. Watch .git directories
                    if let Some(git_dir) = resolve_git_dir(&root) {
                        if !watched.contains_key(&git_dir) {
                            if watcher.watch(&git_dir, RecursiveMode::NonRecursive).is_ok() {
                                let refs_dir = git_dir.join("refs");
                                if refs_dir.is_dir() {
                                    let _ = watcher.watch(&refs_dir, RecursiveMode::Recursive);
                                }
                                watched.insert(git_dir, root.clone());
                            }
                        }
                    }

                    gitignores.insert(root, gi);
                }
            }

            sync_watches(&cfg, &mut watcher, &mut watched_dirs, &mut project_gitignores);

            while !stop_clone.load(Ordering::SeqCst) {
                // Blocks in the OS kernel until an inotify/fsevent arrives
                let first_path = match rx.recv() {
                    Ok(p) => p,
                    Err(_) => break,
                };

                if stop_clone.load(Ordering::SeqCst) {
                    break;
                }

                // Debounce: coalesce rapid editor saves and git ref writes over 500ms
                let debounce = Duration::from_millis(500);
                let start = Instant::now();
                let mut touched = vec![first_path];
                while start.elapsed() < debounce {
                    let remaining = debounce.saturating_sub(start.elapsed());
                    if let Ok(p) = rx.recv_timeout(remaining) {
                        touched.push(p);
                    }
                }

                if stop_clone.load(Ordering::SeqCst) {
                    break;
                }

                // Check if registry changed (new project added) or .gitignore updated
                let reg_changed = touched.iter().any(|p| p.ends_with("projects.json"));
                let gitignore_changed = touched.iter().any(|p| p.ends_with(".gitignore"));
                if reg_changed || gitignore_changed {
                    sync_watches(&cfg, &mut watcher, &mut watched_dirs, &mut project_gitignores);
                }

                // Classify touched paths: git state changes vs uncommitted working tree changes
                let mut git_check_roots: HashSet<PathBuf> = HashSet::new();
                let mut tree_check_roots: HashSet<PathBuf> = HashSet::new();

                for p in &touched {
                    let p_str = p.to_string_lossy();
                    if p_str.contains("/.git/") || p_str.ends_with("/.git") {
                        for (watched_dir, root) in &watched_dirs {
                            if p.starts_with(watched_dir) || p.starts_with(root) {
                                git_check_roots.insert(root.clone());
                            }
                        }
                    } else {
                        for (watched_dir, root) in &watched_dirs {
                            if p.starts_with(watched_dir) || p.starts_with(root) {
                                let gi_ref = project_gitignores.get(root).and_then(|g| g.as_ref());
                                if !is_path_ignored(root, p, p.is_dir(), gi_ref) {
                                    if is_source_or_resource_file(p) || p.is_dir() {
                                        tree_check_roots.insert(root.clone());
                                    }
                                }
                            }
                        }
                    }
                }

                for root in git_check_roots {
                    if root.exists() {
                        let _ = ensure_fresh_index(&cfg, &root);
                    }
                }

                for root in tree_check_roots {
                    if root.exists() {
                        let _ = ensure_fresh_working_tree(&cfg, &root);
                    }
                }
            }
        })
        .context("spawn imc-git-watcher thread")?;

    Ok(GitWatcherHandle {
        stop,
        tx_stop,
        thread: Some(thread),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_head_state_detects_git_repository() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        if let Some((commit, branch)) = git_head_state(repo_root) {
            assert!(!commit.is_empty());
            assert!(!branch.is_empty());
        }
    }

    #[test]
    fn non_git_directory_returns_none() {
        let temp = std::env::temp_dir().join("definitely_not_a_git_repo_99999");
        assert_eq!(git_head_state(&temp), None);
        assert_eq!(resolve_git_dir(&temp), None);
    }

    #[test]
    fn watcher_auto_reindexes_on_commit() {
        let td = std::env::temp_dir().join(format!("test_watcher_auto_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let repo = td.join("repo");
        let data = td.join("data");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&data).unwrap();

        let _ = std::process::Command::new("git").args(["init"]).current_dir(&repo).output().unwrap();
        let _ = std::process::Command::new("git").args(["config", "user.name", "T"]).current_dir(&repo).output().unwrap();
        let _ = std::process::Command::new("git").args(["config", "user.email", "t@t.com"]).current_dir(&repo).output().unwrap();

        std::fs::write(repo.join("hello.py"), "def foo():\n    return 42\n").unwrap();
        let _ = std::process::Command::new("git").args(["add", "hello.py"]).current_dir(&repo).output().unwrap();
        let _ = std::process::Command::new("git").args(["commit", "-m", "init"]).current_dir(&repo).output().unwrap();

        let mut cfg = Config::default();
        cfg.data_dir = data;

        let stats = run(&cfg, &repo, &[], false).unwrap();
        assert_eq!(stats.symbols, 1);

        let meta = ProjectMeta {
            id: md5_hex(repo.to_string_lossy().as_bytes())[..16].to_string(),
            name: "repo".to_string(),
            path: repo.to_string_lossy().to_string(),
            indexed_at: Some(1),
            file_count: Some(1),
        };
        upsert_project(&cfg, meta).unwrap();

        let mut watcher = spawn_git_watcher(cfg.clone()).unwrap();

        // Commit a new function on a new branch
        let _ = std::process::Command::new("git").args(["checkout", "-b", "feature"]).current_dir(&repo).output().unwrap();
        std::fs::write(repo.join("hello.py"), "def foo():\n    return 42\ndef bar():\n    return 100\n").unwrap();
        let _ = std::process::Command::new("git").args(["commit", "-am", "add bar"]).current_dir(&repo).output().unwrap();

        // Wait for debounce (500ms) + indexing
        std::thread::sleep(Duration::from_millis(1500));

        let db = cfg.project_db(&repo);
        let store = imc_core::store::open(&db).unwrap();
        assert_eq!(store.count_symbols(), 2);

        watcher.stop();
        let _ = std::fs::remove_dir_all(&td);
    }

    #[test]
    fn watcher_auto_reindexes_uncommitted_file_edit() {
        let td = std::env::temp_dir().join(format!("test_watcher_uncommitted_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let repo = td.join("repo");
        let src = repo.join("src");
        let data = td.join("data");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&data).unwrap();

        let _ = std::process::Command::new("git").args(["init"]).current_dir(&repo).output().unwrap();
        let _ = std::process::Command::new("git").args(["config", "user.name", "T"]).current_dir(&repo).output().unwrap();
        let _ = std::process::Command::new("git").args(["config", "user.email", "t@t.com"]).current_dir(&repo).output().unwrap();

        std::fs::write(src.join("auth.py"), "def login():\n    return True\n").unwrap();
        let _ = std::process::Command::new("git").args(["add", "src/auth.py"]).current_dir(&repo).output().unwrap();
        let _ = std::process::Command::new("git").args(["commit", "-m", "init"]).current_dir(&repo).output().unwrap();

        let mut cfg = Config::default();
        cfg.data_dir = data;

        let stats = run(&cfg, &repo, &[], false).unwrap();
        assert_eq!(stats.symbols, 1);

        let meta = ProjectMeta {
            id: md5_hex(repo.to_string_lossy().as_bytes())[..16].to_string(),
            name: "repo".to_string(),
            path: repo.to_string_lossy().to_string(),
            indexed_at: Some(1),
            file_count: Some(1),
        };
        upsert_project(&cfg, meta).unwrap();

        let mut watcher = spawn_git_watcher(cfg.clone()).unwrap();

        // Directly edit the file without git commit or git add (dirty working tree!)
        std::fs::write(src.join("auth.py"), "def login():\n    return True\ndef logout():\n    return False\n").unwrap();

        // Wait for debounce (500ms) + incremental indexing
        std::thread::sleep(Duration::from_millis(1500));

        let db = cfg.project_db(&repo);
        let store = imc_core::store::open(&db).unwrap();
        assert_eq!(store.count_symbols(), 2, "uncommitted file edit must be auto-indexed in SQLite");

        watcher.stop();
        let _ = std::fs::remove_dir_all(&td);
    }

    #[test]
    fn watcher_respects_custom_gitignore_rules() {
        let td = std::env::temp_dir().join(format!("test_watcher_gitignore_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let repo = td.join("repo");
        let src = repo.join("src");
        let custom_cache = repo.join("custom_cache");
        let data = td.join("data");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&custom_cache).unwrap();
        std::fs::create_dir_all(&data).unwrap();

        let _ = std::process::Command::new("git").args(["init"]).current_dir(&repo).output().unwrap();
        let _ = std::process::Command::new("git").args(["config", "user.name", "T"]).current_dir(&repo).output().unwrap();
        let _ = std::process::Command::new("git").args(["config", "user.email", "t@t.com"]).current_dir(&repo).output().unwrap();

        std::fs::write(repo.join(".gitignore"), "custom_cache/\n").unwrap();
        std::fs::write(src.join("main.py"), "def run():\n    pass\n").unwrap();
        std::fs::write(custom_cache.join("junk.py"), "def junk():\n    pass\n").unwrap();

        let mut cfg = Config::default();
        cfg.data_dir = data;

        let stats = run(&cfg, &repo, &[], false).unwrap();
        assert_eq!(stats.symbols, 1, "custom_cache/ must not be indexed initially");

        let meta = ProjectMeta {
            id: md5_hex(repo.to_string_lossy().as_bytes())[..16].to_string(),
            name: "repo".to_string(),
            path: repo.to_string_lossy().to_string(),
            indexed_at: Some(1),
            file_count: Some(1),
        };
        upsert_project(&cfg, meta).unwrap();

        let mut watcher = spawn_git_watcher(cfg.clone()).unwrap();

        // 1. Modifying gitignored folder must NOT trigger indexing
        std::fs::write(custom_cache.join("junk.py"), "def junk():\n    pass\ndef junk2():\n    pass\n").unwrap();
        std::thread::sleep(Duration::from_millis(1000));

        let db = cfg.project_db(&repo);
        let store = imc_core::store::open(&db).unwrap();
        assert_eq!(store.count_symbols(), 1, "ignored custom_cache file must be ignored by watcher");

        // 2. Modifying tracked file must trigger incremental indexing
        std::fs::write(src.join("main.py"), "def run():\n    pass\ndef stop():\n    pass\n").unwrap();
        std::thread::sleep(Duration::from_millis(1500));

        let store = imc_core::store::open(&db).unwrap();
        assert_eq!(store.count_symbols(), 2, "source file must be indexed");

        watcher.stop();
        let _ = std::fs::remove_dir_all(&td);
    }
}
