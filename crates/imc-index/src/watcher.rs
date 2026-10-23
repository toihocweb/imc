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

            // Track currently watched git directories and map them to project paths:
            // git_dir -> project_root
            let mut watched_projects: HashMap<PathBuf, PathBuf> = HashMap::new();

            fn sync_watches(
                cfg: &Config,
                watcher: &mut RecommendedWatcher,
                watched: &mut HashMap<PathBuf, PathBuf>,
            ) {
                let current_registry = load_registry(cfg);
                let current_roots: HashSet<PathBuf> = current_registry
                    .into_iter()
                    .map(|p| PathBuf::from(p.path))
                    .filter(|p| p.exists())
                    .collect();

                // Unwatch projects that were removed
                watched.retain(|git_dir, proj_root| {
                    if !current_roots.contains(proj_root) || !proj_root.exists() {
                        let _ = watcher.unwatch(git_dir);
                        let _ = watcher.unwatch(&git_dir.join("refs"));
                        false
                    } else {
                        true
                    }
                });

                // Add watches for new projects
                for root in current_roots {
                    if let Some(git_dir) = resolve_git_dir(&root) {
                        if !watched.contains_key(&git_dir) {
                            if watcher.watch(&git_dir, RecursiveMode::NonRecursive).is_ok() {
                                let refs_dir = git_dir.join("refs");
                                if refs_dir.is_dir() {
                                    let _ = watcher.watch(&refs_dir, RecursiveMode::Recursive);
                                }
                                watched.insert(git_dir, root);
                            }
                        }
                    }
                }
            }

            sync_watches(&cfg, &mut watcher, &mut watched_projects);

            while !stop_clone.load(Ordering::SeqCst) {
                // Blocks in the OS kernel until an inotify/fsevent arrives
                let first_path = match rx.recv() {
                    Ok(p) => p,
                    Err(_) => break,
                };

                if stop_clone.load(Ordering::SeqCst) {
                    break;
                }

                // Debounce: git commands write multiple ref files over 500ms
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

                // Check if registry changed (new project added)
                let reg_changed = touched.iter().any(|p| p.ends_with("projects.json"));
                if reg_changed {
                    sync_watches(&cfg, &mut watcher, &mut watched_projects);
                }

                // Check all watched projects for git branch/commit change
                for (_git_dir, root) in &watched_projects {
                    if root.exists() {
                        let _ = ensure_fresh_index(&cfg, root);
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
}
