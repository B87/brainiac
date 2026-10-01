//! Repository registration, cached observations, and coalesced refresh jobs.
//! Workspace discovery and membership live in the `membership` child module.
//! `feed` and `fetching` connect the service to `ActivityTracker` and
//! `Fetcher`, which own the activity feed and fetching.
//!
//! `RepositoryService` is the application core for v0.1. It depends on
//! `GitService` and `Db` but not on Tauri: change events go through an
//! injected emitter closure, so the whole thing is testable with `cargo test`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Semaphore;

use crate::db::{self, Db, RepositoryRow};

mod feed;
mod fetching;
mod membership;
use crate::activity::ActivityTracker;
use crate::fetcher::Fetcher;
use crate::git::{GitService, LogQuery};
use crate::models::{
    now_rfc3339, AppError, AppResult, AppSnapshot, ChangeGroup, ChangeOrigin, ChangesResult,
    CommitDetail, CommitPage, DiffOptions, DiffResult, DiffSelector, GitInfo, ListCommitsRequest,
    RefsResult, RepositoryChangedEvent, RepositoryState, RepositorySummary, RepositoryTab,
    Settings,
};
pub use membership::WorkspaceChange;

/// Default page size and hard cap for history requests (SPEC.md, Workspaces and repositories).
const DEFAULT_PAGE: u32 = 100;
const MAX_PAGE: u32 = 500;
/// Observations older than this many refresh intervals are labeled stale.
const STALE_MULTIPLIER: u64 = 2;
/// Concurrent `git status` jobs (docs/architecture.md, Concurrency: start with two, tune after measurement).
const STATUS_CONCURRENCY: usize = 2;

pub type Emitter = Arc<dyn Fn(RepositoryChangedEvent) + Send + Sync>;

/// Shows a macOS notification with a title and body. Injected like `Emitter`
/// so the service stays independent of Tauri; tests leave it unset.
pub type Notifier = Arc<dyn Fn(String, String) + Send + Sync>;

/// Per-repository refresh bookkeeping: at most one running job and one pending request.
#[derive(Default)]
struct RefreshSlot {
    running: bool,
    pending: Option<ChangeOrigin>,
}

pub struct RepositoryService {
    db: Db,
    git: Option<GitService>,
    git_info: GitInfo,
    settings: Mutex<Settings>,
    version: AtomicU64,
    emitter: Emitter,
    slots: Mutex<HashMap<String, RefreshSlot>>,
    status_jobs: Arc<Semaphore>,
    notifier: Mutex<Option<Notifier>>,
    fetcher: Fetcher,
    tracker: ActivityTracker,
}

impl RepositoryService {
    pub fn new(
        db: Db,
        git: Result<GitService, AppError>,
        settings: Settings,
        emitter: Emitter,
    ) -> Self {
        let (git, git_info) = match git {
            Ok(g) => {
                let info = GitInfo {
                    available: true,
                    version: Some(g.version().to_string()),
                    path: Some(g.binary().display().to_string()),
                    message: None,
                };
                (
                    Some(g.with_timeout(Duration::from_secs(settings.status_timeout_seconds))),
                    info,
                )
            }
            Err(e) => (
                None,
                GitInfo {
                    available: false,
                    version: None,
                    path: None,
                    message: Some(e.message),
                },
            ),
        };
        Self {
            fetcher: Fetcher::new(db.clone()),
            tracker: ActivityTracker::new(db.clone()),
            db,
            git,
            git_info,
            settings: Mutex::new(settings),
            version: AtomicU64::new(1),
            emitter,
            slots: Mutex::new(HashMap::new()),
            status_jobs: Arc::new(Semaphore::new(STATUS_CONCURRENCY)),
            notifier: Mutex::new(None),
        }
    }

    /// Install the function that shows macOS notifications.
    pub fn set_notifier(&self, notifier: Notifier) {
        *self.notifier.lock().expect("notifier lock") = Some(notifier);
    }

    fn notify(&self, title: String, body: String) {
        // Clone the `Arc` out so the lock is not held while the notification is shown.
        let notifier = self.notifier.lock().expect("notifier lock").clone();
        if let Some(n) = notifier {
            n(title, body);
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn git_info(&self) -> &GitInfo {
        &self.git_info
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().expect("settings lock").clone()
    }

    pub fn snapshot_version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    fn bump_version(&self) -> u64 {
        self.version.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn git(&self) -> AppResult<&GitService> {
        self.git.as_ref().ok_or_else(|| {
            AppError::dependency(
                self.git_info
                    .message
                    .clone()
                    .unwrap_or_else(|| "Git is not available.".into()),
            )
        })
    }

    // -----------------------------------------------------------------------
    // Registration
    // -----------------------------------------------------------------------

    /// Register the repository containing `path` (or return the existing registration),
    /// mark it recently opened, and take a first status observation.
    pub async fn register(self: &Arc<Self>, path: &Path) -> AppResult<RepositorySummary> {
        let git = self.git()?;
        let resolved = git.resolve(path).await?;
        let display_path = resolved.root.display().to_string();
        let now = now_rfc3339();

        let existing = {
            let dp = display_path.clone();
            self.db
                .call(move |conn| db::find_repository_by_display_path(conn, &dp))
                .await?
        };
        let id = match existing {
            Some(row) => {
                let id = row.id.clone();
                let at = now.clone();
                let id2 = id.clone();
                self.db
                    .call(move |conn| db::touch_repository_opened(conn, &id2, &at))
                    .await?;
                id
            }
            None => {
                let row = RepositoryRow {
                    id: uuid::Uuid::new_v4().to_string(),
                    canonical_root: resolved.root.display().to_string(),
                    display_path,
                    git_dir: resolved.git_dir.display().to_string(),
                    common_git_dir: resolved.common_git_dir.display().to_string(),
                    created_at: now.clone(),
                    last_opened_at: Some(now.clone()),
                    last_checked_at: None,
                    last_tab: None,
                    status: None,
                    error: None,
                    last_fetch_at: None,
                    last_fetch_error: None,
                };
                let id = row.id.clone();
                self.db
                    .call(move |conn| db::insert_repository(conn, &row))
                    .await?;
                id
            }
        };
        self.refresh(&id, ChangeOrigin::Registration).await
    }

    pub async fn remove(&self, id: &str) -> AppResult<()> {
        let id2 = id.to_string();
        let removed = self
            .db
            .call(move |conn| db::delete_repository(conn, &id2))
            .await?;
        if !removed {
            return Err(AppError::not_found("That repository is not registered."));
        }
        self.slots.lock().expect("slots").remove(id);
        let version = self.bump_version();
        (self.emitter)(RepositoryChangedEvent {
            repository_id: id.to_string(),
            snapshot_version: version,
            origin: ChangeOrigin::Refresh,
            changed: true,
        });
        Ok(())
    }

    pub async fn mark_opened(&self, id: &str) -> AppResult<()> {
        let id2 = id.to_string();
        let now = now_rfc3339();
        self.db
            .call(move |conn| db::touch_repository_opened(conn, &id2, &now))
            .await
    }

    pub async fn set_tab(&self, id: &str, tab: RepositoryTab) -> AppResult<()> {
        let id2 = id.to_string();
        self.db
            .call(move |conn| db::set_repository_tab(conn, &id2, tab))
            .await
    }

    // -----------------------------------------------------------------------
    // Reads
    // -----------------------------------------------------------------------

    pub async fn row(&self, id: &str) -> AppResult<RepositoryRow> {
        let id2 = id.to_string();
        self.db
            .call(move |conn| db::get_repository(conn, &id2))
            .await?
            .ok_or_else(|| AppError::not_found("That repository is not registered."))
    }

    pub async fn rows(&self) -> AppResult<Vec<RepositoryRow>> {
        self.db.call(|conn| db::list_repositories(conn)).await
    }

    pub async fn list(&self) -> AppResult<Vec<RepositorySummary>> {
        let rows = self.rows().await?;
        let settings = self.settings();
        let slots = self.slots.lock().expect("slots");
        Ok(rows
            .iter()
            .map(|r| self.summarize(r, &settings, slots.get(&r.id).is_some_and(|s| s.running)))
            .collect())
    }

    pub async fn snapshot(&self) -> AppResult<AppSnapshot> {
        let repositories = self.list().await?;
        let workspaces = self.workspaces().await?;
        let (pins, recent) = self
            .db
            .call(|conn| Ok((db::list_pins(conn)?, db::recent_repository_ids(conn, 10)?)))
            .await?;
        Ok(AppSnapshot {
            snapshot_version: self.snapshot_version(),
            git: self.git_info.clone(),
            repositories,
            workspaces,
            pins,
            recent_repository_ids: recent,
            settings: self.settings(),
        })
    }

    fn summarize(
        &self,
        row: &RepositoryRow,
        settings: &Settings,
        refreshing: bool,
    ) -> RepositorySummary {
        let root = PathBuf::from(&row.canonical_root);
        let state = if refreshing {
            RepositoryState::Refreshing
        } else if !root.is_dir() {
            RepositoryState::Missing
        } else if row.error.is_some() {
            RepositoryState::Error
        } else if is_stale(
            row.last_checked_at.as_deref(),
            settings.refresh_interval_seconds * STALE_MULTIPLIER,
        ) {
            RepositoryState::Stale
        } else {
            RepositoryState::Fresh
        };
        RepositorySummary {
            id: row.id.clone(),
            name: root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| row.canonical_root.clone()),
            canonical_root: row.canonical_root.clone(),
            display_path: row.display_path.clone(),
            state,
            last_checked_at: row.last_checked_at.clone(),
            last_commit_at: row.status.as_ref().and_then(|s| s.last_commit_at.clone()),
            head: row.status.as_ref().map(|s| s.head.clone()),
            counts: row.status.as_ref().map(|s| s.counts),
            upstream: row.status.as_ref().and_then(|s| s.upstream.clone()),
            error: row.error.clone(),
            last_tab: row.last_tab,
            last_fetch_at: crate::fetcher::last_fetch_at(
                row.last_fetch_at.as_deref(),
                Path::new(&row.git_dir),
                Path::new(&row.common_git_dir),
            ),
            fetch_error: row.last_fetch_error.clone(),
        }
    }

    // -----------------------------------------------------------------------
    // Refresh
    // -----------------------------------------------------------------------

    /// Observe status now, store it, bump the snapshot version, and emit a change event.
    /// Observe status and emit `repository_changed`. Watcher, manual, and
    /// registration refreshes always report a change: a file edit can leave
    /// `git status` identical while the displayed diff is out of date.
    pub async fn refresh(
        self: &Arc<Self>,
        id: &str,
        origin: ChangeOrigin,
    ) -> AppResult<RepositorySummary> {
        let force = matches!(
            origin,
            ChangeOrigin::Watcher | ChangeOrigin::Refresh | ChangeOrigin::Registration
        );
        self.observe(id, origin, force).await
    }

    async fn observe(
        self: &Arc<Self>,
        id: &str,
        origin: ChangeOrigin,
        force_changed: bool,
    ) -> AppResult<RepositorySummary> {
        let row = self.row(id).await?;
        let outcome = {
            // Only `git status` counts against the status job limit; ref
            // tracking below has its own per-repository serialization.
            let _permit = self.status_jobs.acquire().await.map_err(|_| {
                AppError::new(crate::models::ErrorCode::Cancelled, "Refresh cancelled.")
            })?;
            match self.git() {
                Ok(git) => git.status(Path::new(&row.canonical_root)).await,
                Err(e) => Err(e),
            }
        };
        let checked_at = now_rfc3339();
        let previous = row.status.as_ref().map(without_timestamp);
        let (status, error) = match outcome {
            Ok(s) => (Some(s), None),
            Err(e) => {
                tracing::warn!(repository = %row.display_path, error = %e, "status refresh failed");
                (None, Some(e))
            }
        };
        {
            let id2 = id.to_string();
            let at = checked_at.clone();
            let status2 = status.clone();
            let error2 = error.clone();
            self.db
                .call(move |conn| {
                    db::store_observation(conn, &id2, &at, status2.as_ref(), error2.as_ref())
                })
                .await?;
        }
        // Turn moved watched refs into activity events before announcing the
        // change, so the snapshot that follows already counts them.
        let new_events = match &status {
            Some(s) => self.track(&row, s).await > 0,
            None => false,
        };
        let changed = force_changed
            || new_events
            || previous != status.as_ref().map(without_timestamp)
            || row.error != error;
        let version = self.bump_version();
        (self.emitter)(RepositoryChangedEvent {
            repository_id: id.to_string(),
            snapshot_version: version,
            origin,
            changed,
        });
        let row = self.row(id).await?;
        let settings = self.settings();
        Ok(self.summarize(&row, &settings, false))
    }

    /// Request a background refresh. Requests for a repository that is already
    /// refreshing collapse into a single pending follow-up (docs/architecture.md, Concurrency and lifecycle).
    pub fn request_refresh(self: &Arc<Self>, id: &str, origin: ChangeOrigin) {
        {
            let mut slots = self.slots.lock().expect("slots");
            let slot = slots.entry(id.to_string()).or_default();
            if slot.running {
                slot.pending = Some(origin);
                return;
            }
            slot.running = true;
        }
        let service = Arc::clone(self);
        let id = id.to_string();
        tokio::spawn(async move {
            let mut origin = origin;
            loop {
                if let Err(e) = service.refresh(&id, origin).await {
                    tracing::warn!(repository = %id, error = %e, "background refresh failed");
                }
                let next = {
                    let mut slots = service.slots.lock().expect("slots");
                    match slots.get_mut(&id) {
                        Some(slot) => match slot.pending.take() {
                            Some(o) => Some(o),
                            None => {
                                slot.running = false;
                                None
                            }
                        },
                        None => None, // repository was removed meanwhile
                    }
                };
                match next {
                    Some(o) => origin = o,
                    None => break,
                }
            }
        });
    }

    /// Refresh every registered repository whose observation is older than `min_age`.
    pub async fn request_refresh_all(
        self: &Arc<Self>,
        origin: ChangeOrigin,
        min_age: Duration,
    ) -> AppResult<()> {
        for row in self.rows().await? {
            if is_stale(row.last_checked_at.as_deref(), min_age.as_secs()) {
                self.request_refresh(&row.id, origin);
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Git queries
    // -----------------------------------------------------------------------

    /// A fresh status observation with the full entry list (the dashboard uses
    /// the cached counts; the Changes view wants the entries).
    pub async fn changes(self: &Arc<Self>, id: &str) -> AppResult<ChangesResult> {
        // Never forced: the caller is the view that would reload on a change.
        self.observe(id, ChangeOrigin::Refresh, false).await?;
        let row = self.row(id).await?;
        match (row.status, row.error) {
            (Some(s), None) => {
                let mut entries = s.entries;
                // Line counts are a nicety: a failure leaves them empty.
                if let Ok((staged, unstaged)) = self
                    .git()?
                    .change_stats(Path::new(&row.canonical_root))
                    .await
                {
                    for e in &mut entries {
                        let stats = match e.group {
                            ChangeGroup::Staged => &staged,
                            ChangeGroup::Unstaged => &unstaged,
                            _ => continue,
                        };
                        if let Some(&(adds, dels)) = stats.get(&e.path) {
                            e.additions = adds;
                            e.deletions = dels;
                        }
                    }
                }
                Ok(ChangesResult {
                    repository_id: id.to_string(),
                    observed_at: s.observed_at,
                    head: s.head,
                    counts: s.counts,
                    entries,
                })
            }
            (_, Some(e)) => Err(e),
            (None, None) => Err(AppError::io("No status observation is available yet.")),
        }
    }

    pub async fn diff(
        &self,
        id: &str,
        selector: DiffSelector,
        options: DiffOptions,
    ) -> AppResult<DiffResult> {
        let row = self.row(id).await?;
        let limits = self.settings().diff_limits;
        let content = self
            .git()?
            .diff(Path::new(&row.canonical_root), &selector, &limits, options)
            .await?;
        Ok(DiffResult {
            repository_id: id.to_string(),
            selector,
            content,
        })
    }

    /// One commit's metadata and changed files, compared with parent `parent_index` (default 0).
    pub async fn commit(
        &self,
        id: &str,
        commit_id: &str,
        parent_index: Option<u32>,
    ) -> AppResult<CommitDetail> {
        let row = self.row(id).await?;
        let mut detail = self
            .git()?
            .commit_detail(
                Path::new(&row.canonical_root),
                commit_id,
                parent_index.unwrap_or(0),
            )
            .await?;
        detail.repository_id = id.to_string();
        Ok(detail)
    }

    pub async fn refs(&self, id: &str) -> AppResult<RefsResult> {
        let row = self.row(id).await?;
        let (refs, base) = self
            .git()?
            .list_refs_with_base(Path::new(&row.canonical_root))
            .await?;
        let base = base.and_then(|full| {
            refs.iter()
                .find(|r| r.full_name == full)
                .map(|r| r.name.clone())
        });
        Ok(RefsResult {
            repository_id: id.to_string(),
            base,
            refs,
        })
    }

    pub async fn commits(&self, request: ListCommitsRequest) -> AppResult<CommitPage> {
        let row = self.row(&request.repository_id).await?;
        let root = PathBuf::from(&row.canonical_root);
        let git = self.git()?;
        let ref_name = request.ref_name.clone().unwrap_or_else(|| "HEAD".into());
        let limit = request.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);

        // Resolve the anchor once; cursors carry it so paging survives new commits.
        let (anchor, offset) = match request.cursor.as_deref() {
            Some(cursor) => decode_cursor(cursor)?,
            None => match git.resolve_commit(&root, &ref_name).await? {
                Some(id) => (id, 0),
                None => {
                    return Ok(CommitPage {
                        repository_id: request.repository_id,
                        ref_name,
                        anchor_commit_id: String::new(),
                        items: Vec::new(),
                        next_cursor: None,
                    })
                }
            },
        };

        let filter = request
            .filter
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty());
        let query = LogQuery {
            filter,
            author: request.author.as_deref(),
            exclude: request.exclude.as_deref().filter(|x| !x.is_empty()),
        };
        let mut items = git.log(&root, &anchor, offset, limit, &query).await?;
        if items.is_empty() && offset == 0 && query.author.is_none() && query.exclude.is_none() {
            if let Some(f) = filter {
                items = git.log_by_hash_prefix(&root, &anchor, f).await?;
            }
        }
        let next_cursor = if items.len() as u32 > limit {
            items.truncate(limit as usize);
            Some(encode_cursor(&anchor, offset + limit))
        } else {
            None
        };
        Ok(CommitPage {
            repository_id: request.repository_id,
            ref_name,
            anchor_commit_id: anchor,
            items,
            next_cursor,
        })
    }

    // -----------------------------------------------------------------------
    // Editor and Finder
    // -----------------------------------------------------------------------

    /// Launch the configured editor with a fixed argument template (docs/architecture.md, Security).
    pub async fn open_in_editor(
        &self,
        id: &str,
        file: Option<&str>,
        line: Option<u64>,
    ) -> AppResult<()> {
        let row = self.row(id).await?;
        let settings = self.settings();
        let root = PathBuf::from(&row.canonical_root);
        let (template, target) = match file {
            Some(f) => {
                crate::git::validate_repo_path(f)?;
                (settings.editor.file_args.clone(), root.join(f))
            }
            None => (settings.editor.repo_args.clone(), root.clone()),
        };
        let target = target.display().to_string();
        let line = line.unwrap_or(1).to_string();
        let args: Vec<String> = template
            .iter()
            .map(|a| a.replace("{path}", &target).replace("{line}", &line))
            .collect();
        tokio::process::Command::new(&settings.editor.executable)
            .args(&args)
            .current_dir(&root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                AppError::dependency(format!(
                    "Could not start the editor \"{}\". Check Settings.",
                    settings.editor.executable
                ))
                .with_details(e.to_string())
            })?;
        Ok(())
    }

    pub async fn repository_root(&self, id: &str) -> AppResult<PathBuf> {
        Ok(PathBuf::from(self.row(id).await?.canonical_root))
    }
}

/// A status snapshot with its observation time cleared, for change comparison.
fn without_timestamp(s: &crate::models::StatusSnapshot) -> crate::models::StatusSnapshot {
    crate::models::StatusSnapshot {
        observed_at: String::new(),
        ..s.clone()
    }
}

fn is_stale(last_checked_at: Option<&str>, max_age_seconds: u64) -> bool {
    match last_checked_at.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) {
        Some(t) => {
            let age = chrono::Utc::now().signed_duration_since(t.with_timezone(&chrono::Utc));
            age.num_seconds() < 0 || age.num_seconds() as u64 > max_age_seconds
        }
        None => true,
    }
}

fn encode_cursor(anchor: &str, offset: u32) -> String {
    format!("{anchor}:{offset}")
}

fn decode_cursor(cursor: &str) -> AppResult<(String, u32)> {
    let (anchor, offset) = cursor
        .rsplit_once(':')
        .ok_or_else(|| AppError::validation("Malformed history cursor."))?;
    let offset: u32 = offset
        .parse()
        .map_err(|_| AppError::validation("Malformed history cursor."))?;
    crate::git::validate_revision(anchor)?;
    if !anchor.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::validation("Malformed history cursor."));
    }
    Ok((anchor.to_string(), offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trip() {
        let c = encode_cursor("abcdef", 100);
        assert_eq!(decode_cursor(&c).unwrap(), ("abcdef".into(), 100));
        assert!(decode_cursor("not-a-cursor").is_err());
        assert!(decode_cursor("--evil:3").is_err());
    }

    #[test]
    fn staleness() {
        assert!(is_stale(None, 10));
        assert!(!is_stale(Some(&now_rfc3339()), 10));
        assert!(is_stale(Some("2000-01-01T00:00:00Z"), 10));
    }
}
