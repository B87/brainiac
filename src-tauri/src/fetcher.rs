//! Fetching (SPEC.md, Fetching): the one operation that writes to a
//! repository, and only to remote-tracking refs, tags, and objects.
//!
//! `Fetcher` owns everything that decides how a fetch runs: which remote and
//! refspecs, the lock-file check, at most two fetches at once, one fetch per
//! Git directory (a second request waits for the first and shares its
//! result), the recorded outcome, and, for auto-fetch, when to try again and
//! which watched branches the remote no longer has. It is keyed by the shared
//! Git directory (`Checkout::store`), because a checkout and its linked
//! worktrees share refs. `GitService::fetch` holds the hardened command line
//! (docs/architecture.md, Git).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::{watch, Semaphore};

use crate::db::{self, Db};
use crate::git::{busy_lock, Checkout, GitService, RefName};
use crate::models::{now_rfc3339, AppError, AppResult, ErrorCode};

/// Fetches running at once across all repositories.
const CONCURRENCY: usize = 2;
/// Longest wait between auto-fetch attempts after repeated failures.
const MAX_BACKOFF: Duration = Duration::from_secs(6 * 60 * 60);
/// A lock file older than this is a leftover, not a running Git command.
const STALE_LOCK: Duration = Duration::from_secs(10 * 60);
/// A lock file this far in the future (clock skew, network filesystems) is
/// not trusted to be fresh either.
const FUTURE_LOCK: Duration = Duration::from_secs(60);
/// "Busy" results in a row before auto-fetch waits a full interval.
const BUSY_LIMIT: u32 = 3;
/// How long a branch the remote does not have stays out of auto-fetch.
const MISSING_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// What to fetch.
#[derive(Debug, Clone)]
pub enum Scope {
    /// Fetch now: the remote of the current branch's upstream (else
    /// `origin`), with its configured refspecs minus any that would write
    /// outside remote-tracking refs and tags.
    Remote,
    /// Auto-fetch: these watched branch patterns from `origin` (else the only
    /// remote); failures back off from `interval`.
    Watched {
        patterns: Vec<String>,
        interval: Duration,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub remote: String,
    /// `None` when there was nothing to fetch (no watched branch exists).
    pub fetched_at: Option<String>,
    /// Short names of refs whose tips changed.
    pub moved: Vec<String>,
}

/// How a fetch request ended.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// This request ran the fetch (or found nothing to fetch).
    Fetched(Fetched),
    /// Another request was already fetching the same Git directory; this is
    /// its result. Its caller handles the aftermath.
    Joined(AppResult<Fetched>),
    /// Another Git command held a lock: nothing was fetched or recorded.
    Busy(AppError),
    /// The fetch failed; the error is recorded for the repository.
    Failed(AppError),
}

impl Outcome {
    pub fn into_result(self) -> AppResult<Fetched> {
        match self {
            Outcome::Fetched(f) => Ok(f),
            Outcome::Joined(r) => r,
            Outcome::Busy(e) | Outcome::Failed(e) => Err(e),
        }
    }

    /// Whether refs or the recorded fetch state changed, so the checkouts
    /// should be refreshed.
    pub fn changed_state(&self) -> bool {
        match self {
            Outcome::Fetched(f) => f.fetched_at.is_some(),
            Outcome::Failed(_) => true,
            Outcome::Joined(_) | Outcome::Busy(_) => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Retry {
    failures: u32,
    busy: u32,
    next_at: Instant,
}

/// Where a fetch in flight publishes its result.
type Pending = watch::Receiver<Option<AppResult<Fetched>>>;

pub struct Fetcher {
    db: Db,
    /// Git directories with a fetch in flight, and where to wait for it.
    in_flight: Mutex<HashMap<String, Pending>>,
    jobs: Semaphore,
    /// Auto-fetch only: when each Git directory may be tried again.
    retry: Mutex<HashMap<String, Retry>>,
    /// Auto-fetch only: watched branches each remote no longer has, as
    /// `refs/heads/<name>`, and since when.
    missing: Mutex<HashMap<String, (HashSet<String>, Instant)>>,
}

/// Removes a Git directory's in-flight entry when dropped. If the fetch is
/// cancelled before publishing, dropping the sender wakes the waiters with
/// an error instead of leaving them hanging.
struct InFlight<'a> {
    map: &'a Mutex<HashMap<String, Pending>>,
    store: String,
    sender: watch::Sender<Option<AppResult<Fetched>>>,
}

impl InFlight<'_> {
    fn publish(&self, result: &AppResult<Fetched>) {
        let _ = self.sender.send(Some(result.clone()));
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.map.lock().expect("in-flight lock").remove(&self.store);
    }
}

impl Fetcher {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            in_flight: Mutex::new(HashMap::new()),
            jobs: Semaphore::new(CONCURRENCY),
            retry: Mutex::new(HashMap::new()),
            missing: Mutex::new(HashMap::new()),
        }
    }

    /// Fetch one checkout's remote and record the outcome for every checkout
    /// of its Git directory.
    pub async fn fetch(
        &self,
        git: &GitService,
        checkout: &Checkout,
        scope: Scope,
        timeout: Duration,
    ) -> Outcome {
        let store = checkout.store();
        let claim = {
            let mut map = self.in_flight.lock().expect("in-flight lock");
            match map.get(&store) {
                Some(pending) => Err(pending.clone()),
                None => {
                    let (sender, receiver) = watch::channel(None);
                    map.insert(store.clone(), receiver);
                    Ok(InFlight {
                        map: &self.in_flight,
                        store: store.clone(),
                        sender,
                    })
                }
            }
        };
        let guard = match claim {
            Ok(guard) => guard,
            Err(mut pending) => return Outcome::Joined(wait(&mut pending).await),
        };

        let interval = match &scope {
            Scope::Watched { interval, .. } => Some(*interval),
            Scope::Remote => None,
        };
        let outcome = match self.jobs.acquire().await {
            Ok(_permit) => self.run(git, checkout, scope, timeout).await,
            Err(_) => Outcome::Failed(AppError::new(ErrorCode::Cancelled, "Fetch cancelled.")),
        };

        let recorded = match &outcome {
            Outcome::Fetched(f) => f.fetched_at.as_ref().map(|at| (at.clone(), None)),
            Outcome::Failed(e) => Some((now_rfc3339(), Some(e.clone()))),
            Outcome::Busy(_) | Outcome::Joined(_) => None,
        };
        if let Some((at, error)) = recorded {
            let s = store.clone();
            if let Err(e) = self
                .db
                .call(move |conn| db::store_fetch_outcome(conn, &s, &at, error.as_ref()))
                .await
            {
                tracing::warn!(store = %store, error = %e, "could not record a fetch outcome");
            }
        }
        if let Some(interval) = interval {
            self.schedule_next(&store, &outcome, interval);
        }
        guard.publish(&outcome.clone().into_result());
        outcome
    }

    async fn run(
        &self,
        git: &GitService,
        checkout: &Checkout,
        scope: Scope,
        timeout: Duration,
    ) -> Outcome {
        let store = checkout.store();
        let root = checkout.root.as_path();
        let watched = matches!(scope, Scope::Watched { .. });
        let remote = match fetch_remote(git, root, watched).await {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(e),
        };
        if let Some(lock) = busy_lock(&checkout.git_dir, &checkout.common_git_dir, &remote) {
            return lock_outcome(&lock);
        }
        let before = match git.remote_and_tag_tips(root).await {
            Ok(t) => t,
            Err(e) => return Outcome::Failed(e),
        };
        let refspecs = match &scope {
            Scope::Remote => match git.safe_fetch_refspecs(root, &remote).await {
                Ok(r) => r,
                Err(e) => return Outcome::Failed(e),
            },
            Scope::Watched { patterns, .. } => {
                let missing = self.missing_for(&store);
                auto_refspecs(&remote, patterns)
                    .into_iter()
                    .filter(|spec| !refspec_source_in(spec, &missing))
                    .collect()
            }
        };
        if refspecs.is_empty() {
            return Outcome::Fetched(Fetched {
                remote,
                fetched_at: None,
                moved: Vec::new(),
            });
        }
        let skipped = match git.fetch(root, &remote, &refspecs, timeout).await {
            Ok(skipped) => skipped,
            // Git itself found a lock held by another command.
            Err(e) if e.code == ErrorCode::Conflict => return Outcome::Busy(e),
            Err(e) => return Outcome::Failed(e),
        };
        self.remember_missing(&store, watched, skipped);
        let fetched_at = now_rfc3339();
        let after = match git.remote_and_tag_tips(root).await {
            Ok(t) => t,
            Err(e) => return Outcome::Failed(e),
        };
        let before: HashMap<String, String> =
            before.into_iter().map(|t| (t.name, t.target)).collect();
        let moved = after
            .iter()
            .filter(|t| before.get(&t.name) != Some(&t.target))
            .map(|t| RefName::short(&t.name).to_string())
            .collect();
        Outcome::Fetched(Fetched {
            remote,
            fetched_at: Some(fetched_at),
            moved,
        })
    }

    /// Branches the remote did not have, as found by recent auto-fetches.
    fn missing_for(&self, store: &str) -> HashSet<String> {
        let mut missing = self.missing.lock().expect("missing lock");
        match missing.get(store) {
            Some((_, since)) if since.elapsed() > MISSING_TTL => {
                missing.remove(store);
                HashSet::new()
            }
            Some((set, _)) => set.clone(),
            None => HashSet::new(),
        }
    }

    /// Auto-fetch adds what it skipped; Fetch now starts over, since the
    /// remote may have the branches again.
    fn remember_missing(&self, store: &str, watched: bool, skipped: Vec<String>) {
        let mut missing = self.missing.lock().expect("missing lock");
        if !watched {
            missing.remove(store);
            return;
        }
        if skipped.is_empty() {
            return;
        }
        let entry = missing
            .entry(store.to_string())
            .or_insert_with(|| (HashSet::new(), Instant::now()));
        entry.0.extend(skipped);
    }

    /// Whether an auto-fetch of `store` should start now: its last fetch (by
    /// anyone) is at least `interval` old, it is not fetching, and it is not
    /// waiting out a backoff.
    pub fn is_due(&self, store: &str, last_fetch_at: Option<&str>, interval: Duration) -> bool {
        if self
            .in_flight
            .lock()
            .expect("in-flight lock")
            .contains_key(store)
        {
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

    fn schedule_next(&self, store: &str, outcome: &Outcome, interval: Duration) {
        let mut retry = self.retry.lock().expect("retry lock");
        let previous = retry.get(store).copied();
        let next = match outcome {
            // Also covers "nothing to fetch", which records no fetch time.
            Outcome::Fetched(_) | Outcome::Joined(Ok(_)) => Retry {
                failures: 0,
                busy: 0,
                next_at: Instant::now() + interval,
            },
            // Another Git command was running: try again next tick, unless
            // it keeps happening.
            Outcome::Busy(_) => {
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
            Outcome::Failed(e) | Outcome::Joined(Err(e)) => {
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

    /// Drop everything held for a Git directory that is no longer registered.
    pub fn forget(&self, store: &str) {
        self.retry.lock().expect("retry lock").remove(store);
        self.missing.lock().expect("missing lock").remove(store);
    }
}

async fn wait(pending: &mut Pending) -> AppResult<Fetched> {
    loop {
        if let Some(result) = pending.borrow().clone() {
            return result;
        }
        if pending.changed().await.is_err() {
            return Err(AppError::new(ErrorCode::Cancelled, "Fetch cancelled."));
        }
    }
}

/// Exponential backoff: `interval × 2^failures`, at most `MAX_BACKOFF`.
fn backoff(interval: Duration, failures: u32) -> Duration {
    interval
        .saturating_mul(2u32.saturating_pow(failures.min(16)))
        .min(MAX_BACKOFF)
}

/// "Busy" for a fresh lock file. A stale one (old, or dated in the future)
/// is a failure that names the file: it blocks the user's own Git commands
/// too and will not go away on its own.
fn lock_outcome(lock: &Path) -> Outcome {
    let modified = std::fs::metadata(lock).and_then(|m| m.modified()).ok();
    let now = SystemTime::now();
    let stale = modified.is_some_and(|t| match now.duration_since(t) {
        Ok(age) => age > STALE_LOCK,
        Err(ahead) => ahead.duration() > FUTURE_LOCK,
    });
    if stale {
        Outcome::Failed(AppError::io(format!(
            "A leftover Git lock file blocks fetching: {}. If no Git command is running, delete it.",
            lock.display()
        )))
    } else {
        Outcome::Busy(
            AppError::new(
                ErrorCode::Conflict,
                "Another Git command is using this repository. Try again in a moment.",
            )
            .with_details(lock.display().to_string()),
        )
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

/// The remote to fetch. Auto-fetch watches where the team merges: `origin`,
/// else the only remote, else the upstream of the current branch. Fetch now
/// follows the current branch's upstream first, then `origin`, then the only
/// remote.
async fn fetch_remote(git: &GitService, root: &Path, watched: bool) -> AppResult<String> {
    let remotes = git.remotes(root).await?;
    let has_origin = remotes.iter().any(|r| r == "origin");
    if watched {
        if has_origin {
            return Ok("origin".into());
        }
        if let [only] = remotes.as_slice() {
            return Ok(only.clone());
        }
    }
    if let Some(branch) = git.head_branch(root).await {
        if let Some(remote) = git
            .config_value(root, &format!("branch.{branch}.remote"))
            .await
            .filter(|r| remotes.contains(r))
        {
            return Ok(remote);
        }
    }
    if has_origin {
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
/// are skipped (they are rejected on save). A branch the remote no longer
/// has is dropped by `GitService::fetch` and remembered here.
pub fn auto_refspecs(remote: &str, patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .map(|p| p.trim())
        .filter(|p| crate::activity::refspec_pattern_ok(p))
        .map(|p| format!("+refs/heads/{p}:refs/remotes/{remote}/{p}"))
        .collect()
}

fn refspec_source_in(spec: &str, sources: &HashSet<String>) -> bool {
    let spec = spec.strip_prefix('+').unwrap_or(spec);
    spec.split_once(':')
        .is_some_and(|(src, _)| sources.contains(src))
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
        let missing: HashSet<String> = ["refs/heads/main".to_string()].into();
        assert!(refspec_source_in(
            "+refs/heads/main:refs/remotes/origin/main",
            &missing
        ));
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
        let busy = Outcome::Busy(AppError::new(ErrorCode::Conflict, "busy"));
        let hour = Duration::from_secs(3600);
        fetcher.schedule_next("s", &busy, hour);
        assert!(fetcher.is_due("s", None, hour));
        fetcher.schedule_next("s", &busy, hour);
        fetcher.schedule_next("s", &busy, hour);
        assert!(!fetcher.is_due("s", None, hour));
        fetcher.schedule_next("t", &Outcome::Failed(AppError::io("down")), hour);
        assert!(!fetcher.is_due("t", None, hour));
        fetcher.forget("t");
        assert!(fetcher.is_due("t", None, hour));
    }

    #[test]
    fn future_dated_locks_are_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = tmp.path().join("packed-refs.lock");
        std::fs::write(&lock, "").unwrap();
        assert!(matches!(lock_outcome(&lock), Outcome::Busy(_)));
        let future = std::process::Command::new("touch")
            .args(["-t", "209901010000"])
            .arg(&lock)
            .status()
            .unwrap();
        assert!(future.success());
        assert!(matches!(lock_outcome(&lock), Outcome::Failed(_)));
    }
}
