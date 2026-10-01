//! Fetching (SPEC §7, Fetching): the one operation that writes to a
//! repository, and only to remote-tracking refs, tags, and objects.
//!
//! `Fetcher` owns everything that decides how a fetch runs: which remote and
//! refspecs, the lock-file check, at most two fetches at once, never two on
//! the same Git directory, the recorded outcome, and auto-fetch backoff. It is
//! keyed by the shared Git directory (`Checkout::store`), because a checkout
//! and its linked worktrees share refs. `GitService::fetch` holds the hardened
//! command line.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::Semaphore;

use crate::db::{self, Db};
use crate::git::{busy_lock, Checkout, GitService, RefName};
use crate::models::{now_rfc3339, AppError, AppResult, ErrorCode};

/// Fetches running at once across all repositories.
const CONCURRENCY: usize = 2;
/// Longest wait between auto-fetch attempts after repeated failures.
const MAX_BACKOFF: Duration = Duration::from_secs(6 * 60 * 60);
/// A lock file older than this is a leftover, not a running Git command.
const STALE_LOCK: Duration = Duration::from_secs(10 * 60);
/// "Busy" results in a row before auto-fetch waits a full interval.
const BUSY_LIMIT: u32 = 3;

/// What to fetch.
#[derive(Debug, Clone)]
pub enum Scope {
    /// Fetch now: the remote's configured refspecs, minus any that would
    /// write outside remote-tracking refs and tags.
    Remote,
    /// Auto-fetch: only these watched branch patterns.
    Watched(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub remote: String,
    /// `None` when there was nothing to fetch (no watched branch exists yet).
    pub fetched_at: Option<String>,
    /// Short names of refs whose tips changed.
    pub moved: Vec<String>,
    /// Branches the remote no longer has, left out of this fetch.
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct Retry {
    failures: u32,
    busy: u32,
    next_at: Instant,
}

pub struct Fetcher {
    db: Db,
    /// Shared Git directories with a fetch in flight.
    in_flight: Mutex<HashSet<String>>,
    jobs: Semaphore,
    /// Auto-fetch only: when each Git directory may be tried again.
    retry: Mutex<HashMap<String, Retry>>,
}

/// Marks a Git directory as fetching until dropped. `Drop` runs on every exit
/// path, including early returns through `?`, so the mark is never left behind.
struct InFlight<'a> {
    set: &'a Mutex<HashSet<String>>,
    store: String,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.set.lock().expect("in-flight lock").remove(&self.store);
    }
}

impl Fetcher {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            in_flight: Mutex::new(HashSet::new()),
            jobs: Semaphore::new(CONCURRENCY),
            retry: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_fetching(&self, store: &str) -> bool {
        self.in_flight
            .lock()
            .expect("in-flight lock")
            .contains(store)
    }

    /// Fetch one checkout's remote and record the outcome for every checkout
    /// of its Git directory. A busy repository (another Git command holds a
    /// lock) fails with `CONFLICT` and records nothing.
    pub async fn fetch(
        &self,
        git: &GitService,
        checkout: &Checkout,
        scope: Scope,
        timeout: Duration,
    ) -> AppResult<Fetched> {
        let store = checkout.store();
        let _in_flight = {
            let mut set = self.in_flight.lock().expect("in-flight lock");
            if !set.insert(store.clone()) {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "This repository is already fetching.",
                ));
            }
            InFlight {
                set: &self.in_flight,
                store: store.clone(),
            }
        };
        let _permit = self
            .jobs
            .acquire()
            .await
            .map_err(|_| AppError::new(ErrorCode::Cancelled, "Fetch cancelled."))?;

        let outcome = self.run(git, checkout, scope, timeout).await;
        let recorded = match &outcome {
            Ok(f) => f.fetched_at.as_ref().map(|at| (at.clone(), None)),
            Err(e) if is_busy(e) => None,
            Err(e) => Some((now_rfc3339(), Some(e.clone()))),
        };
        if let Some((at, error)) = recorded {
            self.db
                .call(move |conn| db::store_fetch_outcome(conn, &store, &at, error.as_ref()))
                .await?;
        }
        outcome
    }

    async fn run(
        &self,
        git: &GitService,
        checkout: &Checkout,
        scope: Scope,
        timeout: Duration,
    ) -> AppResult<Fetched> {
        let root = &checkout.root;
        let remote = fetch_remote(git, root).await?;
        if let Some(lock) = busy_lock(&checkout.git_dir, &checkout.common_git_dir, &remote) {
            return Err(lock_error(&lock));
        }
        let before = git.remote_and_tag_tips(root).await?;
        let refspecs = match scope {
            Scope::Remote => git.safe_fetch_refspecs(root, &remote).await?,
            Scope::Watched(patterns) => auto_refspecs(&remote, &patterns),
        };
        if refspecs.is_empty() {
            return Ok(Fetched {
                remote,
                fetched_at: None,
                moved: Vec::new(),
                skipped: Vec::new(),
            });
        }
        let skipped = git.fetch(root, &remote, &refspecs, timeout).await?;
        let fetched_at = now_rfc3339();
        let after = git.remote_and_tag_tips(root).await?;
        let before: HashMap<String, String> = before.into_iter().collect();
        let moved = after
            .iter()
            .filter(|(name, target)| before.get(name) != Some(target))
            .map(|(name, _)| RefName::short(name).to_string())
            .collect();
        Ok(Fetched {
            remote,
            fetched_at: Some(fetched_at),
            moved,
            skipped,
        })
    }

    /// Whether an auto-fetch of `store` should start now: its last fetch (by
    /// anyone) is at least `interval` old, it is not fetching, and it is not
    /// waiting out a backoff.
    pub fn is_due(&self, store: &str, last_fetch_at: Option<&str>, interval: Duration) -> bool {
        if self.is_fetching(store) {
            return false;
        }
        let waiting = self
            .retry
            .lock()
            .expect("retry lock")
            .get(store)
            .is_some_and(|r| r.next_at > Instant::now());
        !waiting && older_than(last_fetch_at, interval)
    }

    /// Schedule the next auto-fetch attempt after one finished.
    pub fn record_auto(&self, store: &str, error: Option<&AppError>, interval: Duration) {
        let mut retry = self.retry.lock().expect("retry lock");
        let previous = retry.get(store).copied();
        let next = match error {
            // Also covers "nothing to fetch", which records no fetch time.
            None => Retry {
                failures: 0,
                busy: 0,
                next_at: Instant::now() + interval,
            },
            // Another Git command was running: try again next tick, unless it
            // keeps happening.
            Some(e) if is_busy(e) => {
                let busy = previous.map_or(0, |r| r.busy) + 1;
                Retry {
                    failures: previous.map_or(0, |r| r.failures),
                    busy: if busy >= BUSY_LIMIT { 0 } else { busy },
                    next_at: if busy >= BUSY_LIMIT {
                        Instant::now() + interval
                    } else {
                        Instant::now()
                    },
                }
            }
            Some(e) => {
                let failures = previous.map_or(0, |r| r.failures) + 1;
                let wait = backoff(interval, failures);
                tracing::info!(store, error = %e, wait_secs = wait.as_secs(), "auto-fetch failed; backing off");
                Retry {
                    failures,
                    busy: 0,
                    next_at: Instant::now() + wait,
                }
            }
        };
        retry.insert(store.to_string(), next);
    }
}

/// Exponential backoff: `interval × 2^failures`, at most `MAX_BACKOFF`.
fn backoff(interval: Duration, failures: u32) -> Duration {
    interval
        .saturating_mul(2u32.saturating_pow(failures.min(16)))
        .min(MAX_BACKOFF)
}

/// A lock that another Git command holds right now.
fn is_busy(e: &AppError) -> bool {
    e.code == ErrorCode::Conflict
}

/// "Busy" for a fresh lock file; a stale one is reported, since it blocks the
/// user's own Git commands too and will not go away on its own.
fn lock_error(lock: &Path) -> AppError {
    let age = std::fs::metadata(lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok());
    if age.is_some_and(|a| a > STALE_LOCK) {
        AppError::io(format!(
            "A leftover Git lock file blocks fetching: {}. If no Git command is running, delete it.",
            lock.display()
        ))
    } else {
        AppError::new(
            ErrorCode::Conflict,
            "Another Git command is using this repository. Try again in a moment.",
        )
        .with_details(lock.display().to_string())
    }
}

/// The later of Brainiac's last recorded fetch and the modification time of
/// `FETCH_HEAD`, which the user's own fetches write.
pub fn last_fetch_at(
    recorded: Option<&str>,
    git_dir: &Path,
    common_git_dir: &Path,
) -> Option<String> {
    let fetch_head = [git_dir, common_git_dir]
        .iter()
        .filter_map(|dir| std::fs::metadata(dir.join("FETCH_HEAD")).ok())
        .filter_map(|m| m.modified().ok())
        .max()
        .map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok();
    match (recorded, fetch_head) {
        (Some(a), Some(b)) => Some(if parse(a) >= parse(&b) {
            a.to_string()
        } else {
            b
        }),
        (a, b) => a.map(str::to_string).or(b),
    }
}

/// The remote to fetch: the current branch's upstream remote, else `origin`,
/// else the only remote.
async fn fetch_remote(git: &GitService, root: &Path) -> AppResult<String> {
    let remotes = git.remotes(root).await?;
    if let Some(branch) = git.head_branch(root).await {
        if let Some(remote) = git
            .config_value(root, &format!("branch.{branch}.remote"))
            .await
            .filter(|r| remotes.contains(r))
        {
            return Ok(remote);
        }
    }
    if remotes.iter().any(|r| r == "origin") {
        return Ok("origin".into());
    }
    match remotes.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(AppError::validation(
            "This repository has no remote to fetch from.",
        )),
        _ => Err(AppError::validation(
            "This repository has several remotes and none is named origin; set an upstream for the current branch.",
        )),
    }
}

/// One explicit refspec per watched branch pattern, with the standard mapping
/// to `refs/remotes/<remote>/`. The `+` allows forced updates, so a
/// force-push shows up as a rewrite. Patterns Git cannot use in a refspec
/// are skipped (`clean_patterns` rejects them on save). A branch the remote
/// no longer has is dropped by `GitService::fetch`.
pub fn auto_refspecs(remote: &str, patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .map(|p| p.trim())
        .filter(|p| crate::activity::refspec_pattern_ok(p))
        .map(|p| format!("+refs/heads/{p}:refs/remotes/{remote}/{p}"))
        .collect()
}

fn older_than(last: Option<&str>, interval: Duration) -> bool {
    match last.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) {
        Some(t) => {
            let age = chrono::Utc::now().signed_duration_since(t.with_timezone(&chrono::Utc));
            age.num_seconds() < 0 || age.num_seconds() as u64 >= interval.as_secs()
        }
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refspecs_for_names_and_globs() {
        let patterns = vec![
            "main".into(),
            "release/*".into(),
            "--upload-pack=x".into(),
            "release/[0-9]*".into(),
            "a*b*".into(),
        ];
        assert_eq!(
            auto_refspecs("origin", &patterns),
            vec![
                "+refs/heads/main:refs/remotes/origin/main".to_string(),
                "+refs/heads/release/*:refs/remotes/origin/release/*".to_string(),
            ]
        );
    }

    #[test]
    fn due_after_interval() {
        let hour = Duration::from_secs(3600);
        assert!(older_than(None, hour));
        assert!(!older_than(Some(&now_rfc3339()), hour));
        assert!(older_than(Some("2000-01-01T00:00:00Z"), hour));
    }

    #[test]
    fn backoff_doubles_up_to_six_hours() {
        let m = Duration::from_secs(15 * 60);
        assert_eq!(backoff(m, 1), Duration::from_secs(30 * 60));
        assert_eq!(backoff(m, 2), Duration::from_secs(60 * 60));
        assert_eq!(backoff(m, 20), MAX_BACKOFF);
    }

    #[test]
    fn busy_three_times_then_waits() {
        let tmp = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new(Db::open(&tmp.path().join("db.sqlite3")).unwrap());
        let busy = AppError::new(ErrorCode::Conflict, "busy");
        let hour = Duration::from_secs(3600);
        fetcher.record_auto("s", Some(&busy), hour);
        assert!(fetcher.is_due("s", None, hour));
        fetcher.record_auto("s", Some(&busy), hour);
        fetcher.record_auto("s", Some(&busy), hour);
        assert!(!fetcher.is_due("s", None, hour));
        fetcher.record_auto("t", Some(&AppError::io("down")), hour);
        assert!(!fetcher.is_due("t", None, hour));
    }
}
