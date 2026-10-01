//! The workspace activity feed (SPEC §7, Workspace activity).
//!
//! `ActivityTracker` turns moves of watched remote-tracking branches and tags
//! into events. It is keyed by the shared Git directory (`Checkout::store`),
//! so a checkout and its linked worktrees produce one feed, and it runs one
//! pass per directory at a time, so two observations that race (a fetch and
//! the watcher it triggers) cannot record the same move twice.
//!
//! A pass is cheap when nothing moved: the ref files' modification times and
//! the watched patterns are fingerprinted, and an unchanged fingerprint skips
//! Git and the database. The stored baseline remembers which patterns it
//! covers; refs that start matching later (a new pattern, a second workspace)
//! join it silently instead of arriving as news.
//!
//! Everything is read from local refs; news arrives when a fetch moves them.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::db::{self, Db, EventRow};
use crate::git::{Checkout, GitService, RefName};
use crate::models::{
    now_rfc3339, ActivityDetail, ActivityDrift, ActivityItem, ActivityKind, ActivitySettings,
    AppError, AppResult, HeadKind, PulseAuthor, StatusSnapshot, TeamPulse,
};

/// Commits listed on one event.
const EVENT_COMMITS: u32 = 5;
/// Overlapping paths listed on one event.
const MAX_CONFLICT_PATHS: usize = 20;
/// Events one pass records; further moves join the baseline silently, so a
/// burst (hundreds of new tags) cannot flood the feed or stall refreshes.
const MAX_EVENTS_PER_PASS: usize = 20;
/// Events kept, and events returned to the Activity tab.
const RETENTION_DAYS: i64 = 90;
const FEED_LIMIT: usize = 200;
/// At most one "branch moved" notification per repository and workspace in this window.
const NOTIFY_EVERY: Duration = Duration::from_secs(60 * 60);
/// Ref directories looked at for the fingerprint.
const MAX_STAMP_DIRS: usize = 5000;

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
    pub pulse: TeamPulse,
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

    /// Whether a stored event is watched.
    fn covers(&self, e: &EventRow) -> bool {
        self.matches(e.ref_name.starts_with("refs/tags/"), &e.match_name)
    }
}

/// A watched ref as observed now.
#[derive(Debug, Clone)]
struct Tip {
    full: String,
    target: String,
    is_tag: bool,
    match_name: String,
}

pub struct ActivityTracker {
    db: Db,
    /// One pass per shared Git directory at a time. A `tokio` mutex, because
    /// the pass holds it across `.await`s (Git subprocesses).
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Fingerprint of ref files and watched patterns at the last completed
    /// pass, per Git directory. `0` marks a directory nothing watches.
    fingerprints: Mutex<HashMap<String, u64>>,
    last_notified: Mutex<HashMap<(String, String), Instant>>,
}

impl ActivityTracker {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            locks: Mutex::new(HashMap::new()),
            fingerprints: Mutex::new(HashMap::new()),
            last_notified: Mutex::new(HashMap::new()),
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
            .copied()
    }

    fn set_fingerprint(&self, store: &str, value: u64) {
        self.fingerprints
            .lock()
            .expect("fingerprints")
            .insert(store.to_string(), value);
    }

    /// Record events for watched refs of `checkout`'s Git directory that moved
    /// since the last pass. `status` is the checkout's current observation,
    /// used for conflict-risk and drift details.
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
        let watched = Watched::union(watchers.iter().map(|w| &w.settings));
        if watched.is_empty() {
            // Not watched any more: drop the baseline once, so watching it
            // again later starts silently instead of reporting old moves.
            if self.fingerprint(&store) != Some(0) {
                let s = store.clone();
                self.db
                    .call(move |conn| db::forget_store(conn, &s, false))
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
        let tips: Vec<Tip> = git
            .remote_and_tag_tips(root)
            .await?
            .into_iter()
            .filter_map(|(full, target)| {
                let (is_tag, name) = match RefName::parse(&full, &remotes) {
                    RefName::RemoteBranch { branch, .. } => (false, branch.to_string()),
                    RefName::Tag(tag) => (true, tag.to_string()),
                    RefName::Other => return None,
                };
                watched.matches(is_tag, &name).then_some(Tip {
                    full,
                    target,
                    is_tag,
                    match_name: name,
                })
            })
            .collect();
        let previous = {
            let store = store.clone();
            self.db.call(move |conn| db::baseline(conn, &store)).await?
        };
        let (old_watched, stored) = match previous {
            Some((json, tips)) => (serde_json::from_str::<Watched>(&json).ok(), tips),
            None => (None, HashMap::new()),
        };

        let now = now_rfc3339();
        let mut events = Vec::new();
        let mut fork_base: Option<Option<String>> = None;
        for tip in &tips {
            // A ref the baseline did not cover joins it silently.
            let known = old_watched
                .as_ref()
                .is_some_and(|w| w.matches(tip.is_tag, &tip.match_name));
            if !known {
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
                // An old tip that no longer exists (garbage-collected after a
                // force-push) cannot be an ancestor: report a rewrite.
                (Some(old), false) => match git.is_ancestor(root, old, &tip.target).await {
                    Ok(true) => ActivityKind::Advanced,
                    _ => ActivityKind::Rewritten,
                },
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
                    db::save_pass(conn, &store, &watched_json, &tips, &to_save, &now, &cutoff)
                })
                .await?;
        }
        self.set_fingerprint(&store, stamp);

        let mut notices = Vec::new();
        for w in watchers.iter().filter(|w| w.settings.notify_moves) {
            let mine = Watched::of(&w.settings);
            let relevant: Vec<&EventRow> = events.iter().filter(|e| mine.covers(e)).collect();
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

    /// The feed of a workspace with `settings` whose members are `checkouts`:
    /// only events its own patterns match.
    pub async fn feed(
        &self,
        git: Option<&GitService>,
        settings: &ActivitySettings,
        checkouts: &[Checkout],
    ) -> AppResult<Feed> {
        let reps = representatives(checkouts);
        let stores: Vec<String> = reps.keys().cloned().collect();
        let watched = Watched::of(settings);
        let (events, unseen) = {
            let stores = stores.clone();
            self.db
                .call(move |conn| {
                    Ok((
                        db::list_events(conn, &stores, FEED_LIMIT * 4)?,
                        db::unseen_events(conn)?,
                    ))
                })
                .await?
        };
        let store_set: HashSet<String> = stores.into_iter().collect();
        let items = events
            .into_iter()
            .filter(|e| watched.covers(e))
            .take(FEED_LIMIT)
            .filter_map(|e| {
                let checkout = reps.get(&e.git_store)?;
                Some(to_item(e, checkout))
            })
            .collect();
        let pulse = match git {
            Some(git) => team_pulse(git, &reps.into_values().collect::<Vec<_>>(), &watched).await,
            None => TeamPulse::default(),
        };
        Ok(Feed {
            items,
            unseen: count_unseen(settings, &store_set, &unseen),
            pulse,
        })
    }

    /// Mark a workspace's unread events as seen: the listed ones, or all of
    /// them. Events its patterns do not match are left alone.
    pub async fn mark_seen(
        &self,
        settings: &ActivitySettings,
        checkouts: &[Checkout],
        event_ids: Option<&[String]>,
    ) -> AppResult<usize> {
        let stores: HashSet<String> = checkouts.iter().map(Checkout::store).collect();
        let watched = Watched::of(settings);
        let wanted: Option<HashSet<String>> = event_ids.map(|ids| ids.iter().cloned().collect());
        let at = now_rfc3339();
        self.db
            .call(move |conn| {
                let ids: Vec<String> = db::unseen_events(conn)?
                    .into_iter()
                    .filter(|e| stores.contains(&e.git_store) && watched.covers(e))
                    .filter(|e| wanted.as_ref().is_none_or(|w| w.contains(&e.id)))
                    .map(|e| e.id)
                    .collect();
                db::mark_events_seen(conn, &ids, &at)
            })
            .await
    }
}

/// Unread events a workspace's patterns match among its Git directories.
pub fn count_unseen(
    settings: &ActivitySettings,
    stores: &HashSet<String>,
    unseen: &[EventRow],
) -> u32 {
    let watched = Watched::of(settings);
    unseen
        .iter()
        .filter(|e| stores.contains(&e.git_store) && watched.covers(e))
        .count() as u32
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
/// `refs/tags`. Git updates a loose ref by renaming a lock file into its
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
    let mut stack = vec![
        common.join("refs").join("remotes"),
        common.join("refs").join("tags"),
    ];
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
    h.finish()
}

/// Glob matching with `*` (any run of characters, `/` included); every other
/// character matches itself.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == t[ti] && p[pi] != '*' {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            // Let the last `*` swallow one more character and retry.
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Characters a ref name (and so a pattern) may not contain, besides `*`.
const FORBIDDEN: &[char] = &['?', '[', ']', ':', '^', '~', '\\', '\0'];

/// Whether a branch pattern can become a fetch refspec: a valid ref name
/// with at most one `*`.
pub fn refspec_pattern_ok(p: &str) -> bool {
    pattern_ok(p) && p.matches('*').count() <= 1
}

fn pattern_ok(p: &str) -> bool {
    !p.is_empty()
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

/// What one repository contributed to the pulse.
struct RepoPulse {
    name: String,
    commits: Vec<(String, bool)>,
    releases: u32,
}

/// Commits, merges, releases, and authors on the watched refs over seven
/// days. Repositories are read in parallel.
async fn team_pulse(git: &GitService, checkouts: &[Checkout], watched: &Watched) -> TeamPulse {
    let since_time = chrono::Utc::now() - chrono::Duration::days(7);
    let since = since_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut jobs = tokio::task::JoinSet::new();
    for checkout in checkouts.iter().filter(|c| c.root.is_dir()) {
        // Each task owns clones: spawned tasks may outlive this borrow.
        let (git, checkout, watched, since) = (
            git.clone(),
            checkout.clone(),
            watched.clone(),
            since.clone(),
        );
        jobs.spawn(async move {
            let root = checkout.root.as_path();
            let remotes = git.remotes(root).await.unwrap_or_default();
            let tips = git.remote_and_tag_tips(root).await.unwrap_or_default();
            let branches: Vec<String> = tips
                .iter()
                .filter(|(full, _)| match RefName::parse(full, &remotes) {
                    RefName::RemoteBranch { branch, .. } => watched.matches(false, branch),
                    _ => false,
                })
                .map(|(full, _)| full.clone())
                .collect();
            let commits = git
                .commits_since(root, &branches, &since)
                .await
                .unwrap_or_default();
            let releases = git
                .tag_dates(root)
                .await
                .unwrap_or_default()
                .iter()
                .filter(|(name, date)| {
                    watched.matches(true, name)
                        && chrono::DateTime::parse_from_rfc3339(date).is_ok_and(|d| d >= since_time)
                })
                .count() as u32;
            RepoPulse {
                name: checkout.name.clone(),
                commits,
                releases,
            }
        });
    }
    let mut pulse = TeamPulse {
        since,
        ..TeamPulse::default()
    };
    let mut authors: Vec<PulseAuthor> = Vec::new();
    let mut per_repo: Vec<(String, u32)> = Vec::new();
    while let Some(Ok(repo)) = jobs.join_next().await {
        pulse.releases += repo.releases;
        let mut n = 0;
        for (author, is_merge) in repo.commits {
            if is_merge {
                pulse.merges += 1;
                continue;
            }
            pulse.commits += 1;
            n += 1;
            match authors.iter_mut().find(|a| a.name == author) {
                Some(a) => a.commits += 1,
                None => authors.push(PulseAuthor {
                    name: author,
                    commits: 1,
                }),
            }
        }
        if n > 0 {
            per_repo.push((repo.name, n));
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
