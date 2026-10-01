//! The workspace activity feed (SPEC.md, Workspace activity).
//!
//! `ActivityTracker` turns moves of watched remote-tracking branches and tags
//! into events. It is keyed by the shared Git directory (`Checkout::store`),
//! so a checkout and its linked worktrees produce one feed, and it runs one
//! pass per directory at a time, so two observations that race (a fetch and
//! the watcher it triggers) cannot record the same move twice.
//!
//! The tracker is the only owner of its tables (the private `store` module):
//! its in-memory state (fingerprints, caches) can only be invalidated through
//! it, for example by `forget` when a repository's last checkout goes away.
//!
//! A pass is cheap when nothing moved: the ref files' modification times and
//! the watched patterns are fingerprinted, and an unchanged, recent
//! fingerprint skips Git and the database. The stored baseline remembers which
//! patterns it covers; refs that start matching later (a new pattern, a second
//! workspace, a new remote) join it silently instead of arriving as news.
//!
//! Everything is read from local refs; news arrives when a fetch moves them.

mod store;

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::db::{self, Db, WatchConfig};
use crate::git::{Checkout, GitService, RefName};
use crate::models::{
    now_rfc3339, ActivityDetail, ActivityDrift, ActivityItem, ActivityKind, ActivitySettings,
    AppError, AppResult, HeadKind, PulseAuthor, StatusSnapshot, TeamPulse,
};
use store::{EventRow, Filter};

/// Commits listed on one event.
const EVENT_COMMITS: u32 = 5;
/// Overlapping paths listed on one event.
const MAX_CONFLICT_PATHS: usize = 20;
/// Events one pass records, newest first; further moves join the baseline
/// silently, so a burst (hundreds of new tags) cannot flood the feed or
/// stall refreshes.
const MAX_EVENTS_PER_PASS: usize = 20;
/// Events kept, and events returned to the Activity tab.
const RETENTION_DAYS: i64 = 90;
const FEED_LIMIT: usize = 200;
/// At most one "branch moved" notification per repository and workspace in this window.
const NOTIFY_EVERY: Duration = Duration::from_secs(60 * 60);
/// Ref directories looked at for the fingerprint, per kind (remotes, tags).
const MAX_STAMP_DIRS: usize = 5000;
/// A fingerprint older than this is not trusted: filesystems with coarse
/// modification times can miss a second update within one tick.
const FINGERPRINT_TTL: Duration = Duration::from_secs(5 * 60);

/// A notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// What one tracking pass produced.
#[derive(Debug, Default)]
pub struct Pass {
    pub new_events: usize,
    pub notices: Vec<Notice>,
}

/// One workspace's feed.
#[derive(Debug)]
pub struct Feed {
    pub items: Vec<ActivityItem>,
    pub unseen: u32,
}

/// What the tracker needs to know about a workspace to count its unread events.
#[derive(Debug, Clone)]
pub struct WorkspaceScope {
    pub workspace_id: String,
    pub config: WatchConfig,
    /// The workspace's Git directories.
    pub stores: Vec<String>,
}

/// Watched patterns, sorted and without duplicates so that equal sets
/// serialize equally (the baseline compares them as JSON).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Watched {
    pub branches: Vec<String>,
    pub tags: Vec<String>,
}

impl Watched {
    pub fn of(settings: &ActivitySettings) -> Self {
        Self::union([settings])
    }

    pub fn union<'a>(settings: impl IntoIterator<Item = &'a ActivitySettings>) -> Self {
        let mut w = Watched::default();
        for s in settings {
            w.branches.extend(s.watched_branches.iter().cloned());
            w.tags.extend(s.watched_tags.iter().cloned());
        }
        for list in [&mut w.branches, &mut w.tags] {
            list.sort();
            list.dedup();
        }
        w
    }

    pub fn is_empty(&self) -> bool {
        self.branches.is_empty() && self.tags.is_empty()
    }

    /// Whether `name` (a branch without its remote, or a tag) is watched.
    pub fn matches(&self, is_tag: bool, name: &str) -> bool {
        let patterns = if is_tag { &self.tags } else { &self.branches };
        patterns.iter().any(|p| glob_match(p, name))
    }
}

/// A watched ref as observed now.
#[derive(Debug, Clone)]
struct Tip {
    full: String,
    target: String,
    is_tag: bool,
    /// The remote of a remote-tracking branch.
    remote: Option<String>,
    match_name: String,
    /// When the tag or tip commit was made, Unix seconds.
    time: i64,
}

#[derive(Debug, Clone, Copy)]
struct Fingerprint {
    value: u64,
    at: Instant,
}

pub struct ActivityTracker {
    db: Db,
    /// One pass per shared Git directory at a time. A `tokio` mutex, because
    /// the pass holds it across `.await`s (Git subprocesses).
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Fingerprint of ref files and watched patterns at the last completed
    /// pass, per Git directory. `value == 0` marks a directory nothing watches.
    fingerprints: Mutex<HashMap<String, Fingerprint>>,
    last_notified: Mutex<HashMap<(String, String), Instant>>,
    /// Team-pulse inputs per Git directory, reused while the ref files and
    /// the patterns they were read for are unchanged.
    pulse_cache: Mutex<HashMap<String, RepoPulse>>,
}

impl ActivityTracker {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            locks: Mutex::new(HashMap::new()),
            fingerprints: Mutex::new(HashMap::new()),
            last_notified: Mutex::new(HashMap::new()),
            pulse_cache: Mutex::new(HashMap::new()),
        }
    }

    fn lock_for(&self, store: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            self.locks
                .lock()
                .expect("tracker locks")
                .entry(store.to_string())
                .or_default(),
        )
    }

    fn fingerprint(&self, store: &str) -> Option<u64> {
        self.fingerprints
            .lock()
            .expect("fingerprints")
            .get(store)
            .filter(|f| f.value == 0 || f.at.elapsed() < FINGERPRINT_TTL)
            .map(|f| f.value)
    }

    fn set_fingerprint(&self, store: &str, value: u64) {
        self.fingerprints.lock().expect("fingerprints").insert(
            store.to_string(),
            Fingerprint {
                value,
                at: Instant::now(),
            },
        );
    }

    /// Forget a Git directory: its baseline and tips, and with `events` its
    /// feed, plus everything held in memory for it. The next pass starts over
    /// silently. Waits for a pass in progress, so none can write it back.
    pub async fn forget(&self, store: &str, events: bool) -> AppResult<()> {
        let lock = self.lock_for(store);
        let _pass = lock.lock().await;
        let s = store.to_string();
        self.db
            .call(move |conn| store::forget(conn, &s, events))
            .await?;
        self.fingerprints
            .lock()
            .expect("fingerprints")
            .remove(store);
        self.pulse_cache.lock().expect("pulse cache").remove(store);
        self.last_notified
            .lock()
            .expect("notify lock")
            .retain(|(s, _), _| s != store);
        if events {
            self.locks.lock().expect("tracker locks").remove(store);
        }
        Ok(())
    }

    /// Record events for watched refs of `checkout`'s Git directory that moved
    /// since the last pass. `status` is the checkout's current observation,
    /// used for conflict-risk and drift details. A Git failure that leaves
    /// the moves undecided fails the pass, which is then retried.
    pub async fn observe(
        &self,
        git: &GitService,
        checkout: &Checkout,
        status: &StatusSnapshot,
    ) -> AppResult<Pass> {
        let store = checkout.store();
        let lock = self.lock_for(&store);
        let _pass = lock.lock().await;

        let watchers = {
            let store = store.clone();
            self.db
                .call(move |conn| db::watchers_of_store(conn, &store))
                .await?
        };
        let watched = Watched::union(watchers.iter().map(|w| &w.config.settings));
        if watched.is_empty() {
            // Not watched any more: drop the baseline once, so watching it
            // again later starts silently instead of reporting old moves.
            if self.fingerprint(&store) != Some(0) {
                let s = store.clone();
                self.db
                    .call(move |conn| store::forget(conn, &s, false))
                    .await?;
                self.set_fingerprint(&store, 0);
            }
            return Ok(Pass::default());
        }
        let watched_json = serde_json::to_string(&watched)?;
        let stamp = {
            let mut h = DefaultHasher::new();
            watched_json.hash(&mut h);
            ref_stamp(&checkout.common_git_dir).hash(&mut h);
            // Never 0, which means "not watched".
            h.finish() | 1
        };
        if self.fingerprint(&store) == Some(stamp) {
            return Ok(Pass::default());
        }

        let root = checkout.root.as_path();
        let remotes = git.remotes(root).await?;
        let mut tips: Vec<Tip> = git
            .remote_and_tag_tips(root)
            .await?
            .into_iter()
            .filter_map(|t| {
                let (is_tag, remote, name) = match RefName::parse(&t.name, &remotes) {
                    RefName::RemoteBranch { remote, branch } => {
                        (false, Some(remote.to_string()), branch.to_string())
                    }
                    RefName::Tag(tag) => (true, None, tag.to_string()),
                    RefName::Other => return None,
                };
                watched.matches(is_tag, &name).then_some(Tip {
                    full: t.name,
                    target: t.target,
                    is_tag,
                    remote,
                    match_name: name,
                    time: t.time,
                })
            })
            .collect();
        // Newest first, so the per-pass cap keeps the latest news.
        tips.sort_by(|a, b| b.time.cmp(&a.time).then(a.full.cmp(&b.full)));
        let previous = {
            let store = store.clone();
            self.db
                .call(move |conn| store::baseline(conn, &store))
                .await?
        };
        let (old_watched, stored) = match previous {
            Some((json, tips)) => (serde_json::from_str::<Watched>(&json).ok(), tips),
            None => (None, HashMap::new()),
        };
        // Remotes the baseline already knew: a new or renamed remote's
        // branches join silently, they are not news.
        let known_remotes: HashSet<String> = stored
            .keys()
            .filter_map(|full| match RefName::parse(full, &remotes) {
                RefName::RemoteBranch { remote, .. } => Some(remote.to_string()),
                _ => None,
            })
            .collect();

        let now = now_rfc3339();
        let mut events = Vec::new();
        let mut fork_base: Option<Option<String>> = None;
        for tip in &tips {
            // A ref the baseline did not cover joins it silently.
            let covered = old_watched
                .as_ref()
                .is_some_and(|w| w.matches(tip.is_tag, &tip.match_name));
            let new_remote = tip
                .remote
                .as_ref()
                .is_some_and(|r| !known_remotes.contains(r));
            if !covered || new_remote {
                continue;
            }
            let old = stored.get(&tip.full);
            if old == Some(&tip.target) {
                continue;
            }
            let kind = match (old, tip.is_tag) {
                // A tag that moves is unusual and not news; just follow it.
                (Some(_), true) => continue,
                (None, true) => ActivityKind::Tagged,
                (None, false) => ActivityKind::Created,
                (Some(old), false) => classify_move(git, root, old, &tip.target).await?,
            };
            if events.len() == MAX_EVENTS_PER_PASS {
                tracing::info!(store = %store, "too many moved refs in one pass; the rest join the baseline");
                break;
            }
            if !tip.is_tag && fork_base.is_none() {
                fork_base = Some(fork_base_of(git, root, status, &tips).await);
            }
            // Details are best effort: a failure records the move without them,
            // and the tips still advance, so one bad ref cannot wedge the feed.
            let detail = describe(
                git,
                root,
                status,
                kind,
                tip,
                old.map(String::as_str),
                &tips,
                fork_base.as_ref().and_then(Option::as_deref),
            )
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(git_ref = %tip.full, error = %e, "could not describe a moved ref");
                ActivityDetail::default()
            });
            events.push(EventRow {
                id: uuid::Uuid::new_v4().to_string(),
                git_store: store.clone(),
                kind,
                ref_name: tip.full.clone(),
                match_name: tip.match_name.clone(),
                old_id: old.cloned(),
                new_id: tip.target.clone(),
                observed_at: now.clone(),
                seen_at: None,
                detail,
            });
        }

        let unchanged = events.is_empty()
            && old_watched.as_ref() == Some(&watched)
            && stored.len() == tips.len()
            && tips.iter().all(|t| stored.get(&t.full) == Some(&t.target));
        if !unchanged {
            let store = store.clone();
            let tips: Vec<(String, String)> = tips
                .iter()
                .map(|t| (t.full.clone(), t.target.clone()))
                .collect();
            let to_save = events.clone();
            let cutoff = (chrono::Utc::now() - chrono::Duration::days(RETENTION_DAYS))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            self.db
                .call(move |conn| {
                    store::save_pass(conn, &store, &watched_json, &tips, &to_save, &now, &cutoff)
                })
                .await?;
        }
        self.set_fingerprint(&store, stamp);

        let mut notices = Vec::new();
        for w in watchers.iter().filter(|w| w.config.settings.notify_moves) {
            let mine = Watched::of(&w.config.settings);
            let relevant: Vec<&EventRow> = events
                .iter()
                .filter(|e| mine.matches(e.ref_name.starts_with("refs/tags/"), &e.match_name))
                .collect();
            if relevant.is_empty() || !self.may_notify(&store, &w.workspace_id) {
                continue;
            }
            notices.push(notice(&checkout.name, &w.workspace_name, &relevant));
        }
        Ok(Pass {
            new_events: events.len(),
            notices,
        })
    }

    fn may_notify(&self, store: &str, workspace_id: &str) -> bool {
        let mut last = self.last_notified.lock().expect("notify lock");
        let key = (store.to_string(), workspace_id.to_string());
        if last.get(&key).is_some_and(|t| t.elapsed() < NOTIFY_EVERY) {
            return false;
        }
        last.insert(key, Instant::now());
        true
    }

    /// The feed of a workspace configured by `config` whose members are
    /// `checkouts`: only events its own patterns match, observed after it
    /// started watching them.
    pub async fn feed(&self, config: &WatchConfig, checkouts: &[Checkout]) -> AppResult<Feed> {
        let reps = representatives(checkouts);
        let filter = Filter {
            stores: reps.keys().cloned().collect(),
            patterns: config.patterns(),
        };
        let (events, unseen) = self
            .db
            .call(move |conn| {
                Ok((
                    store::list_events(conn, &filter, FEED_LIMIT)?,
                    store::count_unseen(conn, &filter)?,
                ))
            })
            .await?;
        let items = events
            .into_iter()
            .filter_map(|e| {
                let checkout = reps.get(&e.git_store)?;
                Some(to_item(e, checkout))
            })
            .collect();
        Ok(Feed { items, unseen })
    }

    /// Unread events per workspace, by workspace ID.
    pub async fn unread_counts(
        &self,
        scopes: Vec<WorkspaceScope>,
    ) -> AppResult<HashMap<String, u32>> {
        self.db
            .call(move |conn| {
                let mut counts = HashMap::new();
                for scope in scopes {
                    let filter = Filter {
                        stores: scope.stores,
                        patterns: scope.config.patterns(),
                    };
                    counts.insert(scope.workspace_id, store::count_unseen(conn, &filter)?);
                }
                Ok(counts)
            })
            .await
    }

    /// Commits, merges, releases, and authors on a workspace's watched refs
    /// over the last seven days. Each Git directory is read only when its
    /// ref files (or the patterns) changed since the last time; the others
    /// come from a cache, and the misses are read in parallel. A failed read
    /// is not cached; the previous reading stands in for it.
    pub async fn pulse(
        &self,
        git: &GitService,
        settings: &ActivitySettings,
        checkouts: &[Checkout],
    ) -> TeamPulse {
        let watched = Watched::of(settings);
        let watched_json = serde_json::to_string(&watched).unwrap_or_default();
        let since_time = chrono::Utc::now() - chrono::Duration::days(7);
        let mut repos: Vec<RepoPulse> = Vec::new();
        let mut jobs = tokio::task::JoinSet::new();
        for checkout in representatives(checkouts).into_values() {
            if !checkout.root.is_dir() {
                continue;
            }
            let store = checkout.store();
            let stamp = ref_stamp(&checkout.common_git_dir);
            let cached = self
                .pulse_cache
                .lock()
                .expect("pulse cache")
                .get(&store)
                .filter(|r| r.stamp == stamp && r.watched_json == watched_json)
                .cloned();
            match cached {
                Some(r) => repos.push(r),
                // Each task owns clones: spawned tasks may outlive this borrow.
                None => {
                    let (git, watched, watched_json) =
                        (git.clone(), watched.clone(), watched_json.clone());
                    jobs.spawn(async move {
                        let r =
                            read_pulse(&git, &checkout, &watched, since_time, stamp, watched_json)
                                .await;
                        (store, r)
                    });
                }
            }
        }
        while let Some(Ok((store, r))) = jobs.join_next().await {
            let mut cache = self.pulse_cache.lock().expect("pulse cache");
            match r {
                Ok(r) => {
                    cache.insert(store, r.clone());
                    repos.push(r);
                }
                Err(e) => {
                    tracing::warn!(store = %store, error = %e, "could not read the team pulse");
                    if let Some(old) = cache.get(&store) {
                        repos.push(old.clone());
                    }
                }
            }
        }
        summarize_pulse(&repos, since_time)
    }

    /// Mark a workspace's unread events as seen: the listed ones, or all of
    /// them. Events it does not show are left alone.
    pub async fn mark_seen(
        &self,
        config: &WatchConfig,
        checkouts: &[Checkout],
        event_ids: Option<&[String]>,
    ) -> AppResult<usize> {
        let filter = Filter {
            stores: representatives(checkouts).into_keys().collect(),
            patterns: config.patterns(),
        };
        let ids = event_ids.map(<[String]>::to_vec);
        let at = now_rfc3339();
        self.db
            .call(move |conn| store::mark_seen(conn, &filter, ids.as_deref(), &at))
            .await
    }
}

/// Whether a branch that moved from `old` to `new` advanced or was
/// rewritten. "Rewritten" needs evidence: `old` is not an ancestor, or no
/// longer exists (garbage-collected after a force-push). Any other failure
/// is an error, so the pass is retried rather than reporting a rewrite.
async fn classify_move(
    git: &GitService,
    root: &Path,
    old: &str,
    new: &str,
) -> AppResult<ActivityKind> {
    match git.is_ancestor(root, old, new).await {
        Ok(true) => Ok(ActivityKind::Advanced),
        Ok(false) => Ok(ActivityKind::Rewritten),
        Err(e) => match git.object_exists(root, old).await {
            Ok(false) => Ok(ActivityKind::Rewritten),
            _ => Err(e),
        },
    }
}

/// One checkout per Git directory, preferring the main checkout over a
/// linked worktree.
fn representatives(checkouts: &[Checkout]) -> HashMap<String, Checkout> {
    let mut reps: HashMap<String, Checkout> = HashMap::new();
    for c in checkouts {
        let replace = reps
            .get(&c.store())
            .is_none_or(|current| !current.is_main() && c.is_main());
        if replace {
            reps.insert(c.store(), c.clone());
        }
    }
    reps
}

/// A cheap fingerprint of the ref files: modification times of `packed-refs`,
/// the reftable list, and every directory under `refs/remotes` and
/// `refs/tags` (each walked separately, so many tag folders cannot crowd out
/// the remotes). Git updates a loose ref by renaming a lock file into its
/// directory, which changes that directory's time.
fn ref_stamp(common: &Path) -> u64 {
    let mtime = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
    };
    let mut h = DefaultHasher::new();
    for f in ["packed-refs", "reftable/tables.list"] {
        mtime(&common.join(f)).hash(&mut h);
    }
    for kind in ["remotes", "tags"] {
        let mut stack = vec![common.join("refs").join(kind)];
        let mut seen = 0;
        while let Some(dir) = stack.pop() {
            seen += 1;
            if seen > MAX_STAMP_DIRS {
                break;
            }
            dir.hash(&mut h);
            mtime(&dir).hash(&mut h);
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        stack.push(entry.path());
                    }
                }
            }
        }
    }
    h.finish()
}

/// Glob matching with `*` (any run of characters, `/` included); every other
/// character matches itself. Works on bytes: `*` never splits a character
/// that the pattern names, and nothing is allocated.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == t[ti] && p[pi] != b'*' {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            // Let the last `*` swallow one more byte and retry.
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

/// Characters a ref name (and so a pattern) may not contain, besides `*`.
const FORBIDDEN: &[char] = &['?', '[', ']', ':', '^', '~', '\\', '\0'];

/// Whether a branch pattern can become a fetch refspec: a valid ref name
/// with at most one `*`.
pub fn refspec_pattern_ok(p: &str) -> bool {
    pattern_ok(p) && p.matches('*').count() <= 1
}

fn pattern_ok(p: &str) -> bool {
    // `@` alone is reserved for HEAD; Git refuses it as a branch name even
    // though `refs/heads/@` passes `check-ref-format`.
    !p.is_empty()
        && p != "@"
        && !p.starts_with(['-', '/', '.'])
        && !p.ends_with(['/', '.'])
        && !p.ends_with(".lock")
        && !p.contains("..")
        && !p.contains("//")
        && !p.contains("@{")
        && !p.contains(FORBIDDEN)
        && !p.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Trim, drop empties and duplicates, and reject patterns Git could not use:
/// branch patterns become fetch refspecs, so they allow one `*` at most.
pub fn clean_patterns(patterns: &[String], branches: bool) -> AppResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for p in patterns {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        let ok = if branches {
            refspec_pattern_ok(p)
        } else {
            pattern_ok(p)
        };
        if !ok {
            return Err(AppError::validation(format!(
                "\"{p}\" is not a valid {} pattern. Use names and *{}; ? [ ] : ^ ~ and spaces are not allowed.",
                if branches { "branch" } else { "tag" },
                if branches { " (once)" } else { "" },
            )));
        }
        if !out.iter().any(|o| o == p) {
            out.push(p.to_string());
        }
    }
    Ok(out)
}

/// The watched remote branch the current branch was most likely forked from:
/// the one with the fewest commits unique to `HEAD`.
async fn fork_base_of(
    git: &GitService,
    root: &Path,
    status: &StatusSnapshot,
    tips: &[Tip],
) -> Option<String> {
    if status.head.kind != HeadKind::Branch {
        return None;
    }
    let mut best: Option<(u32, String)> = None;
    for tip in tips.iter().filter(|t| !t.is_tag) {
        let Ok(unique) = git.count_commits(root, "HEAD", &[&tip.full]).await else {
            continue;
        };
        if best.as_ref().is_none_or(|(n, _)| unique < *n) {
            best = Some((unique, tip.full.clone()));
        }
    }
    best.map(|(_, name)| RefName::short(&name).to_string())
}

/// Gather what an event shows: commits, counts, authors, overlap with the
/// working tree, and drift of the current branch.
#[allow(clippy::too_many_arguments)]
async fn describe(
    git: &GitService,
    root: &Path,
    status: &StatusSnapshot,
    kind: ActivityKind,
    tip: &Tip,
    old: Option<&str>,
    tips: &[Tip],
    fork_base: Option<&str>,
) -> AppResult<ActivityDetail> {
    let new = tip.target.as_str();
    let mut detail = ActivityDetail::default();
    match (kind, old) {
        (ActivityKind::Advanced | ActivityKind::Rewritten, Some(old)) => {
            detail.commits = git.range_commits(root, new, &[old], EVENT_COMMITS).await?;
            let stats = git.range_stats(root, new, &[old]).await?;
            detail.total_commits = stats.commits;
            detail.merges = stats.merges;
            detail.authors = stats.authors;
            if kind == ActivityKind::Rewritten {
                detail.replaced = git.count_commits(root, old, &[new]).await?;
            }
            let incoming = git.changed_paths(root, old, new, 2000).await?;
            let local: HashSet<&str> = status.entries.iter().map(|e| e.path.as_str()).collect();
            detail.conflict_paths = incoming
                .into_iter()
                .filter(|p| local.contains(p.as_str()))
                .take(MAX_CONFLICT_PATHS)
                .collect();
        }
        (ActivityKind::Created, _) => {
            // Commits on the new branch that no other watched branch (or HEAD) has.
            let mut others: Vec<&str> = tips
                .iter()
                .filter(|t| t.full != tip.full && !t.is_tag)
                .map(|t| t.full.as_str())
                .collect();
            if status.head.commit_id.is_some() {
                others.push("HEAD");
            }
            detail.commits = git.range_commits(root, new, &others, EVENT_COMMITS).await?;
            let stats = git.range_stats(root, new, &others).await?;
            detail.total_commits = stats.commits;
            detail.merges = stats.merges;
            detail.authors = stats.authors;
        }
        (ActivityKind::Tagged, _) => {
            detail.commits = git.range_commits(root, new, &[], 1).await?;
            detail.authors = detail
                .commits
                .iter()
                .map(|c| c.author_name.clone())
                .collect();
            if let Some(previous) = git.previous_tag(root, new).await {
                detail.commits_since_previous_tag =
                    Some(git.count_commits(root, new, &[&previous]).await?);
                detail.previous_tag = Some(previous);
            }
        }
        _ => {}
    }
    if kind != ActivityKind::Tagged {
        detail.drift = drift(git, root, status, RefName::short(&tip.full), new, fork_base).await;
    }
    Ok(detail)
}

/// How far the current branch is behind the moved ref, when that ref is its
/// upstream or the branch it was forked from.
async fn drift(
    git: &GitService,
    root: &Path,
    status: &StatusSnapshot,
    short: &str,
    new: &str,
    fork_base: Option<&str>,
) -> Option<ActivityDrift> {
    if status.head.kind != HeadKind::Branch {
        return None;
    }
    let branch = status.head.branch.clone()?;
    if let Some(up) = status.upstream.as_ref().filter(|u| u.ref_name == short) {
        return (up.behind > 0).then(|| ActivityDrift {
            branch,
            base: short.to_string(),
            behind: up.behind as u32,
        });
    }
    if fork_base != Some(short) {
        return None;
    }
    let behind = git.count_commits(root, new, &["HEAD"]).await.ok()?;
    (behind > 0).then(|| ActivityDrift {
        branch,
        base: short.to_string(),
        behind,
    })
}

fn to_item(e: EventRow, checkout: &Checkout) -> ActivityItem {
    ActivityItem {
        repository_id: checkout.repository_id.clone(),
        repository_name: checkout.name.clone(),
        ref_name: RefName::short(&e.ref_name).to_string(),
        id: e.id,
        kind: e.kind,
        full_ref: e.ref_name,
        old_id: e.old_id,
        new_id: e.new_id,
        observed_at: e.observed_at,
        seen: e.seen_at.is_some(),
        detail: e.detail,
    }
}

/// "api · Work" with one line per event, at most three.
fn notice(repository: &str, workspace: &str, events: &[&EventRow]) -> Notice {
    let lines: Vec<String> = events.iter().take(3).map(|e| summary_line(e)).collect();
    let more = events.len().saturating_sub(lines.len());
    let mut body = lines.join("\n");
    if more > 0 {
        body.push_str(&format!("\nand {more} more"));
    }
    Notice {
        title: format!("{repository} · {workspace}"),
        body,
    }
}

/// One line of a notification: "origin/main gained 4 commits (Alex Kim)".
fn summary_line(e: &EventRow) -> String {
    let d = &e.detail;
    let name = RefName::short(&e.ref_name);
    let by = d
        .authors
        .first()
        .map(|a| format!(" ({a})"))
        .unwrap_or_default();
    let commits = |n: u32| format!("{n} {}", if n == 1 { "commit" } else { "commits" });
    match e.kind {
        ActivityKind::Advanced => format!("{name} gained {}{by}", commits(d.total_commits)),
        ActivityKind::Rewritten => format!("{name} was rewritten{by}"),
        ActivityKind::Created => format!("{name} was created{by}"),
        ActivityKind::Tagged => format!("{name} was tagged{by}"),
    }
}

/// What one Git directory contributes to the pulse, before the seven-day
/// window is applied (so a cached entry stays correct as days pass: commits
/// only leave the window, and new ones change the ref files).
#[derive(Debug, Clone)]
struct RepoPulse {
    stamp: u64,
    /// The patterns it was read for.
    watched_json: String,
    name: String,
    /// Author, merge, commit time (Unix seconds).
    commits: Vec<(String, bool, i64)>,
    /// Creation times of watched tags (Unix seconds).
    releases: Vec<i64>,
}

async fn read_pulse(
    git: &GitService,
    checkout: &Checkout,
    watched: &Watched,
    since_time: chrono::DateTime<chrono::Utc>,
    stamp: u64,
    watched_json: String,
) -> AppResult<RepoPulse> {
    let root = checkout.root.as_path();
    let since = since_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let remotes = git.remotes(root).await?;
    let tips = git.remote_and_tag_tips(root).await?;
    let branches: Vec<String> = tips
        .iter()
        .filter(|t| match RefName::parse(&t.name, &remotes) {
            RefName::RemoteBranch { branch, .. } => watched.matches(false, branch),
            _ => false,
        })
        .map(|t| t.name.clone())
        .collect();
    let commits = git.commits_since(root, &branches, &since).await?;
    let releases = tips
        .iter()
        .filter_map(|t| match RefName::parse(&t.name, &remotes) {
            RefName::Tag(name) if watched.matches(true, name) => Some(t.time),
            _ => None,
        })
        .collect();
    Ok(RepoPulse {
        stamp,
        watched_json,
        name: checkout.name.clone(),
        commits,
        releases,
    })
}

/// Aggregate per-repository pulse inputs inside the seven-day window.
fn summarize_pulse(repos: &[RepoPulse], since_time: chrono::DateTime<chrono::Utc>) -> TeamPulse {
    let since = since_time.timestamp();
    let mut pulse = TeamPulse {
        since: since_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        ..TeamPulse::default()
    };
    let mut authors: Vec<PulseAuthor> = Vec::new();
    let mut per_repo: Vec<(String, u32)> = Vec::new();
    for repo in repos {
        pulse.releases += repo.releases.iter().filter(|t| **t >= since).count() as u32;
        let mut n = 0;
        for (author, is_merge, time) in &repo.commits {
            if *time < since {
                continue;
            }
            if *is_merge {
                pulse.merges += 1;
                continue;
            }
            pulse.commits += 1;
            n += 1;
            match authors.iter_mut().find(|a| &a.name == author) {
                Some(a) => a.commits += 1,
                None => authors.push(PulseAuthor {
                    name: author.clone(),
                    commits: 1,
                }),
            }
        }
        if n > 0 {
            per_repo.push((repo.name.clone(), n));
        }
    }
    authors.sort_by(|a, b| b.commits.cmp(&a.commits).then(a.name.cmp(&b.name)));
    authors.truncate(6);
    per_repo.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    pulse.authors = authors;
    pulse.repositories = per_repo.into_iter().take(3).map(|(r, _)| r).collect();
    pulse
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("main", "main"));
        assert!(!glob_match("main", "maintenance"));
        assert!(glob_match("release/*", "release/2.3"));
        assert!(glob_match("v*", "v1.2.0"));
        assert!(!glob_match("v?", "v1"), "? is literal");
        assert!(!glob_match("v*", "next"));
        assert!(glob_match("*", "anything/at/all"));
    }

    #[test]
    fn watched_patterns_are_canonical() {
        let a = ActivitySettings {
            watched_branches: vec!["main".into(), "develop".into()],
            ..ActivitySettings::default()
        };
        let b = ActivitySettings {
            watched_branches: vec!["develop".into(), "main".into(), "main".into()],
            ..ActivitySettings::default()
        };
        assert_eq!(Watched::of(&a), Watched::of(&b));
        let both = Watched::union([&a, &b]);
        assert!(both.matches(false, "develop"));
        assert!(both.matches(true, "v1.0"));
        assert!(!both.matches(false, "v1.0"));
    }

    #[test]
    fn cleans_patterns() {
        assert_eq!(
            clean_patterns(
                &[
                    " main ".into(),
                    "".into(),
                    "main".into(),
                    "release/*".into()
                ],
                true
            )
            .unwrap(),
            vec!["main".to_string(), "release/*".to_string()]
        );
        for bad in [
            "--force",
            "a..b",
            "release/[0-9]*",
            "v?",
            "a*b*",
            "x y",
            "a:b",
        ] {
            assert!(clean_patterns(&[bad.into()], true).is_err(), "{bad}");
        }
        assert!(clean_patterns(&["v*.*".into()], false).is_ok());
        assert!(clean_patterns(&["v*.*".into()], true).is_err());
    }

    #[test]
    fn pulse_counts_only_the_last_seven_days() {
        let now = chrono::Utc::now();
        let since = now - chrono::Duration::days(7);
        let t = |days: i64| (now - chrono::Duration::days(days)).timestamp();
        let repo = |name: &str, commits: Vec<(String, bool, i64)>| RepoPulse {
            stamp: 1,
            watched_json: String::new(),
            name: name.into(),
            commits,
            releases: vec![t(1), t(30)],
        };
        let a = |n: &str, merge: bool, days: i64| (n.to_string(), merge, t(days));
        let pulse = summarize_pulse(
            &[
                repo(
                    "api",
                    vec![a("Alex", false, 1), a("Alex", false, 2), a("Jo", true, 3)],
                ),
                repo("web", vec![a("Jo", false, 1), a("Sam", false, 9)]),
            ],
            since,
        );
        assert_eq!((pulse.commits, pulse.merges, pulse.releases), (3, 1, 2));
        assert_eq!(pulse.authors[0].name, "Alex");
        assert_eq!(
            pulse.repositories,
            vec!["api".to_string(), "web".to_string()]
        );
    }

    #[test]
    fn prefers_the_main_checkout() {
        let c = |id: &str, git_dir: &str| Checkout {
            repository_id: id.into(),
            name: id.into(),
            root: format!("/r/{id}").into(),
            git_dir: git_dir.into(),
            common_git_dir: "/r/main/.git".into(),
        };
        let reps = representatives(&[
            c("wt", "/r/main/.git/worktrees/wt"),
            c("main", "/r/main/.git"),
        ]);
        assert_eq!(reps.len(), 1);
        assert_eq!(reps["/r/main/.git"].repository_id, "main");
    }
}
