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
use super::adapter::{
    find_posted, own_branch, Client, ForgeAdapter, ReviewToSend, SentPart, Session,
    BITBUCKET_WRITE, GITHUB_CONTENTS_WRITE, GITHUB_PULL_REQUESTS_WRITE,
};
use super::bitbucket::Bitbucket;
use super::budget::Budget;
use super::cache;
use super::drafts;
use super::github::Github;
use super::http::Http;
use super::patch::split_patch;
use super::{Endpoints, ForgeRepository, PullRequestRef};
use crate::db::{Db, RepositoryRow};
use crate::git::{parse_unified_diff, validate_repo_path, GitService};
use crate::models::{
    now_rfc3339, AppError, AppResult, ChangeKind, ChangedFile, ChangedFileStatus, CommentRequest,
    CommitFile, Conversation, DiffContent, DiffResult, DiffSelector, DiffSource, ErrorCode,
    FetchResult, ForgeKind, ListPullRequestsRequest, MergeOptions, MergeOutcome, MergeRequest,
    PullRequest, PullRequestChangeOrigin, PullRequestChangedEvent, PullRequestChecks,
    PullRequestDiff, PullRequestDiffRequest, PullRequestFiles, PullRequestGroup, PullRequestList,
    PullRequestState, ReplyRequest, RepositorySummary, ResolveThreadRequest, ReviewCount,
    ReviewDrafts, ReviewVerdict, SaveReviewDraftRequest, SubmitReviewRequest, Thread, WriteOutcome,
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
    /// `brainiac.db`, for the review drafts: the user's own text.
    core: Db,
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
            "{what} are not on this Mac yet. Fetch the repository to compare from that commit."
        ),
    )
}

/// A commit named by a caller: hexadecimal, so it can go on a Git command
/// line and in a URL as it is.
fn validate_sha(sha: &str) -> AppResult<()> {
    let ok = (7..=64).contains(&sha.len()) && sha.chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(AppError::validation(format!("{sha:?} is not a commit.")))
    }
}

/// Whether `head` is the commit `expected`, which may be Bitbucket's short form.
fn same_commit(head: &str, expected: &str) -> bool {
    !expected.is_empty()
        && (head == expected || head.starts_with(expected) || expected.starts_with(head))
}

/// The head moved since the user looked: nothing is sent (SPEC.md, Reviewing).
fn head_moved(what: &str) -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        format!("New commits arrived since you {what}. Nothing was sent; look at the new changes first."),
    )
}

/// The permission a provider's refusal of a write means the token lacks.
fn permission_for(kind: ForgeKind, contents: bool) -> &'static str {
    match (kind, contents) {
        (ForgeKind::Github, false) => GITHUB_PULL_REQUESTS_WRITE,
        (ForgeKind::Github, true) => GITHUB_CONTENTS_WRITE,
        (ForgeKind::BitbucketCloud, _) => BITBUCKET_WRITE,
    }
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
        core: Db,
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
            core,
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
                        partial: false,
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

    /// The files a pull request changes, for its current head; or, since a
    /// commit of it (the one last reviewed, or the one drafts were written
    /// on), from local Git when both commits are here.
    pub async fn files(&self, reference: &str, since: Option<&str>) -> AppResult<PullRequestFiles> {
        let pr = self.get(reference, LIST_MAX_AGE_SECONDS).await?;
        if let Some(since) = since {
            validate_sha(since)?;
            let parsed: PullRequestRef = reference.parse()?;
            let tracked = self.tracked_for(&parsed).await?;
            if !self.both_local(&tracked.root, since, &pr.head_sha).await {
                return Err(not_local("The new commits"));
            }
            let files = self
                .git()?
                .range_files(&tracked.root, since, &pr.head_sha)
                .await?;
            return Ok(PullRequestFiles {
                reference: reference.to_string(),
                head_sha: pr.head_sha,
                base_sha: since.to_string(),
                partial: true,
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
            partial: false,
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
    /// Since a commit of the pull request, only local Git can answer.
    pub async fn diff(&self, request: PullRequestDiffRequest) -> AppResult<PullRequestDiff> {
        validate_repo_path(&request.path)?;
        let pr = self.get(&request.reference, LIST_MAX_AGE_SECONDS).await?;
        let parsed: PullRequestRef = request.reference.parse()?;
        let tracked = self.tracked_for(&parsed).await?;
        let base = match &request.since {
            Some(since) => {
                validate_sha(since)?;
                since.clone()
            }
            None => pr.base_sha.clone(),
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
        if request.since.is_some() {
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

    /// The local checkout a pull request's repository is, for the side panel.
    pub async fn repository(&self, reference: &str) -> AppResult<RepositorySummary> {
        let parsed: PullRequestRef = reference.parse()?;
        let tracked = self.tracked_for(&parsed).await?;
        self.repositories.summary(&tracked.repository_id).await
    }

    // --- Review drafts (SPEC.md, Reviewing) -----------------------------------

    /// A pull request's drafts and any submission under way.
    pub async fn drafts(&self, reference: &str) -> AppResult<ReviewDrafts> {
        let reference = reference.to_string();
        self.core
            .call(move |conn| drafts::list(conn, &reference))
            .await
    }

    /// Write a draft on a line, or change one. The anchor's commit is the
    /// head the diff was shown for, so **Finish Review** can tell whether
    /// new commits arrived since.
    pub async fn save_draft(&self, request: SaveReviewDraftRequest) -> AppResult<ReviewDrafts> {
        let parsed: PullRequestRef = request.reference.parse()?;
        validate_repo_path(&request.anchor.path)?;
        if request.body.trim().is_empty() {
            return Err(AppError::validation("Write the comment first."));
        }
        if request.anchor.line.is_none() {
            return Err(AppError::validation("A draft goes on a line."));
        }
        if let Some(commit) = &request.anchor.commit {
            validate_sha(commit)?;
        }
        let pr = self.get(&request.reference, LIST_MAX_AGE_SECONDS).await?;
        if !pr.actions.review.allowed {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                pr.actions.review.reason.unwrap_or_default(),
            ));
        }
        let reference = parsed.to_string();
        let now = now_rfc3339();
        self.core
            .call(move |conn| {
                let id = match request.id {
                    Some(id) if drafts::exists(conn, &reference, &id)? => id,
                    Some(_) => return Err(AppError::not_found("The draft is gone.")),
                    None => uuid::Uuid::new_v4().to_string(),
                };
                drafts::save(conn, &reference, &id, &request.anchor, &request.body, &now)?;
                drafts::list(conn, &reference)
            })
            .await
    }

    pub async fn delete_draft(&self, reference: &str, id: &str) -> AppResult<ReviewDrafts> {
        let reference = reference.to_string();
        let id = id.to_string();
        self.core
            .call(move |conn| {
                drafts::delete(conn, &reference, &id)?;
                drafts::list(conn, &reference)
            })
            .await
    }

    /// The user looked at the new commits: the drafts go on the head now.
    pub async fn move_drafts(&self, reference: &str, head_sha: &str) -> AppResult<ReviewDrafts> {
        validate_sha(head_sha)?;
        let reference = reference.to_string();
        let head = head_sha.to_string();
        let now = now_rfc3339();
        self.core
            .call(move |conn| {
                drafts::move_to(conn, &reference, &head, &now)?;
                drafts::list(conn, &reference)
            })
            .await
    }

    // --- Writes to the provider (SPEC.md, Reviewing) ----------------------------

    /// What every write ends with: the conversation and the pull request
    /// read again, stored, and announced as a change made from Brainiac.
    async fn after_write(&self, reference: &str) -> AppResult<WriteOutcome> {
        let conversation = self.conversation(reference, 0).await?;
        let pull_request = self.get(reference, 0).await?;
        (self.emitter)(PullRequestChangedEvent {
            reference: reference.to_string(),
            version: pull_request.version.clone(),
            origin: PullRequestChangeOrigin::App,
        });
        Ok(WriteOutcome {
            pull_request,
            conversation,
        })
    }

    /// A refusal for a permission the token lacks turns that action off
    /// for the account until the token is replaced (SPEC.md, Accounts).
    async fn note_refusal(&self, kind: ForgeKind, contents: bool, e: &AppError) {
        if e.code == ErrorCode::PermissionDenied && e.message.contains("The token needs") {
            let permission = permission_for(kind, contents);
            if let Err(e) = self.accounts.mark_missing(kind, permission).await {
                tracing::warn!(error = %e, "could not record the missing permission");
            }
        }
    }

    /// A write that got no answer in time may have been applied: read the
    /// conversation again and look for it before saying so (no blind retries).
    async fn posted_anyway(
        &self,
        reference: &str,
        path: Option<&str>,
        thread_id: Option<&str>,
        body: &str,
    ) -> bool {
        match self.conversation(reference, 0).await {
            Ok(c) => find_posted(&c.threads, path, thread_id, body).is_some(),
            Err(_) => false,
        }
    }

    fn timed_out(provider: &str, what: &str) -> AppError {
        AppError::new(
            ErrorCode::Timeout,
            format!("{provider} did not answer in time, and the {what} is not on {provider}. Try again."),
        )
    }

    /// Comment on the whole pull request; posted at once.
    pub async fn comment(&self, request: CommentRequest) -> AppResult<WriteOutcome> {
        let parsed: PullRequestRef = request.reference.parse()?;
        let body = request.body.trim().to_string();
        if body.is_empty() {
            return Err(AppError::validation("Write the comment first."));
        }
        let pr = self.get(&request.reference, LIST_MAX_AGE_SECONDS).await?;
        if !pr.actions.comment.allowed {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                pr.actions.comment.reason.unwrap_or_default(),
            ));
        }
        let kind = parsed.repository.kind;
        let session = self.session_for(kind).await?;
        let result = match kind {
            ForgeKind::Github => self.github.comment(&session, &parsed, &body).await,
            ForgeKind::BitbucketCloud => self.bitbucket.comment(&session, &parsed, &body).await,
        };
        match result {
            Ok(_) => {}
            Err(e) if e.code == ErrorCode::Timeout => {
                if !self
                    .posted_anyway(&request.reference, None, None, &body)
                    .await
                {
                    return Err(Self::timed_out(kind.label(), "comment"));
                }
            }
            Err(e) => {
                self.note_refusal(kind, false, &e).await;
                return Err(e);
            }
        }
        self.after_write(&request.reference).await
    }

    /// The thread, from the conversation as cached or read.
    async fn thread(&self, reference: &str, thread_id: &str) -> AppResult<Thread> {
        let conversation = self.conversation(reference, LIST_MAX_AGE_SECONDS).await?;
        conversation
            .threads
            .into_iter()
            .find(|t| t.id == thread_id)
            .ok_or_else(|| AppError::not_found("The thread is gone. Read the pull request again."))
    }

    /// Reply to a thread; posted at once.
    pub async fn reply(&self, request: ReplyRequest) -> AppResult<WriteOutcome> {
        let parsed: PullRequestRef = request.reference.parse()?;
        let body = request.body.trim().to_string();
        if body.is_empty() {
            return Err(AppError::validation("Write the reply first."));
        }
        let pr = self.get(&request.reference, LIST_MAX_AGE_SECONDS).await?;
        if !pr.actions.comment.allowed {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                pr.actions.comment.reason.unwrap_or_default(),
            ));
        }
        let thread = self.thread(&request.reference, &request.thread_id).await?;
        let kind = parsed.repository.kind;
        let session = self.session_for(kind).await?;
        let result = match kind {
            ForgeKind::Github => self.github.reply(&session, &parsed, &thread, &body).await,
            ForgeKind::BitbucketCloud => {
                self.bitbucket
                    .reply(&session, &parsed, &thread, &body)
                    .await
            }
        };
        match result {
            Ok(_) => {}
            Err(e) if e.code == ErrorCode::Timeout => {
                // A reply to a comment on the pull request is another such
                // comment on GitHub, so it is looked for anywhere.
                let in_thread = thread
                    .anchor
                    .is_some()
                    .then_some(request.thread_id.as_str());
                if !self
                    .posted_anyway(&request.reference, None, in_thread, &body)
                    .await
                {
                    return Err(Self::timed_out(kind.label(), "reply"));
                }
            }
            Err(e) => {
                self.note_refusal(kind, false, &e).await;
                return Err(e);
            }
        }
        self.after_write(&request.reference).await
    }

    /// Resolve or reopen a thread on a line.
    pub async fn resolve(&self, request: ResolveThreadRequest) -> AppResult<WriteOutcome> {
        let parsed: PullRequestRef = request.reference.parse()?;
        let pr = self.get(&request.reference, LIST_MAX_AGE_SECONDS).await?;
        if !pr.actions.resolve.allowed {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                pr.actions.resolve.reason.unwrap_or_default(),
            ));
        }
        let thread = self.thread(&request.reference, &request.thread_id).await?;
        if thread.resolved == request.resolved {
            return self.after_write(&request.reference).await;
        }
        let kind = parsed.repository.kind;
        let session = self.session_for(kind).await?;
        let result = match kind {
            ForgeKind::Github => {
                self.github
                    .resolve(&session, &parsed, &thread, request.resolved)
                    .await
            }
            ForgeKind::BitbucketCloud => {
                self.bitbucket
                    .resolve(&session, &parsed, &thread, request.resolved)
                    .await
            }
        };
        match result {
            Ok(()) => {}
            Err(e) if e.code == ErrorCode::Timeout => {
                let applied = self
                    .conversation(&request.reference, 0)
                    .await
                    .ok()
                    .and_then(|c| c.threads.into_iter().find(|t| t.id == request.thread_id))
                    .is_some_and(|t| t.resolved == request.resolved);
                if !applied {
                    return Err(Self::timed_out(kind.label(), "change"));
                }
            }
            Err(e) => {
                self.note_refusal(kind, true, &e).await;
                return Err(e);
            }
        }
        self.after_write(&request.reference).await
    }

    /// **Finish Review**: the drafts, a summary, and a verdict, for the head
    /// the user looked at. The head is read again first; one that moved, or
    /// drafts written on an earlier commit, send nothing (`CONFLICT`). On
    /// Bitbucket each part is recorded as it lands, so a submission cut off
    /// midway resumes with what is left.
    pub async fn submit_review(&self, request: SubmitReviewRequest) -> AppResult<WriteOutcome> {
        let parsed: PullRequestRef = request.reference.parse()?;
        let reference = parsed.to_string();
        let kind = parsed.repository.kind;
        let body = request.body.trim().to_string();
        let current = self.drafts(&reference).await?;
        let drafts = current.drafts;
        let pending = current.pending;
        // A summary already posted is not posted again, with its first words.
        let (body, summary_sent) = match &pending {
            Some(p) if p.summary_sent => (p.body.clone(), true),
            _ => (body, false),
        };
        if body.is_empty() && drafts.is_empty() && request.verdict == ReviewVerdict::Comment {
            return Err(AppError::validation(
                "Write a summary or a comment on a line first.",
            ));
        }
        let pr = self.get(&reference, 0).await?;
        if !pr.actions.review.allowed {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                pr.actions.review.reason.unwrap_or_default(),
            ));
        }
        if request.verdict == ReviewVerdict::Approve && !pr.actions.approve.allowed {
            return Err(AppError::validation(
                pr.actions.approve.reason.unwrap_or_default(),
            ));
        }
        let session = self.session_for(kind).await?;
        let head = match kind {
            ForgeKind::Github => self.github.current_head(&session, &pr).await?,
            ForgeKind::BitbucketCloud => self.bitbucket.current_head(&session, &pr).await?,
        };
        if !same_commit(&head, &request.expected_head_sha) {
            // What the user sees is behind: read the pull request again.
            if let Ok(fresh) = self.get(&reference, 0).await {
                self.emit(&fresh);
            }
            return Err(head_moved("looked at this pull request"));
        }
        if drafts.iter().filter(|d| d.remote_id.is_none()).any(|d| {
            !d.anchor
                .commit
                .as_deref()
                .is_some_and(|c| same_commit(&head, c))
        }) {
            return Err(head_moved("started this review"));
        }

        let now = now_rfc3339();
        {
            let (reference, body, head) = (reference.clone(), body.clone(), head.clone());
            let verdict = request.verdict;
            self.core
                .call(move |conn| {
                    drafts::set_pending(conn, &reference, &body, verdict, &head, &now)
                })
                .await?;
        }
        let review = ReviewToSend {
            drafts: &drafts,
            body: &body,
            summary_sent,
            verdict: request.verdict,
            head_sha: &head,
        };
        // Each part Bitbucket posts is recorded at once, in case the next
        // one never answers. `Arc<Mutex<_>>`: the callback and this task
        // both reach the list, so it is shared and locked for each push.
        let sent: Arc<std::sync::Mutex<Vec<SentPart>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&sent);
        let mut progress = move |part: SentPart| sink.lock().expect("sent parts").push(part);
        let result = match kind {
            ForgeKind::Github => {
                self.github
                    .submit_review(&session, &pr, &review, &mut progress)
                    .await
            }
            ForgeKind::BitbucketCloud => {
                self.bitbucket
                    .submit_review(&session, &pr, &review, &mut progress)
                    .await
            }
        };
        let parts = std::mem::take(&mut *sent.lock().expect("sent parts"));
        self.record_sent(&reference, parts).await?;

        match result {
            Ok(()) => {}
            Err(e) if e.code == ErrorCode::Timeout => {
                // What landed before the answer was lost, matched against the
                // conversation read again; the rest is offered again.
                let matched = self.match_sent(&reference, &drafts, &body).await;
                let whole = kind == ForgeKind::Github && matched;
                if !whole {
                    return Err(AppError::new(
                        ErrorCode::Timeout,
                        format!(
                            "{} did not answer in time. What it received is marked as sent; send the rest when you are ready.",
                            kind.label()
                        ),
                    ));
                }
            }
            Err(e) => {
                self.note_refusal(kind, false, &e).await;
                return Err(e);
            }
        }
        {
            let reference = reference.clone();
            self.core
                .call(move |conn| drafts::clear(conn, &reference))
                .await?;
        }
        self.after_write(&reference).await
    }

    // --- Merging (SPEC.md, Merging) ---

    /// What the Merge confirmation offers, read from the provider.
    pub async fn merge_options(&self, reference: &str) -> AppResult<MergeOptions> {
        let parsed: PullRequestRef = reference.parse()?;
        let kind = parsed.repository.kind;
        let pr = self.get(reference, LIST_MAX_AGE_SECONDS).await?;
        let session = self.session_for(kind).await?;
        match kind {
            ForgeKind::Github => self.github.merge_options(&session, &pr).await,
            ForgeKind::BitbucketCloud => self.bitbucket.merge_options(&session, &pr).await,
        }
    }

    /// Merge, for the head the user looked at. The branch's tip is read
    /// again first; nothing is merged when it moved. An answer that never
    /// came is checked against the pull request read again: merged counts.
    pub async fn merge(&self, request: MergeRequest) -> AppResult<MergeOutcome> {
        let parsed: PullRequestRef = request.reference.parse()?;
        let reference = parsed.to_string();
        let kind = parsed.repository.kind;
        validate_sha(&request.expected_head_sha)?;
        let pr = self.get(&reference, 0).await?;
        if !pr.actions.merge.allowed {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                pr.actions.merge.reason.unwrap_or_default(),
            ));
        }
        let session = self.session_for(kind).await?;
        let head = match kind {
            ForgeKind::Github => self.github.current_head(&session, &pr).await?,
            ForgeKind::BitbucketCloud => self.bitbucket.current_head(&session, &pr).await?,
        };
        if !same_commit(&head, &request.expected_head_sha) || !same_commit(&head, &pr.head_sha) {
            if let Ok(fresh) = self.get(&reference, 0).await {
                self.emit(&fresh);
            }
            return Err(AppError::new(
                ErrorCode::Conflict,
                "New commits arrived since you looked at this pull request. Nothing was merged; look at the new changes first.",
            ));
        }
        // GitHub wants the full 40 characters; the branch read gives them.
        let request = MergeRequest {
            expected_head_sha: head,
            ..request
        };
        let result = match kind {
            ForgeKind::Github => self.github.merge(&session, &pr, &request).await,
            ForgeKind::BitbucketCloud => self.bitbucket.merge(&session, &pr, &request).await,
        };
        let warning = match result {
            Ok(warning) => warning,
            Err(e) if e.code == ErrorCode::Timeout => {
                let merged = self
                    .get(&reference, 0)
                    .await
                    .is_ok_and(|p| p.state == PullRequestState::Merged);
                if !merged {
                    return Err(AppError::new(
                        ErrorCode::Timeout,
                        format!(
                            "{} did not answer in time, and the pull request is not merged. Try again.",
                            kind.label()
                        ),
                    )
                    .with_details(e.message));
                }
                None
            }
            Err(e) => {
                self.note_refusal(kind, true, &e).await;
                return Err(e);
            }
        };
        tracing::info!(reference, method = ?request.method, "pull request merged");
        let outcome = self.after_write(&reference).await?;
        Ok(MergeOutcome {
            pull_request: outcome.pull_request,
            conversation: outcome.conversation,
            warning,
        })
    }

    async fn record_sent(&self, reference: &str, parts: Vec<SentPart>) -> AppResult<()> {
        if parts.is_empty() {
            return Ok(());
        }
        let summary = drafts::summary_id(reference);
        let now = now_rfc3339();
        self.core
            .call(move |conn| {
                for part in parts {
                    match part {
                        SentPart::Draft { id, remote_id } => {
                            drafts::mark_sent(conn, &id, &remote_id, &now)?
                        }
                        SentPart::Summary { remote_id } => {
                            drafts::mark_sent(conn, &summary, &remote_id, &now)?
                        }
                    }
                }
                Ok(())
            })
            .await
    }

    /// After a timeout: which unsent drafts (and the summary) are in the
    /// conversation after all. They are marked sent. Returns whether all of
    /// them were.
    async fn match_sent(
        &self,
        reference: &str,
        drafts: &[crate::models::ReviewDraft],
        body: &str,
    ) -> bool {
        let Ok(conversation) = self.conversation(reference, 0).await else {
            return false;
        };
        let mut parts = Vec::new();
        let mut all = true;
        for d in drafts.iter().filter(|d| d.remote_id.is_none()) {
            match find_posted(&conversation.threads, Some(&d.anchor.path), None, &d.body) {
                Some(remote_id) => parts.push(SentPart::Draft {
                    id: d.id.clone(),
                    remote_id,
                }),
                None => all = false,
            }
        }
        if !body.trim().is_empty() {
            match find_posted(&conversation.threads, None, None, body) {
                Some(remote_id) => parts.push(SentPart::Summary { remote_id }),
                None => all = false,
            }
        }
        if let Err(e) = self.record_sent(reference, parts).await {
            tracing::warn!(error = %e, "could not record what was sent");
            return false;
        }
        all
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

    /// How many open pull requests wait on the account's user in each
    /// workspace with pull requests on, for the sidebar. Counted from the
    /// cache only, so it costs no request: the lists are kept fresh by
    /// `sync_tick`, and `pr_changed` tells the UI when to count again.
    pub async fn review_counts(&self) -> AppResult<Vec<ReviewCount>> {
        let mut counts = Vec::new();
        for ws in self.repositories.workspaces().await? {
            if !ws.pull_requests {
                continue;
            }
            // Two checkouts of one repository share its pull requests: count
            // each hosted repository once.
            let mut forges: Vec<String> = Vec::new();
            for id in ws.members.iter().filter_map(|m| m.repository_id.as_ref()) {
                let Ok(row) = self.repositories.row(id).await else {
                    continue;
                };
                if let Some(t) = tracked(&row) {
                    let forge = t.forge.to_string();
                    if !forges.contains(&forge) {
                        forges.push(forge);
                    }
                }
            }
            let awaiting = self
                .cache
                .call(move |conn| {
                    let mut n = 0u32;
                    for forge in &forges {
                        n += cache::list(conn, forge, false)?
                            .iter()
                            .filter(|pr| pr.awaiting_my_review)
                            .count() as u32;
                    }
                    Ok(n)
                })
                .await?;
            counts.push(ReviewCount {
                workspace_id: ws.id,
                awaiting,
            });
        }
        Ok(counts)
    }

    /// A fetch moved branches on the remote: the open pull requests whose
    /// source is one of them are marked stale and read again now, so the
    /// head on screen moves with the branch without waiting for the next
    /// list refresh (SPEC.md, Staying up to date). Returns what was read
    /// anew; the rest stays stale until the provider answers again.
    pub async fn branches_moved(&self, fetched: &FetchResult) -> Vec<String> {
        let prefix = format!("{}/", fetched.remote);
        let branches: Vec<&str> = fetched
            .moved
            .iter()
            .filter_map(|r| r.strip_prefix(prefix.as_str()))
            .collect();
        if branches.is_empty() {
            return Vec::new();
        }
        let Ok(row) = self.repositories.row(&fetched.repository_id).await else {
            return Vec::new();
        };
        let Some(t) = tracked(&row) else {
            return Vec::new();
        };
        // The branch lives in the remote that was fetched: the tracked
        // repository, or `origin` when the pull requests are tracked
        // elsewhere (a fork pointed at its upstream).
        let origin = row
            .remote_url
            .as_deref()
            .and_then(ForgeRepository::from_remote_url);
        let forge = t.forge.to_string();
        let cached = self
            .cache
            .call(move |conn| cache::list(conn, &forge, false))
            .await
            .unwrap_or_default();
        let references: Vec<String> = cached
            .iter()
            .filter(|pr| branches.contains(&pr.source_branch.as_str()))
            .filter(|pr| {
                own_branch(pr, &t.forge) || origin.as_ref().is_some_and(|o| own_branch(pr, o))
            })
            .map(|pr| pr.reference.clone())
            .collect();
        if references.is_empty() {
            return Vec::new();
        }
        let stale = references.clone();
        if let Err(e) = self
            .cache
            .call(move |conn| cache::mark_stale(conn, &stale))
            .await
        {
            tracing::warn!(error = %e, "could not mark pull requests stale");
        }
        for reference in &references {
            // An unreachable provider answers with the cached pull request,
            // which stays marked stale for the next read.
            if let Err(e) = self.get(reference, 0).await {
                tracing::debug!(reference, error = %e, "refresh after fetch did not complete");
            }
        }
        self.cache
            .call(move |conn| cache::read_again(conn, &references))
            .await
            .unwrap_or_default()
    }
}
