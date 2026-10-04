//! Filesystem notifications for registered repositories.
//!
//! `notify` delivers raw events on its own thread. We map each event to the
//! most specific registered repository, drop noise from `.git/objects` and
//! lock files, and forward the repository id to a debounce loop that
//! requests one coalesced refresh per repository (SPEC.md, Refresh strategy).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::models::{AppError, AppResult, ChangeOrigin};
use crate::workspaces::RepositoryService;

/// Debounce window for repository invalidation (docs/architecture.md, Refresh: ~500 ms).
pub const DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
struct WatchedRepo {
    id: String,
    root: PathBuf,
    git_dir: PathBuf,
}

pub struct RepositoryWatcher {
    watcher: Mutex<RecommendedWatcher>,
    repos: Arc<RwLock<Vec<WatchedRepo>>>,
}

impl RepositoryWatcher {
    /// Create the watcher. Repository ids with pending changes arrive on the returned receiver.
    pub fn new() -> AppResult<(Self, mpsc::UnboundedReceiver<String>)> {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let repos: Arc<RwLock<Vec<WatchedRepo>>> = Arc::new(RwLock::new(Vec::new()));
        let repos_for_cb = Arc::clone(&repos);
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let event = match res {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "filesystem watcher error");
                    return;
                }
            };
            let repos = repos_for_cb.read().expect("watched repos");
            for path in &event.paths {
                if let Some(id) = classify(&repos, path) {
                    let _ = tx.send(id);
                }
            }
        })
        .map_err(|e| {
            AppError::io("Could not start the filesystem watcher.").with_details(e.to_string())
        })?;
        Ok((
            Self {
                watcher: Mutex::new(watcher),
                repos,
            },
            rx,
        ))
    }

    pub fn watch(&self, id: &str, root: &Path, git_dir: &Path) -> AppResult<()> {
        let mut watcher = self.watcher.lock().expect("watcher");
        watcher.watch(root, RecursiveMode::Recursive).map_err(|e| {
            AppError::io(format!("Could not watch {}.", root.display())).with_details(e.to_string())
        })?;
        // A linked worktree keeps its metadata outside the working tree; watch that too.
        if !git_dir.starts_with(root) {
            if let Err(e) = watcher.watch(git_dir, RecursiveMode::Recursive) {
                tracing::warn!(path = %git_dir.display(), error = %e, "could not watch git dir");
            }
        }
        let mut repos = self.repos.write().expect("watched repos");
        repos.retain(|r| r.id != id);
        repos.push(WatchedRepo {
            id: id.to_string(),
            root: root.to_path_buf(),
            git_dir: git_dir.to_path_buf(),
        });
        // Longest root first so nested repositories win over their parents.
        repos.sort_by_key(|r| std::cmp::Reverse(r.root.as_os_str().len()));
        Ok(())
    }

    pub fn unwatch(&self, id: &str) {
        let removed = {
            let mut repos = self.repos.write().expect("watched repos");
            let pos = repos.iter().position(|r| r.id == id);
            pos.map(|p| repos.remove(p))
        };
        if let Some(r) = removed {
            let mut watcher = self.watcher.lock().expect("watcher");
            let _ = watcher.unwatch(&r.root);
            if !r.git_dir.starts_with(&r.root) {
                let _ = watcher.unwatch(&r.git_dir);
            }
        }
    }
}

/// Map an event path to the repository that should refresh, or `None` for noise.
fn classify(repos: &[WatchedRepo], path: &Path) -> Option<String> {
    let repo = repos
        .iter()
        .find(|r| path.starts_with(&r.root) || path.starts_with(&r.git_dir))?;
    let inside_git_dir = path.starts_with(r_git_dir(repo))
        || path
            .strip_prefix(&repo.root)
            .ok()
            .is_some_and(|rel| rel.starts_with(".git"));
    if is_noise(path, inside_git_dir) {
        return None;
    }
    Some(repo.id.clone())
}

fn r_git_dir(repo: &WatchedRepo) -> &Path {
    &repo.git_dir
}

fn is_noise(path: &Path, inside_git_dir: bool) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name.ends_with(".lock") || name == ".DS_Store" {
        return true;
    }
    if inside_git_dir {
        // Object and reflog writes happen constantly during Git operations and
        // are always followed by a ref/index/HEAD update that we do observe.
        let s = path.to_string_lossy();
        return s.contains("/objects/") || s.contains("/logs/") || s.contains("/lfs/");
    }
    false
}

/// Consume watcher notifications and request one refresh per repository per burst.
pub async fn run_debounce(
    mut rx: mpsc::UnboundedReceiver<String>,
    service: Arc<RepositoryService>,
    window: Duration,
) {
    let mut deadlines: HashMap<String, Instant> = HashMap::new();
    loop {
        let next_deadline = deadlines.values().min().copied();
        tokio::select! {
            received = rx.recv() => {
                match received {
                    Some(id) => {
                        deadlines.insert(id, Instant::now() + window);
                    }
                    None => break, // watcher dropped
                }
            }
            _ = async {
                match next_deadline {
                    Some(t) => tokio::time::sleep_until(t).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                let now = Instant::now();
                let ready: Vec<String> = deadlines
                    .iter()
                    .filter(|(_, t)| **t <= now)
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in ready {
                    deadlines.remove(&id);
                    service.request_refresh(&id, ChangeOrigin::Watcher);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repos() -> Vec<WatchedRepo> {
        let mut v = vec![
            WatchedRepo {
                id: "root".into(),
                root: "/w".into(),
                git_dir: "/w/.git".into(),
            },
            WatchedRepo {
                id: "child".into(),
                root: "/w/projects/a".into(),
                git_dir: "/w/projects/a/.git".into(),
            },
            WatchedRepo {
                id: "wt".into(),
                root: "/elsewhere/wt".into(),
                git_dir: "/main/.git/worktrees/wt".into(),
            },
        ];
        v.sort_by_key(|r| std::cmp::Reverse(r.root.as_os_str().len()));
        v
    }

    #[test]
    fn routes_to_most_specific_repository() {
        let r = repos();
        assert_eq!(
            classify(&r, Path::new("/w/projects/a/src/main.rs")).as_deref(),
            Some("child")
        );
        assert_eq!(
            classify(&r, Path::new("/w/README.md")).as_deref(),
            Some("root")
        );
        assert_eq!(
            classify(&r, Path::new("/main/.git/worktrees/wt/HEAD")).as_deref(),
            Some("wt")
        );
        assert_eq!(classify(&r, Path::new("/unrelated/file")), None);
    }

    #[test]
    fn drops_noise() {
        let r = repos();
        assert_eq!(classify(&r, Path::new("/w/.git/index.lock")), None);
        assert_eq!(classify(&r, Path::new("/w/.git/objects/ab/cdef")), None);
        assert_eq!(classify(&r, Path::new("/w/.git/logs/HEAD")), None);
        assert_eq!(
            classify(&r, Path::new("/w/.git/index")).as_deref(),
            Some("root")
        );
        assert_eq!(
            classify(&r, Path::new("/w/.git/refs/heads/main")).as_deref(),
            Some("root")
        );
        assert_eq!(classify(&r, Path::new("/w/src/.DS_Store")), None);
    }
}
