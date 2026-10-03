//! `PullRequestService` (docs/architecture.md, Pull requests — v0.3): the
//! neutral model over both adapters, the `forge.db` cache, each account's
//! request budget, and `pr_changed` events. Reads answer from the cache when
//! it is young enough and ask the provider otherwise; a repository that
//! fails keeps its cached list and reports its error beside it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::task::JoinSet;

use super::accounts::AccountService;
use super::adapter::{Client, ForgeAdapter, Session};
use super::bitbucket::Bitbucket;
use super::budget::Budget;
use super::cache;
use super::github::Github;
use super::http::Http;
use super::patch::split_patch;
use super::{Endpoints, ForgeRepository, PullRequestRef};
use crate::db::{Db, RepositoryRow};
use crate::git::{parse_unified_diff, validate_repo_path, GitService};
use crate::models::{
    now_rfc3339, AppError, AppResult, ChangeKind, ChangedFile, ChangedFileStatus, CommitFile,
    Conversation, DiffContent, DiffResult, DiffSelector, DiffSource, ErrorCode, ForgeKind,
    ListPullRequestsRequest, PullRequest, PullRequestChangeOrigin, PullRequestChangedEvent,
    PullRequestChecks, PullRequestDiff, PullRequestDiffRequest, PullRequestFiles, PullRequestGroup,
    PullRequestList,
};
use crate::workspaces::RepositoryService;

/// Workspace lists are read again after this long (SPEC.md, Staying up to date).
pub const LIST_MAX_AGE_SECONDS: u64 = 300;
/// The pull request on screen, and its checks, after this long.
pub const DETAIL_MAX_AGE_SECONDS: u64 = 60;

pub type PullRequestEmitter = Arc<dyn Fn(PullRequestChangedEvent) + Send + Sync>;

pub struct PullRequestService {
    repositories: Arc<RepositoryService>,
    accounts: Arc<AccountService>,
    cache: Db,
    budget: Arc<Budget>,
    github: Github,
    bitbucket: Bitbucket,
    emitter: PullRequestEmitter,
}

/// A tracked repository: its registration and its forge repository.
#[derive(Clone)]
struct Tracked {
    repository_id: String,
    name: String,
    root: PathBuf,
    forge: ForgeRepository,
}

fn tracked(row: &RepositoryRow) -> Option<Tracked> {
    let forge = match &row.forge_override {
        Some(f) => f.clone(),
        None => ForgeRepository::from_remote_url(row.remote_url.as_deref()?)?,
    };
    let root = PathBuf::from(&row.canonical_root);
    Some(Tracked {
        repository_id: row.id.clone(),
        name: root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| row.canonical_root.clone()),
        root,
        forge,
    })
}

/// A file of a local range as the neutral model lists it.
fn changed_file(f: CommitFile) -> ChangedFile {
    ChangedFile {
        status: match f.kind {
            ChangeKind::Added => ChangedFileStatus::Added,
            ChangeKind::Deleted => ChangedFileStatus::Removed,
            ChangeKind::Renamed => ChangedFileStatus::Renamed,
            ChangeKind::Modified => ChangedFileStatus::Modified,
            _ => ChangedFileStatus::Other,
        },
        path: f.path,
        old_path: f.old_path,
        additions: f.additions.unwrap_or(0) as u32,
        deletions: f.deletions.unwrap_or(0) as u32,
        binary: f.is_binary,
    }
}

/// The commits are not on the Mac: what to do about it.
fn not_local(what: &str) -> AppError {
    AppError::new(
        ErrorCode::NotFound,
        format!(
            "{what} are not on this Mac yet. Fetch the repository to compare since your review."
        ),
    )
}

/// Whether `fetched_at` is younger than `max_age` seconds.
fn fresh(fetched_at: &str, max_age: u64) -> bool {
    chrono::DateTime::parse_from_rfc3339(fetched_at)
        .map(|t| {
            (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds() < max_age as i64
        })
        .unwrap_or(false)
}

impl PullRequestService {
    pub fn new(
        repositories: Arc<RepositoryService>,
        accounts: Arc<AccountService>,
        cache: Db,
        http: Http,
        endpoints: Endpoints,
        emitter: PullRequestEmitter,
    ) -> Self {
        let budget = Arc::new(Budget::default());
        let github = Github::new(Client::new(
            ForgeKind::Github,
            endpoints.github,
            http.clone(),
            Arc::clone(&budget),
        ));
        let bitbucket = Bitbucket::new(Client::new(
            ForgeKind::BitbucketCloud,
            endpoints.bitbucket,
            http,
            Arc::clone(&budget),
        ));
        PullRequestService {
            repositories,
            accounts,
            cache,
            budget,
            github,
            bitbucket,
            emitter,
        }
    }

    /// Open `forge.db` in the data folder, creating it again when it cannot be used.
    pub fn open_cache(data_dir: &Path) -> AppResult<Db> {
        let db = Db::open_rebuildable(&data_dir.join(crate::db::FORGE_FILE), &crate::db::FORGE)?;
        let now = now_rfc3339();
        if let Err(e) = db.call_blocking(move |conn| cache::prune(conn, &now)) {
            tracing::warn!(error = %e, "could not prune the pull request cache");
        }
        Ok(db)
    }

    pub fn budgets(&self) -> Vec<crate::models::RequestBudget> {
        self.budget.snapshot()
    }

    /// The provider's session for the account, or `None` without an account.
    async fn session(&self, kind: ForgeKind) -> AppResult<Option<Session>> {
        let Some(account) = self.accounts.account(kind).await? else {
            return Ok(None);
        };
        let token = self.accounts.token(kind).await?;
        Ok(Some(Session {
            token,
            email: account.email.clone(),
            user_id: account.user_id.clone(),
            account,
        }))
    }

    fn emit(&self, pr: &PullRequest) {
        (self.emitter)(PullRequestChangedEvent {
            reference: pr.reference.clone(),
            version: pr.version.clone(),
            origin: PullRequestChangeOrigin::Remote,
        });
    }

    /// Expand Bitbucket's short head commit with local Git when the commit
    /// is on the Mac; otherwise it stays short until a precondition needs it.
    async fn expand_head(&self, root: &Path, pr: &mut PullRequest) {
        if pr.head_sha.len() >= 40 || pr.head_sha.is_empty() {
            return;
        }
        let Some(git) = self.repositories.git_service() else {
            return;
        };
        if let Ok(Some(full)) = git.resolve_commit(root, &pr.head_sha).await {
            if full.starts_with(&pr.head_sha) {
                pr.version = pr
                    .version
                    .strip_suffix(&pr.head_sha)
                    .map(|v| format!("{v}{full}"))
                    .unwrap_or_else(|| pr.version.clone());
                pr.head_sha = full;
            }
        }
    }

    /// Whether both commits are objects of the local repository, without
    /// downloading anything (`git cat-file -e`).
    async fn both_local(&self, root: &Path, a: &str, b: &str) -> bool {
        let Some(git) = self.repositories.git_service() else {
            return false;
        };
        if a.is_empty() || b.is_empty() {
            return false;
        }
        git.object_exists(root, a).await.unwrap_or(false)
            && git.object_exists(root, b).await.unwrap_or(false)
    }

    /// Read one pull request from its provider, with what Bitbucket's list
    /// leaves out when its version changed since `cached`, and how many
    /// commits arrived since the account's last review when local Git can say.
    async fn read_one(
        &self,
        session: &Session,
        root: &Path,
        reference: &PullRequestRef,
        mut pr: PullRequest,
        cached: Option<&PullRequest>,
    ) -> AppResult<PullRequest> {
        self.expand_head(root, &mut pr).await;
        if session.account.kind == ForgeKind::BitbucketCloud {
            match cached {
                Some(c) if c.version == pr.version => {
                    pr.checks = c.checks.clone();
                    pr.counts.additions = c.counts.additions;
                    pr.counts.deletions = c.counts.deletions;
                    pr.counts.changed_files = c.counts.changed_files;
                    pr.counts.unresolved_threads = c.counts.unresolved_threads;
                }
                _ => {
                    let (files, threads) =
                        self.bitbucket.detail(session, &mut pr, reference).await?;
                    let now = now_rfc3339();
                    let files = PullRequestFiles {
                        reference: pr.reference.clone(),
                        head_sha: pr.head_sha.clone(),
                        base_sha: pr.base_sha.clone(),
                        since_review: false,
                        files,
                        fetched_at: now.clone(),
                    };
                    let conversation = Conversation {
                        reference: pr.reference.clone(),
                        threads,
                        fetched_at: now,
                    };
                    self.cache
                        .call(move |conn| {
                            cache::put_files(conn, &files)?;
                            cache::put_conversation(conn, &conversation)
                        })
                        .await?;
                }
            }
        }
        if let Some(reviewed) = pr.reviewed_sha.clone() {
            if reviewed != pr.head_sha && self.both_local(root, &reviewed, &pr.head_sha).await {
                if let Some(git) = self.repositories.git_service() {
                    pr.commits_since_review = git
                        .count_commits(root, &pr.head_sha, &[reviewed.as_str()])
                        .await
                        .ok();
                }
            }
        }
        Ok(pr)
    }

    /// Read a repository's list anew and store it. Returns how many changed.
    async fn refresh_list(
        &self,
        session: &Session,
        tracked: &Tracked,
        closed: bool,
    ) -> AppResult<usize> {
        let forge = tracked.forge.to_string();
        let read = {
            let forge = forge.clone();
            self.cache
                .call(move |conn| cache::list_read(conn, &forge, closed))
                .await?
        };
        let outcome = match session.account.kind {
            ForgeKind::Github => {
                self.github
                    .list(
                        session,
                        &tracked.forge,
                        closed,
                        read.as_ref().and_then(|r| r.etag.as_deref()),
                    )
                    .await?
            }
            ForgeKind::BitbucketCloud => {
                self.bitbucket
                    .list(
                        session,
                        &tracked.forge,
                        closed,
                        read.as_ref().and_then(|r| r.etag.as_deref()),
                    )
                    .await?
            }
        };
        let now = now_rfc3339();
        if outcome.not_modified {
            let forge = forge.clone();
            self.cache
                .call(move |conn| {
                    cache::set_list_read(conn, &forge, closed, outcome.etag.as_deref(), &now)
                })
                .await?;
            return Ok(0);
        }
        let cached: HashMap<String, PullRequest> = {
            let forge = forge.clone();
            self.cache
                .call(move |conn| cache::list(conn, &forge, closed))
                .await?
                .into_iter()
                .map(|p| (p.reference.clone(), p))
                .collect()
        };
        let mut changed = Vec::new();
        let mut all = Vec::with_capacity(outcome.pull_requests.len());
        for pr in outcome.pull_requests {
            let reference = PullRequestRef {
                repository: tracked.forge.clone(),
                number: pr.number,
            };
            let before = cached.get(&pr.reference);
            let pr = self
                .read_one(session, &tracked.root, &reference, pr, before)
                .await?;
            if before.is_none_or(|b| b.version != pr.version) {
                changed.push(pr.clone());
            }
            all.push(pr);
        }
        let references: Vec<String> = all.iter().map(|p| p.reference.clone()).collect();
        let etag = outcome.etag.clone();
        let stored = all.clone();
        let forge2 = forge.clone();
        self.cache
            .call(move |conn| {
                for pr in &stored {
                    cache::put(conn, &forge2, pr, &now)?;
                }
                if !closed {
                    cache::retain_open(conn, &forge2, &references)?;
                }
                cache::set_list_read(conn, &forge2, closed, etag.as_deref(), &now)
            })
            .await?;
        for pr in &changed {
            self.emit(pr);
        }
        Ok(changed.len())
    }

    /// A repository's group for a list: refresh when stale, then what is cached.
    async fn group(
        self: Arc<Self>,
        tracked: Tracked,
        closed: bool,
        max_age: u64,
    ) -> PullRequestGroup {
        let kind = tracked.forge.kind;
        let forge = tracked.forge.to_string();
        let mut error = None;
        let read = {
            let forge = forge.clone();
            self.cache
                .call(move |conn| cache::list_read(conn, &forge, closed))
                .await
                .ok()
                .flatten()
        };
        let stale = read.as_ref().is_none_or(|r| !fresh(&r.fetched_at, max_age));
        if stale {
            match self.session(kind).await {
                Ok(Some(session)) => {
                    if let Err(e) = self.refresh_list(&session, &tracked, closed).await {
                        error = Some(e);
                    }
                }
                Ok(None) => {
                    error = Some(AppError::new(
                        crate::models::ErrorCode::PermissionDenied,
                        format!(
                            "No {} account. Add one in Settings → Accounts.",
                            kind.label()
                        ),
                    ));
                }
                Err(e) => error = Some(e),
            }
        }
        let (pull_requests, fetched_at) = {
            let forge = forge.clone();
            self.cache
                .call(move |conn| {
                    Ok((
                        cache::list(conn, &forge, closed)?,
                        cache::list_read(conn, &forge, closed)?.map(|r| r.fetched_at),
                    ))
                })
                .await
                .unwrap_or_else(|e| {
                    error.get_or_insert(e);
                    (Vec::new(), None)
                })
        };
        PullRequestGroup {
            repository_id: tracked.repository_id,
            repository_name: tracked.name,
            forge,
            kind,
            pull_requests,
            fetched_at,
            error,
        }
    }

    /// The pull requests of a workspace's repositories, or of one repository.
    pub async fn list(
        self: &Arc<Self>,
        request: ListPullRequestsRequest,
    ) -> AppResult<PullRequestList> {
        let (enabled, tracked_by, rows) = match (&request.workspace_id, &request.repository_id) {
            (Some(workspace_id), _) => {
                let ws = self.repositories.workspace(workspace_id).await?;
                let mut rows = Vec::new();
                for id in ws.members.iter().filter_map(|m| m.repository_id.as_ref()) {
                    if let Ok(row) = self.repositories.row(id).await {
                        rows.push(row);
                    }
                }
                (ws.pull_requests, Vec::new(), rows)
            }
            (None, Some(repository_id)) => {
                let row = self.repositories.row(repository_id).await?;
                let tracked_by: Vec<String> = self
                    .repositories
                    .workspaces()
                    .await?
                    .into_iter()
                    .filter(|w| {
                        w.pull_requests
                            && w.members
                                .iter()
                                .any(|m| m.repository_id.as_deref() == Some(repository_id))
                    })
                    .map(|w| w.name)
                    .collect();
                (!tracked_by.is_empty(), tracked_by, vec![row])
            }
            (None, None) => {
                return Err(AppError::validation("Name a workspace or a repository."));
            }
        };
        if request.closed && request.workspace_id.is_some() {
            return Err(AppError::validation(
                "Merged and closed pull requests are listed for one repository at a time.",
            ));
        }
        let mut list = PullRequestList {
            enabled,
            tracked_by,
            missing_accounts: Vec::new(),
            groups: Vec::new(),
            budgets: self.budgets(),
        };
        if !enabled {
            return Ok(list);
        }
        let mut set = JoinSet::new();
        let mut order = Vec::new();
        for row in &rows {
            let Some(t) = tracked(row) else { continue };
            if self.accounts.account(t.forge.kind).await?.is_none()
                && !list.missing_accounts.contains(&t.forge.kind)
            {
                list.missing_accounts.push(t.forge.kind);
            }
            order.push(t.repository_id.clone());
            let service = Arc::clone(self);
            set.spawn(service.group(t, request.closed, request.max_age_seconds));
        }
        let mut groups: Vec<PullRequestGroup> = Vec::new();
        while let Some(result) = set.join_next().await {
            match result {
                Ok(group) => groups.push(group),
                Err(e) => tracing::warn!(error = %e, "a pull request list task failed"),
            }
        }
        groups.sort_by_key(|g| order.iter().position(|id| *id == g.repository_id));
        list.groups = groups;
        list.budgets = self.budgets();
        Ok(list)
    }

    /// The repository registration a pull request belongs to, through the
    /// tracked repositories; its local checkout expands short commits.
    async fn tracked_for(&self, reference: &PullRequestRef) -> AppResult<Tracked> {
        let rows = self.repositories.tracked_forges().await?;
        rows.iter()
            .find(|(_, f)| *f == reference.repository)
            .and_then(|(row, _)| tracked(row))
            .ok_or_else(|| {
                AppError::not_found(format!(
                    "No workspace tracks the pull requests of {}.",
                    reference.repository
                ))
            })
    }

    /// One pull request: cached when younger than `max_age` seconds, read anew otherwise.
    pub async fn get(&self, reference: &str, max_age: u64) -> AppResult<PullRequest> {
        let parsed: PullRequestRef = reference.parse()?;
        let cached = {
            let reference = reference.to_string();
            self.cache
                .call(move |conn| cache::get(conn, &reference))
                .await?
        };
        if let Some((pr, fetched_at)) = &cached {
            if fresh(fetched_at, max_age) {
                return Ok(pr.clone());
            }
        }
        let tracked = self.tracked_for(&parsed).await?;
        let kind = parsed.repository.kind;
        let session = self.session(kind).await?.ok_or_else(|| {
            AppError::new(
                crate::models::ErrorCode::PermissionDenied,
                format!(
                    "No {} account. Add one in Settings → Accounts.",
                    kind.label()
                ),
            )
        })?;
        let read = match kind {
            ForgeKind::Github => self.github.get(&session, &parsed).await,
            ForgeKind::BitbucketCloud => self.bitbucket.get(&session, &parsed).await,
        };
        let pr = match read {
            Ok(pr) => pr,
            // Keep showing what was cached when the provider is unreachable.
            Err(e) if e.retryable && cached.is_some() => {
                tracing::warn!(reference, error = %e, "showing the cached pull request");
                return Ok(cached.map(|(pr, _)| pr).expect("cached"));
            }
            Err(e) => return Err(e),
        };
        let before = cached.as_ref().map(|(pr, _)| pr);
        let pr = self
            .read_one(&session, &tracked.root, &parsed, pr, before)
            .await?;
        let changed = before.is_none_or(|b| b.version != pr.version);
        let stored = pr.clone();
        let forge = parsed.repository.to_string();
        let now = now_rfc3339();
        self.cache
            .call(move |conn| cache::put(conn, &forge, &stored, &now))
            .await?;
        if changed {
            self.emit(&pr);
        }
        Ok(pr)
    }

    fn git(&self) -> AppResult<&GitService> {
        self.repositories
            .git_service()
            .ok_or_else(|| AppError::dependency("Git was not found on this Mac."))
    }

    /// The files a pull request changes, for its current head; or, since
    /// the account's last review, from local Git when the commits are here.
    pub async fn files(&self, reference: &str, since_review: bool) -> AppResult<PullRequestFiles> {
        let pr = self.get(reference, LIST_MAX_AGE_SECONDS).await?;
        if since_review {
            let parsed: PullRequestRef = reference.parse()?;
            let reviewed = pr
                .reviewed_sha
                .clone()
                .ok_or_else(|| AppError::validation("You have not reviewed this pull request."))?;
            let tracked = self.tracked_for(&parsed).await?;
            if !self
                .both_local(&tracked.root, &reviewed, &pr.head_sha)
                .await
            {
                return Err(not_local("The new commits"));
            }
            let files = self
                .git()?
                .range_files(&tracked.root, &reviewed, &pr.head_sha)
                .await?;
            return Ok(PullRequestFiles {
                reference: reference.to_string(),
                head_sha: pr.head_sha,
                base_sha: reviewed,
                since_review: true,
                files: files.into_iter().map(changed_file).collect(),
                fetched_at: now_rfc3339(),
            });
        }
        let cached = {
            let reference = reference.to_string();
            self.cache
                .call(move |conn| cache::files(conn, &reference))
                .await?
        };
        if let Some(files) = cached {
            if files.head_sha == pr.head_sha {
                return Ok(files);
            }
        }
        let parsed: PullRequestRef = reference.parse()?;
        let session = self.session_for(parsed.repository.kind).await?;
        let files = match parsed.repository.kind {
            ForgeKind::Github => self.github.files(&session, &parsed).await?,
            ForgeKind::BitbucketCloud => self.bitbucket.files(&session, &parsed).await?,
        };
        let files = PullRequestFiles {
            reference: reference.to_string(),
            head_sha: pr.head_sha,
            base_sha: pr.base_sha,
            since_review: false,
            files,
            fetched_at: now_rfc3339(),
        };
        let stored = files.clone();
        self.cache
            .call(move |conn| cache::put_files(conn, &stored))
            .await?;
        Ok(files)
    }

    /// The conversation: cached when younger than `max_age` seconds, read
    /// anew otherwise; what was cached when the provider is unreachable.
    pub async fn conversation(&self, reference: &str, max_age: u64) -> AppResult<Conversation> {
        let parsed: PullRequestRef = reference.parse()?;
        let cached = {
            let reference = reference.to_string();
            self.cache
                .call(move |conn| cache::conversation(conn, &reference))
                .await?
        };
        if let Some(c) = &cached {
            if fresh(&c.fetched_at, max_age) {
                return Ok(c.clone());
            }
        }
        let kind = parsed.repository.kind;
        let session = self.session_for(kind).await?;
        let read = match kind {
            ForgeKind::Github => self.github.conversation(&session, &parsed).await,
            ForgeKind::BitbucketCloud => self.bitbucket.conversation(&session, &parsed).await,
        };
        let threads = match read {
            Ok(threads) => threads,
            Err(e) if e.retryable && cached.is_some() => {
                tracing::warn!(reference, error = %e, "showing the cached conversation");
                return Ok(cached.expect("cached"));
            }
            Err(e) => return Err(e),
        };
        let conversation = Conversation {
            reference: reference.to_string(),
            threads,
            fetched_at: now_rfc3339(),
        };
        let stored = conversation.clone();
        self.cache
            .call(move |conn| cache::put_conversation(conn, &stored))
            .await?;
        Ok(conversation)
    }

    /// One file's diff: from local Git when both commits are on the Mac
    /// (`base...head`, never downloading), otherwise the provider's patch.
    /// Since the last review, only local Git can answer.
    pub async fn diff(&self, request: PullRequestDiffRequest) -> AppResult<PullRequestDiff> {
        validate_repo_path(&request.path)?;
        let pr = self.get(&request.reference, LIST_MAX_AGE_SECONDS).await?;
        let parsed: PullRequestRef = request.reference.parse()?;
        let tracked = self.tracked_for(&parsed).await?;
        let base = if request.since_review {
            pr.reviewed_sha
                .clone()
                .ok_or_else(|| AppError::validation("You have not reviewed this pull request."))?
        } else {
            pr.base_sha.clone()
        };
        let head = pr.head_sha.clone();
        let selector = DiffSelector::Range {
            base: base.clone(),
            head: head.clone(),
            path: request.path.clone(),
            old_path: request.old_path.clone(),
        };
        let limits = self.repositories.settings().diff_limits;
        if self.both_local(&tracked.root, &base, &head).await {
            match self
                .git()?
                .diff(&tracked.root, &selector, &limits, request.options)
                .await
            {
                Ok(content) => {
                    return Ok(PullRequestDiff {
                        reference: request.reference,
                        source: DiffSource::Local,
                        base_sha: base,
                        head_sha: head,
                        diff: DiffResult {
                            repository_id: tracked.repository_id,
                            selector,
                            content,
                        },
                    })
                }
                // No merge base, for one: the provider's patch is the answer.
                Err(e) => tracing::debug!(error = %e, "local diff failed; asking the provider"),
            }
        }
        if request.since_review {
            return Err(not_local("The new commits"));
        }
        let patch = self
            .provider_patch(&parsed, &pr, &request.path, request.old_path.as_deref())
            .await?;
        let content = match patch {
            Some(text) => parse_unified_diff(text.as_bytes(), false, &limits),
            None => DiffContent::Text {
                old_path: request.old_path.clone(),
                new_path: Some(request.path.clone()),
                hunks: Vec::new(),
                truncated: false,
                total_lines: Some(0),
            },
        };
        Ok(PullRequestDiff {
            reference: request.reference,
            source: DiffSource::Provider,
            base_sha: base,
            head_sha: head,
            diff: DiffResult {
                repository_id: tracked.repository_id,
                selector,
                content,
            },
        })
    }

    /// One file's part of the provider's diff, read whole once per head and
    /// kept in the cache.
    async fn provider_patch(
        &self,
        parsed: &PullRequestRef,
        pr: &PullRequest,
        path: &str,
        old_path: Option<&str>,
    ) -> AppResult<Option<String>> {
        let reference = pr.reference.clone();
        let stored = {
            let reference = reference.clone();
            self.cache
                .call(move |conn| cache::patch_set(conn, &reference))
                .await?
        };
        if stored.as_deref() != Some(pr.head_sha.as_str()) {
            let session = self.session_for(parsed.repository.kind).await?;
            let text = match parsed.repository.kind {
                ForgeKind::Github => self.github.patch(&session, parsed).await?,
                ForgeKind::BitbucketCloud => self.bitbucket.patch(&session, parsed).await?,
            };
            let files = split_patch(&text);
            let head = pr.head_sha.clone();
            let reference = reference.clone();
            let now = now_rfc3339();
            self.cache
                .call(move |conn| cache::put_patch_set(conn, &reference, &head, &files, &now))
                .await?;
        }
        let path = path.to_string();
        let old = old_path.map(str::to_string);
        self.cache
            .call(move |conn| match cache::patch(conn, &reference, &path)? {
                Some(p) => Ok(Some(p)),
                None => match old {
                    Some(old) => cache::patch(conn, &reference, &old),
                    None => Ok(None),
                },
            })
            .await
    }

    /// The checks of a pull request's head, read again after `max_age` seconds.
    pub async fn checks(&self, reference: &str, max_age: u64) -> AppResult<PullRequestChecks> {
        let pr = self.get(reference, max_age).await?;
        let cached = {
            let reference = reference.to_string();
            self.cache
                .call(move |conn| cache::checks(conn, &reference))
                .await?
        };
        if let Some(checks) = cached {
            if checks.head_sha == pr.head_sha && fresh(&checks.fetched_at, max_age) {
                return Ok(checks);
            }
        }
        let parsed: PullRequestRef = reference.parse()?;
        let session = self.session_for(parsed.repository.kind).await?;
        let checks = match parsed.repository.kind {
            ForgeKind::Github => self.github.checks(&session, &parsed, &pr.head_sha).await?,
            ForgeKind::BitbucketCloud => {
                self.bitbucket
                    .checks(&session, &parsed, &pr.head_sha)
                    .await?
            }
        };
        let checks = PullRequestChecks {
            reference: reference.to_string(),
            head_sha: pr.head_sha,
            checks,
            fetched_at: now_rfc3339(),
        };
        let stored = checks.clone();
        self.cache
            .call(move |conn| cache::put_checks(conn, &stored))
            .await?;
        Ok(checks)
    }

    async fn session_for(&self, kind: ForgeKind) -> AppResult<Session> {
        self.session(kind).await?.ok_or_else(|| {
            AppError::new(
                crate::models::ErrorCode::PermissionDenied,
                format!(
                    "No {} account. Add one in Settings → Accounts.",
                    kind.label()
                ),
            )
        })
    }

    /// Read again the lists that are older than five minutes, for every
    /// tracked repository. Run every minute while Brainiac is open.
    pub async fn sync_tick(self: &Arc<Self>) {
        let rows = match self.repositories.tracked_forges().await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "cannot list the tracked repositories");
                return;
            }
        };
        for (row, _) in rows {
            let Some(t) = tracked(&row) else { continue };
            let group = Arc::clone(self).group(t, false, LIST_MAX_AGE_SECONDS).await;
            if let Some(e) = group.error {
                tracing::debug!(repository = group.forge, error = %e, "pull request sync");
            }
        }
    }
}
