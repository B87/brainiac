//! Entities, IPC DTOs, and the structured error type.
//!
//! Every type here is serialized to the frontend with `serde` and exported to
//! TypeScript with `ts-rs` (run `cargo test export_bindings` to regenerate
//! `src/lib/generated/`). Optional fields serialize as `null`, so the
//! TypeScript side sees `T | null`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Stable error codes shared with the frontend (docs/architecture.md, IPC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export)]
pub enum ErrorCode {
    Validation,
    NotFound,
    Conflict,
    PermissionDenied,
    /// A server did not accept the credential: a wrong password, or a token
    /// revoked or expired. The next use reads or asks for it again.
    Unauthenticated,
    Io,
    Db,
    DependencyUnavailable,
    Timeout,
    Cancelled,
}

/// The one error type that crosses the IPC boundary.
///
/// `thiserror::Error` lets this struct be used with Rust's `?` operator and
/// `Result`, while `Serialize` lets Tauri hand it to the frontend as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, thiserror::Error)]
#[error("{code:?}: {message}")]
#[ts(export)]
pub struct AppError {
    pub code: ErrorCode,
    /// User-facing text describing the failed action.
    pub message: String,
    pub retryable: bool,
    /// Low-level diagnostics (stderr, SQL error). Shown only on demand.
    pub details: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: matches!(
                code,
                ErrorCode::Io | ErrorCode::Timeout | ErrorCode::DependencyUnavailable
            ),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Validation, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Io, message)
    }

    pub fn db(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Db, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Timeout, message)
    }

    pub fn dependency(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::DependencyUnavailable, message)
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        let code = match err.kind() {
            std::io::ErrorKind::NotFound => ErrorCode::NotFound,
            std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
            _ => ErrorCode::Io,
        };
        AppError::new(code, "A file operation failed.").with_details(err.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::db("The local database reported an error.").with_details(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::db("Stored data could not be decoded.").with_details(err.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

// ---------------------------------------------------------------------------
// Settings and snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EditorSettings {
    pub executable: String,
    /// Argument template for opening a repository root; `{path}` and
    /// `{path_url}` (the path encoded for a link) are substituted.
    pub repo_args: Vec<String>,
    /// Argument template for opening a file; `{path}`, `{path_url}`, and
    /// `{line}` are substituted.
    pub file_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiffLimits {
    #[ts(type = "number")]
    pub max_bytes: u64,
    #[ts(type = "number")]
    pub max_lines: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Settings {
    pub editor: EditorSettings,
    #[ts(type = "number")]
    pub refresh_interval_seconds: u64,
    #[ts(type = "number")]
    pub status_timeout_seconds: u64,
    pub diff_limits: DiffLimits,
    /// Minutes between automatic fetches of a workspace's watched branches (minimum 5).
    #[ts(type = "number")]
    pub auto_fetch_interval_minutes: u64,
    #[ts(type = "number")]
    pub fetch_timeout_seconds: u64,
    /// Write `brainiac_id` into a note the first time it gets a task or a
    /// repository link (SPEC.md, Note identity).
    pub write_note_ids: bool,
    /// What agents connected through `brainiac mcp` may do (SPEC.md, Agent access).
    pub agent_access: AgentAccess,
    /// Keep a history of the statements run on database connections
    /// (SPEC.md, Databases: History).
    pub query_history: bool,
}

/// Settings → Agent access (SPEC.md, section 9). Off by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentAccess {
    #[default]
    Off,
    ReadOnly,
    ReadWrite,
}

/// Settings → Agent access: what the app shows about agents (SPEC.md, section 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentAccessStatus {
    pub access: AgentAccess,
    /// Agents connected now, whatever they may do.
    pub connections: u32,
    /// The program an agent runs with the argument `mcp`, such as
    /// `/Applications/Brainiac.app/Contents/MacOS/brainiac`.
    pub executable: String,
    /// Why agents cannot connect, such as a socket that could not be created.
    pub problem: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            editor: EditorSettings {
                executable: "code".into(),
                repo_args: vec!["{path}".into()],
                file_args: vec!["-g".into(), "{path}:{line}".into()],
            },
            refresh_interval_seconds: 60,
            status_timeout_seconds: 5,
            diff_limits: DiffLimits {
                max_bytes: 1024 * 1024,
                max_lines: 10_000,
            },
            auto_fetch_interval_minutes: 15,
            fetch_timeout_seconds: 60,
            write_note_ids: true,
            agent_access: AgentAccess::Off,
            query_history: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PinEntityType {
    Repository,
    Workspace,
    Note,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Pin {
    pub entity_type: PinEntityType,
    pub entity_id: String,
    #[ts(type = "number")]
    pub position: i64,
}

/// What the app knows about the system Git binary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitInfo {
    pub available: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    /// Setup guidance when Git is missing or too old.
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AppSnapshot {
    /// Increments on every committed change; clients discard older snapshots.
    #[ts(type = "number")]
    pub snapshot_version: u64,
    pub git: GitInfo,
    pub repositories: Vec<RepositorySummary>,
    pub workspaces: Vec<Workspace>,
    pub pins: Vec<Pin>,
    pub recent_repository_ids: Vec<String>,
    pub settings: Settings,
}

// ---------------------------------------------------------------------------
// Repositories
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RepositoryState {
    Fresh,
    Stale,
    Refreshing,
    Missing,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HeadKind {
    Branch,
    Detached,
    /// A repository with no commits yet.
    Unborn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HeadState {
    pub kind: HeadKind,
    pub branch: Option<String>,
    pub commit_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpstreamState {
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub ref_name: String,
    #[ts(type = "number")]
    pub ahead: u64,
    #[ts(type = "number")]
    pub behind: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChangeCounts {
    #[ts(type = "number")]
    pub staged: u64,
    #[ts(type = "number")]
    pub unstaged: u64,
    #[ts(type = "number")]
    pub untracked: u64,
    #[ts(type = "number")]
    pub conflicted: u64,
    /// Distinct changed paths; a file that is both staged and unstaged counts once.
    #[ts(type = "number")]
    pub unique_paths: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RepositoryTab {
    Changes,
    History,
    Refs,
    Notes,
    /// v0.3: the repository's pull requests.
    PullRequests,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RepositorySummary {
    pub id: String,
    /// Folder name of the checkout.
    pub name: String,
    pub canonical_root: String,
    /// The path as registered; differs from `canonical_root` for linked worktrees.
    pub display_path: String,
    pub state: RepositoryState,
    pub last_checked_at: Option<String>,
    pub last_commit_at: Option<String>,
    pub head: Option<HeadState>,
    pub counts: Option<ChangeCounts>,
    pub upstream: Option<UpstreamState>,
    pub error: Option<AppError>,
    pub last_tab: Option<RepositoryTab>,
    /// Later of Brainiac's last fetch and the modification time of `FETCH_HEAD`.
    pub last_fetch_at: Option<String>,
    /// Outcome of Brainiac's last fetch; cleared by a successful one.
    pub fetch_error: Option<AppError>,
    /// The `origin` fetch URL, refreshed on registration, manual refresh, and wake.
    pub remote_url: Option<String>,
    /// Where its pull requests come from; `None` when `origin` is on neither
    /// provider and nothing was chosen (SPEC.md, Which pull requests a repository has).
    pub forge: Option<RepositoryForge>,
}

/// The cached result of one `git status` observation (stored as JSON in SQLite).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusSnapshot {
    pub observed_at: String,
    pub head: HeadState,
    pub counts: ChangeCounts,
    pub upstream: Option<UpstreamState>,
    pub last_commit_at: Option<String>,
    pub entries: Vec<ChangeEntry>,
}

// ---------------------------------------------------------------------------
// Changes and diffs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChangeGroup {
    Staged,
    Unstaged,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChangeEntry {
    pub group: ChangeGroup,
    pub kind: ChangeKind,
    pub path: String,
    pub old_path: Option<String>,
    pub is_submodule: bool,
    /// Line counts from `--numstat`, filled in by `list_changes` only; absent
    /// for untracked and binary files. `serde(default)` lets cached status
    /// JSON written before these fields existed still load.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub additions: Option<u64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub deletions: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChangesResult {
    pub repository_id: String,
    pub observed_at: String,
    pub head: HeadState,
    pub counts: ChangeCounts,
    pub entries: Vec<ChangeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum DiffSelector {
    IndexVsHead {
        path: String,
    },
    WorktreeVsIndex {
        path: String,
    },
    /// Staged and unstaged changes together: HEAD against the working tree.
    WorktreeVsHead {
        path: String,
    },
    UntrackedPreview {
        path: String,
    },
    /// `parent_index` 0 compares with the first parent (or the empty tree for a root commit).
    /// `old_path` is the pre-rename path, so Git can pair a renamed file with its source.
    Commit {
        commit_id: String,
        path: String,
        old_path: Option<String>,
        parent_index: u32,
    },
    /// What `head` changed since it left `base`: `base...head`, from their
    /// merge base, as a pull request's diff is shown (v0.3).
    Range {
        base: String,
        head: String,
        path: String,
        old_path: Option<String>,
    },
}

impl DiffSelector {
    pub fn path(&self) -> &str {
        match self {
            DiffSelector::IndexVsHead { path }
            | DiffSelector::WorktreeVsIndex { path }
            | DiffSelector::WorktreeVsHead { path }
            | DiffSelector::UntrackedPreview { path }
            | DiffSelector::Commit { path, .. }
            | DiffSelector::Range { path, .. } => path,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiffLineKind {
    Context,
    Add,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    #[ts(type = "number | null")]
    pub old_no: Option<u64>,
    #[ts(type = "number | null")]
    pub new_no: Option<u64>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Hunk {
    pub header: String,
    #[ts(type = "number")]
    pub old_start: u64,
    #[ts(type = "number")]
    pub old_lines: u64,
    #[ts(type = "number")]
    pub new_start: u64,
    #[ts(type = "number")]
    pub new_lines: u64,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NonTextKind {
    Binary,
    Submodule,
    LfsPointer,
    Symlink,
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum DiffContent {
    Text {
        old_path: Option<String>,
        new_path: Option<String>,
        hunks: Vec<Hunk>,
        /// True when display limits cut the patch short.
        truncated: bool,
        #[ts(type = "number | null")]
        total_lines: Option<u64>,
    },
    NonText {
        reason: NonTextKind,
        summary: String,
        #[ts(type = "number | null")]
        byte_size: Option<u64>,
    },
}

/// How a patch is computed, independent of which patch is shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiffOptions {
    /// `git diff -w`: ignore whitespace when comparing lines.
    #[serde(default)]
    pub ignore_whitespace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiffResult {
    pub repository_id: String,
    pub selector: DiffSelector,
    pub content: DiffContent,
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ListCommitsRequest {
    pub repository_id: String,
    /// Ref name or commit id; defaults to HEAD.
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub ref_name: Option<String>,
    /// Substring matched against subject, body, and hash (case-insensitive).
    pub filter: Option<String>,
    pub cursor: Option<String>,
    /// Default 100, maximum 500.
    pub limit: Option<u32>,
    /// Substring of the author's name or email (case-insensitive).
    #[serde(default)]
    pub author: Option<String>,
    /// Ref whose history is left out: `ref` = topic, `exclude` = main lists
    /// the commits on topic that are not on main.
    #[serde(default)]
    pub exclude: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommitSummary {
    pub id: String,
    pub short_id: String,
    pub subject: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub committed_at: String,
    pub parent_ids: Vec<String>,
    /// Ref decorations as Git prints them: `HEAD -> main`, `tag: v1.2`, `origin/main`.
    pub decorations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommitPage {
    pub repository_id: String,
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub ref_name: String,
    /// The commit the traversal is anchored to; paging never moves it.
    pub anchor_commit_id: String,
    pub items: Vec<CommitSummary>,
    pub next_cursor: Option<String>,
}

/// One file touched by a commit, relative to the compared parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommitFile {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: ChangeKind,
    /// Line counts; absent for binary files.
    #[ts(type = "number | null")]
    pub additions: Option<u64>,
    #[ts(type = "number | null")]
    pub deletions: Option<u64>,
    pub is_binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommitDetail {
    pub repository_id: String,
    /// `flatten` inlines the summary fields, so the JSON (and the TypeScript
    /// type) is `CommitSummary` plus the fields below rather than a nested object.
    #[serde(flatten)]
    pub summary: CommitSummary,
    /// Message text after the subject line; empty when there is none.
    pub body: String,
    pub committer_name: String,
    pub committer_email: String,
    /// Index into `parent_ids` the file list compares against (0 for a root commit).
    pub compared_parent_index: u32,
    pub files: Vec<CommitFile>,
}

// ---------------------------------------------------------------------------
// Refs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RefEntry {
    /// Short name as Git abbreviates it: `main`, `origin/main`, `v1.2`.
    pub name: String,
    /// Full ref name, `refs/heads/main`; pass this as `ListCommitsRequest.ref`.
    pub full_name: String,
    pub kind: RefKind,
    /// The commit the ref points at (annotated tags are peeled to their commit).
    pub target_id: String,
    /// True for the branch HEAD has checked out.
    pub is_head: bool,
    /// Configured upstream of a local branch, short form.
    pub upstream: Option<String>,
    /// Subject of the commit the ref points at.
    pub subject: String,
    /// Committer date of that commit.
    pub committed_at: Option<String>,
    /// Commits on the branch but not on its upstream, from local refs; absent
    /// without an upstream or when the upstream ref is gone.
    pub ahead: Option<u32>,
    /// Commits on the upstream but not on the branch.
    pub behind: Option<u32>,
    /// Commits on this ref but not on `RefsResult::base`; absent for tags and the base itself.
    pub base_ahead: Option<u32>,
    /// Commits on the base but not on this ref.
    pub base_behind: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RefsResult {
    pub repository_id: String,
    /// The default branch refs are compared with, short form (`origin/main`).
    pub base: Option<String>,
    pub refs: Vec<RefEntry>,
}

// ---------------------------------------------------------------------------
// Workspaces
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiscoveryMode {
    /// Members were found inside a chosen folder (SPEC.md, Workspaces and repositories).
    Discovered,
    /// Members were picked one by one.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MemberOrigin {
    Discovered,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MemberStatus {
    Ok,
    Missing,
    NotGit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceMember {
    pub origin: MemberOrigin,
    pub display_name: String,
    pub canonical_path: String,
    /// Absent for a non-Git folder. The root is the member whose ID equals
    /// `Workspace::root_repository_id`.
    pub repository_id: Option<String>,
    pub status: MemberStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub discovery_mode: DiscoveryMode,
    /// Set when the discovery root is itself a Git repository.
    pub root_repository_id: Option<String>,
    /// Canonical path of the folder chosen for discovery; discovered workspaces only.
    pub discovery_root: Option<String>,
    /// Folder scanned for repositories, relative to `discovery_root`; absent means the root itself.
    pub discovery_path: Option<String>,
    pub members: Vec<WorkspaceMember>,
    pub activity: ActivitySettings,
    /// Unread activity events across the members.
    pub unseen_activity: u32,
    /// Whether the workspace tracks its repositories' pull requests (v0.3). Off by default.
    pub pull_requests: bool,
}

/// How a folder looked when a discovery preview was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PreviewStatus {
    /// A Git working-tree root that can be tracked.
    Ok,
    /// The folder does not exist (for example a discovery folder that was removed).
    Missing,
    /// A plain folder, or a folder that belongs to an enclosing repository.
    NotGit,
    /// Skipped on purpose (a symbolic link) or Git could not inspect it.
    Unsupported,
    /// Resolves to a repository already listed in the same preview.
    Duplicate,
    /// The selected folder sits inside another repository without being its root.
    Nested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspacePreviewEntry {
    /// Folder name.
    pub display_name: String,
    /// The path as found during discovery.
    pub configured_path: String,
    /// Canonical path, when the folder exists.
    pub resolved_path: Option<String>,
    pub status: PreviewStatus,
    /// Working-tree root Git reported, for `ok` and `nested` entries.
    pub repository_root: Option<String>,
    /// Set when this repository is already registered.
    pub existing_repository_id: Option<String>,
    /// Why the entry cannot be tracked, for any status other than `ok`.
    pub message: Option<String>,
}

/// Result of `discover_repositories`: what a workspace created from a folder would contain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspacePreview {
    /// Suggested workspace name: the selected folder's name.
    pub name: String,
    pub discovery_mode: DiscoveryMode,
    /// The root (when the folder is a repository) first, then every child folder
    /// of the discovery folder sorted by name, including skipped ones with a message.
    pub entries: Vec<WorkspacePreviewEntry>,
}

/// Result of `rescan_workspace`: what a discovered workspace's folder holds
/// that the workspace does not track (SPEC.md, Discovery and membership).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceRescan {
    pub workspace_id: String,
    /// Untracked repositories (`ok` entries), minus the targets of `moves`.
    pub repositories: Vec<WorkspacePreviewEntry>,
    /// Untracked folders that cannot be tracked: plain folders, symbolic links, nested folders.
    pub skipped: u32,
    /// Missing members that appear to have moved to an untracked folder.
    pub moves: Vec<SuggestedMove>,
}

/// A missing member and the one untracked folder that holds the same repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SuggestedMove {
    pub repository_id: String,
    /// The member's name.
    pub name: String,
    /// The member's old, missing folder.
    pub from: String,
    /// The untracked working-tree root it appears to have moved to.
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RelocateRepositoryRequest {
    pub repository_id: String,
    /// Absolute folder chosen by the user; a folder inside a working tree stands for its root.
    pub path: String,
    /// The `root` of a `needs_confirmation` the user accepted: proceed despite
    /// its concerns, as long as the folder still resolves to that root.
    pub confirmed_root: Option<String>,
}

/// Why relocating needs the user's confirmation (SPEC.md, Relocating a repository).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RelocationConcern {
    /// The chosen folder is below the top of its working tree.
    InsideRepository,
    /// The working tree contains none of the commits recorded for the registration.
    UnrelatedHistory,
    /// Nothing was recorded for the registration to compare with.
    UnverifiedHistory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[ts(export)]
pub enum RelocationOutcome {
    Relocated {
        /// Boxed so this variant is not much larger than the other one in
        /// memory; it serializes exactly like a plain `RepositorySummary`.
        repository: Box<RepositorySummary>,
        /// Names of the missing repositories inside the old folder that moved along.
        carried: Vec<String>,
    },
    /// Nothing was written; repeat the request with `confirmed` to proceed.
    NeedsConfirmation {
        /// The working-tree root the registration would point to.
        root: String,
        concerns: Vec<RelocationConcern>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub discovery_mode: DiscoveryMode,
    /// Discovered workspaces only: the selected folder (absolute).
    pub discovery_root: Option<String>,
    /// Discovered workspaces only: scanned folder relative to `discovery_root`.
    pub discovery_path: Option<String>,
    /// Absolute folders to track. A path equal to `discovery_root` that is a
    /// repository becomes the workspace root.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateWorkspaceMembershipRequest {
    pub workspace_id: String,
    /// Absolute folders to add as members.
    pub add: Vec<String>,
    /// `canonical_path` values of members to drop. Repository registrations are kept.
    pub remove: Vec<String>,
}

// ---------------------------------------------------------------------------
// Activity and fetching
// ---------------------------------------------------------------------------

/// Per-workspace activity configuration (SPEC.md, Workspace activity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActivitySettings {
    /// Branch names or globs, matched without the remote prefix.
    pub watched_branches: Vec<String>,
    /// Tag globs.
    pub watched_tags: Vec<String>,
    /// Fetch the watched branches on a schedule. Off by default.
    pub auto_fetch: bool,
    /// macOS notification when a watched branch moves.
    pub notify_moves: bool,
    /// Notification at 09:00 when there are unread events.
    pub morning_digest: bool,
    /// Show incoming paths that the working tree also changes.
    pub warn_conflicts: bool,
}

impl Default for ActivitySettings {
    fn default() -> Self {
        Self {
            watched_branches: vec!["main".into(), "master".into(), "develop".into()],
            watched_tags: vec!["v*".into()],
            auto_fetch: false,
            notify_moves: false,
            morning_digest: false,
            warn_conflicts: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ActivityKind {
    /// The old tip is an ancestor of the new one.
    Advanced,
    /// History was replaced (force-push).
    Rewritten,
    /// A watched branch appeared.
    Created,
    /// A watched tag appeared.
    Tagged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActivityCommit {
    pub id: String,
    pub short_id: String,
    pub subject: String,
    pub author_name: String,
    pub committed_at: String,
    pub is_merge: bool,
}

/// The current branch fell behind a watched branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActivityDrift {
    /// The checked-out branch.
    pub branch: String,
    /// The watched ref it is compared with, such as `origin/develop`.
    pub base: String,
    pub behind: u32,
}

/// What was recorded about one moved ref, stored as JSON with the event and
/// sent inline in `ActivityItem`. `serde(default)` keeps older stored rows
/// readable when fields are added; a failed lookup leaves a field empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct ActivityDetail {
    /// Newest first, at most five.
    pub commits: Vec<ActivityCommit>,
    pub total_commits: u32,
    pub merges: u32,
    /// Commits that a rewrite removed from the branch.
    pub replaced: u32,
    pub authors: Vec<String>,
    /// Incoming paths that the working tree also changes.
    pub conflict_paths: Vec<String>,
    pub drift: Option<ActivityDrift>,
    pub previous_tag: Option<String>,
    pub commits_since_previous_tag: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActivityItem {
    pub id: String,
    /// A checkout of the repository the event belongs to.
    pub repository_id: String,
    pub repository_name: String,
    pub kind: ActivityKind,
    /// Short name for display: `origin/main`, or the tag name.
    pub ref_name: String,
    /// Full name: `refs/remotes/origin/main` or `refs/tags/v1`.
    pub full_ref: String,
    pub old_id: Option<String>,
    pub new_id: String,
    /// When Brainiac noticed the move.
    pub observed_at: String,
    pub seen: bool,
    /// Inlined: the JSON and TypeScript type are flat.
    #[serde(flatten)]
    pub detail: ActivityDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RepositoryFreshness {
    pub repository_id: String,
    pub name: String,
    pub last_fetch_at: Option<String>,
    pub fetch_error: Option<AppError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PulseAuthor {
    pub name: String,
    pub commits: u32,
}

/// Activity on the watched refs over the last seven days, from local refs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TeamPulse {
    pub since: String,
    pub commits: u32,
    pub merges: u32,
    pub releases: u32,
    /// Most active first.
    pub authors: Vec<PulseAuthor>,
    /// Names of the repositories with the most commits, most active first.
    pub repositories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceActivity {
    pub workspace_id: String,
    pub settings: ActivitySettings,
    /// Unread first, then seen; newest first within each; at most 200.
    pub items: Vec<ActivityItem>,
    pub unseen: u32,
    /// Oldest fetch first.
    pub freshness: Vec<RepositoryFreshness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FetchResult {
    pub repository_id: String,
    pub remote: String,
    /// `None` when there was nothing to fetch (auto-fetch: no watched branch
    /// exists on the remote yet).
    pub fetched_at: Option<String>,
    /// Refs whose tips changed, such as `origin/main` or `v1.2`.
    pub moved: Vec<String>,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChangeOrigin {
    Watcher,
    Refresh,
    Timer,
    Wake,
    Registration,
    Fetch,
}

/// Emitted as the `repository_changed` event after a status observation is committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RepositoryChangedEvent {
    pub repository_id: String,
    #[ts(type = "number")]
    pub snapshot_version: u64,
    pub origin: ChangeOrigin,
    /// False when the observation matched the previous one, so open views can
    /// skip reloading. Reloading on every event would loop: loading the
    /// Changes tab itself refreshes status and emits this event.
    pub changed: bool,
}

/// Emitted as the `menu` event when a native menu item is activated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MenuEvent {
    pub id: String,
}

// ---------------------------------------------------------------------------
// Vault and notes (v0.2)
// ---------------------------------------------------------------------------

/// The vault Brainiac edits: one folder of Markdown notes (SPEC.md, The vault).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VaultInfo {
    pub id: String,
    /// Folder name of the vault.
    pub name: String,
    pub root_path: String,
    /// False while the folder cannot be read, such as on an unmounted disk.
    pub available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum IndexState {
    /// No vault is chosen; tasks are still searchable.
    NoVault,
    /// The vault is being scanned; results may be incomplete.
    Indexing,
    Ready,
    /// The vault cannot be read, or the index failed; repository names still match.
    Unavailable,
}

/// How complete search is. Emitted as `index_status_changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IndexStatus {
    pub state: IndexState,
    /// Notes indexed so far and in total during a scan.
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number")]
    pub total: u64,
    /// Saved notes whose search update failed and is being retried.
    #[ts(type = "number")]
    pub pending_repairs: u64,
    /// Why search is unavailable.
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VaultState {
    pub vault: Option<VaultInfo>,
    pub index: IndexStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NoteTextState {
    Text,
    /// Over 5 MiB: found by name, opened externally.
    TooLarge,
    /// Not UTF-8: found by name, opened externally.
    NotUtf8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteSummary {
    pub id: String,
    /// Path inside the vault with forward slashes, such as `Projects/Plan.md`.
    pub relative_path: String,
    pub title: String,
    pub text_state: NoteTextState,
    /// The file is gone; the note keeps its tasks and links.
    pub missing: bool,
    /// Moved to Brainiac's trash.
    pub trashed: bool,
    /// Another note carries the same `brainiac_id` (SPEC.md, Note identity).
    pub id_conflict: bool,
    /// Whether the file carries `brainiac_id` in its frontmatter.
    pub has_embedded_id: bool,
    /// The file's modification time.
    pub modified_at: String,
    pub last_opened_at: Option<String>,
    /// The file name the title would give, such as `Payment retries.md`,
    /// while it differs from the note's own; `None` when they match
    /// (SPEC.md, Note identity).
    pub title_file_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FolderEntryKind {
    Folder,
    Note,
    /// Any other file: listed, opened externally.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FolderEntry {
    pub name: String,
    pub relative_path: String,
    pub kind: FolderEntryKind,
    pub note: Option<NoteSummary>,
}

/// One folder of the vault, listed from disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FolderListing {
    /// Empty for the vault's top folder.
    pub relative_path: String,
    pub entries: Vec<FolderEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteLists {
    pub pinned: Vec<NoteSummary>,
    pub recent: Vec<NoteSummary>,
}

/// Edits that were not saved, kept in history.db until a save succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteDraft {
    pub text: String,
    /// The version the edits started from; when it differs from the file's, the draft is a conflict.
    pub base_version: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteContent {
    pub note: NoteSummary,
    /// The note's Markdown; `None` for a note that is not editable text. For
    /// a missing note, its last known text from revision history, if any.
    pub text: Option<String>,
    /// SHA-256 of the file as read; a save must name it.
    pub version: String,
    pub draft: Option<NoteDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SaveNoteRequest {
    pub note_id: String,
    pub expected_version: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SaveNoteResult {
    pub note: NoteSummary,
    pub version: String,
    /// The file is saved but search could not be updated yet; it is retried.
    pub search_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateNoteRequest {
    /// Folder inside the vault; `None` for the top folder.
    pub folder: Option<String>,
    /// Title, also used for the file name; "Untitled" when empty.
    pub title: Option<String>,
    /// Link the new note to this repository (New Note for repository).
    pub repository_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RenameNoteRequest {
    pub note_id: String,
    /// New path inside the vault, ending in `.md`.
    pub new_path: String,
    /// Also rewrite links to the note in other notes.
    pub update_links: bool,
}

/// What a rename would change before it is made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RenamePreview {
    pub new_path: String,
    /// Notes whose links to this note would be rewritten.
    pub linking_notes: Vec<NoteSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RenameResult {
    pub note: NoteSummary,
    /// Notes whose links were rewritten.
    pub updated: Vec<String>,
    /// Notes that could not be rewritten, such as ones changed meanwhile.
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TrashedNote {
    pub note: NoteSummary,
    pub trashed_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RevisionReason {
    AppSave,
    ExternalChange,
    Restore,
    /// Saved by an agent through agent access (SPEC.md, section 9).
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteRevision {
    pub id: String,
    pub created_at: String,
    pub reason: RevisionReason,
    #[ts(type = "number")]
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LinkedRepository {
    pub repository_id: String,
    /// The name when the link was made; the live name comes from the snapshot.
    pub name: String,
    pub remote_url: Option<String>,
    /// False when the repository was removed from Brainiac.
    pub registered: bool,
    /// A registered repository with the same remote, offered as a reconnection.
    pub reconnect_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Backlink {
    pub note: NoteSummary,
    /// The linking line, trimmed.
    pub excerpt: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NoteLinkKind {
    Markdown,
    Wikilink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UnresolvedLink {
    pub raw_target: String,
    pub kind: NoteLinkKind,
    pub line: u32,
    /// Where **Create** would put the note.
    pub suggested_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RepositorySuggestion {
    pub repository_id: String,
    pub name: String,
}

/// Where a clicked link in a note leads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResolvedLink {
    /// The note the link points at, when it exists.
    pub note: Option<NoteSummary>,
    /// Where **Create** would put the note when it does not.
    pub suggested_path: String,
}

/// Everything the context panel shows for one note (SPEC.md, Notes view).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteContext {
    pub note_id: String,
    pub repositories: Vec<LinkedRepository>,
    pub tasks: Vec<Task>,
    pub backlinks: Vec<Backlink>,
    pub unresolved: Vec<UnresolvedLink>,
    pub suggestions: Vec<RepositorySuggestion>,
}

/// A repository's Notes tab (SPEC.md, Notes and repositories).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RepositoryNotes {
    pub repository_id: String,
    pub notes: Vec<NoteSummary>,
    /// Open tasks linked to the repository.
    pub tasks: Vec<Task>,
    /// Notes that mention the repository by name and are not linked to it.
    pub suggested: Vec<NoteSummary>,
}

// ---------------------------------------------------------------------------
// Tasks (v0.2)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
    Cancelled,
}

impl TaskStatus {
    pub fn is_open(self) -> bool {
        matches!(self, TaskStatus::Todo | TaskStatus::InProgress)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskNote {
    pub id: String,
    pub title: String,
    pub relative_path: String,
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    /// No planned date, no deadline, and not marked Sorted (SPEC.md, Tasks and Today).
    pub to_sort: bool,
    /// Local calendar dates, `YYYY-MM-DD`.
    pub planned_date: Option<String>,
    pub due_date: Option<String>,
    pub note: Option<TaskNote>,
    pub repository_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    /// Increments on every change; an update must name the version it read.
    #[ts(type = "number")]
    pub version: i64,
}

/// Everything a task's editor can change. Sent whole with the version it was read at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskFields {
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub planned_date: Option<String>,
    pub due_date: Option<String>,
    pub note_id: Option<String>,
    pub repository_id: Option<String>,
    /// Marked Sorted without a date. Giving the task a date also sorts it.
    pub sorted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateTaskRequest {
    pub task_id: String,
    #[ts(type = "number")]
    pub expected_version: i64,
    pub fields: TaskFields,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskFilter {
    /// Only these statuses; all when `None`.
    pub statuses: Option<Vec<TaskStatus>>,
    /// Only tasks to sort (true) or only sorted ones (false).
    pub to_sort: Option<bool>,
    pub note_id: Option<String>,
    pub repository_id: Option<String>,
}

/// The Today view for the Mac's current date (SPEC.md, Tasks and Today).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TodayView {
    /// Today's local date, `YYYY-MM-DD`.
    pub date: String,
    /// Open tasks that are overdue, due today, or planned for today.
    pub open: Vec<Task>,
    /// Tasks completed today.
    pub completed: Vec<Task>,
    /// Open tasks to sort, shown folded.
    pub to_sort: Vec<Task>,
    /// Repositories linked to today's tasks or their notes, without duplicates.
    pub repository_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Search (v0.2)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SearchKind {
    Note,
    Task,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchRequest {
    /// Literal text; never read as search syntax.
    pub query: String,
    /// Only these kinds; both when `None`.
    pub kinds: Option<Vec<SearchKind>>,
    /// Results per kind (default 8, at most 200).
    pub limit: Option<u32>,
}

/// A piece of a snippet or title; highlighted pieces matched a query word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TextPart {
    pub text: String,
    pub highlight: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchHit {
    pub kind: SearchKind,
    pub id: String,
    pub title: Vec<TextPart>,
    /// The note's path, or the task's status and dates.
    pub detail: String,
    pub snippet: Vec<TextPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchGroup {
    pub hits: Vec<SearchHit>,
    /// How many match in all (Show all N).
    #[ts(type = "number")]
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchResults {
    pub query: String,
    pub notes: SearchGroup,
    pub tasks: SearchGroup,
    pub index: IndexStatus,
}

// ---------------------------------------------------------------------------
// Export and restore (v0.2)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExportResult {
    /// The folder the export was written to.
    pub path: String,
    #[ts(type = "number")]
    pub notes: u64,
    #[ts(type = "number")]
    pub tasks: u64,
    /// Files that changed during the export and could not be copied consistently.
    pub problems: Vec<String>,
    /// True only when every note was copied as listed in the manifest.
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExportedRepository {
    pub name: String,
    pub remote_url: Option<String>,
}

/// What an export contains, checked before anything is replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RestorePreview {
    pub path: String,
    pub exported_at: String,
    pub app_version: String,
    pub vault_name: Option<String>,
    #[ts(type = "number")]
    pub notes: u64,
    #[ts(type = "number")]
    pub tasks: u64,
    pub repositories: Vec<ExportedRepository>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RestoreRequest {
    /// The export folder.
    pub export_path: String,
    /// Where the vault goes: an empty or new folder the export's notes are
    /// copied into, or, with `use_existing_vault`, a folder that already holds them.
    pub vault_path: String,
    pub use_existing_vault: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RestoreResult {
    /// Repositories matched to a registered one by remote URL.
    #[ts(type = "number")]
    pub matched_repositories: u64,
    /// Repositories that need Locate….
    pub unmatched_repositories: Vec<String>,
    /// Brainiac restarts to open the restored data.
    pub needs_restart: bool,
}

// ---------------------------------------------------------------------------
// v0.2 events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NoteChangeOrigin {
    /// Saved, created, or renamed in Brainiac.
    App,
    /// Changed outside Brainiac: another editor, a `git pull`, an agent.
    External,
    /// Trashed or restored.
    Trash,
    /// Saved or created by an agent through agent access (SPEC.md, section 9).
    Agent,
}

/// Emitted as `note_changed`; carries no note text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteChangedEvent {
    pub note_id: String,
    /// The note's version after the change; `None` once it is gone.
    pub version: Option<String>,
    pub relative_path: String,
    pub origin: NoteChangeOrigin,
}

/// Emitted as `note_missing` when a note's file is gone for good.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteMissingEvent {
    pub note_id: String,
}

/// Emitted as `task_changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskChangedEvent {
    pub task_id: String,
    /// `None` when the task was deleted.
    #[ts(type = "number | null")]
    pub version: Option<i64>,
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

// ---------------------------------------------------------------------------
// v0.3: pull requests (SPEC.md, section 10)
// ---------------------------------------------------------------------------

/// A hosting service Brainiac reads pull requests from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ForgeKind {
    Github,
    BitbucketCloud,
}

/// What kind of token an account uses, as far as Brainiac can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ForgeTokenKind {
    /// A GitHub fine-grained personal access token (`github_pat_…`).
    FineGrained,
    /// A GitHub personal access token (classic) (`ghp_…`).
    Classic,
    /// An Atlassian API token.
    ApiToken,
    Other,
}

/// An account on GitHub or Bitbucket Cloud, as checked when it was added or
/// its token replaced. The token itself is only in the Keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForgeAccount {
    pub kind: ForgeKind,
    pub login: String,
    /// The provider's stable ID for the user (GitHub's number, Bitbucket's
    /// UUID), which pull requests name authors and reviewers by.
    pub user_id: String,
    pub display_name: Option<String>,
    /// Bitbucket Cloud: the Atlassian account email sent with the token.
    pub email: Option<String>,
    pub token_kind: ForgeTokenKind,
    pub expires_at: Option<String>,
    /// The token's scopes when the provider says (Bitbucket, and GitHub
    /// classic tokens); `None` for a GitHub fine-grained token, whose
    /// permissions GitHub does not reveal.
    pub scopes: Option<Vec<String>>,
    /// Saved with **Save as Read-Only**, or found unable to write since:
    /// reviewing and merging are not offered.
    pub read_only: bool,
    /// Scopes or permissions to add to the token for what it cannot do.
    pub missing: Vec<String>,
    pub checked_at: String,
    /// Where the token comes from: Store (the Keychain item `brainiac/github`
    /// or `brainiac/bitbucket`), an environment variable, or a command.
    pub token_source: SecretSource,
    pub credential: CredentialState,
}

/// Settings → Accounts: one entry per provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForgeAccountSlot {
    pub kind: ForgeKind,
    pub account: Option<ForgeAccount>,
    /// The Keychain item (service `brainiac`, account `github` or
    /// `bitbucket`) holds a token while no account was added, such as one
    /// stored from Terminal. Found without reading the token.
    pub keychain_token: bool,
}

/// Add an account or replace its token.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SaveForgeAccountRequest {
    pub kind: ForgeKind,
    /// Where the token comes from: Store, an environment variable, or a command.
    pub source: SecretSource,
    /// Store: a pasted token; `None` uses the one already in the Keychain item.
    pub token: Option<String>,
    /// Bitbucket Cloud only; `None` keeps the account's email.
    pub email: Option<String>,
    /// Save a token that can read but not write.
    pub read_only: bool,
}

// Written by hand instead of derived so a pasted token never reaches a log.
impl std::fmt::Debug for SaveForgeAccountRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveForgeAccountRequest")
            .field("kind", &self.kind)
            .field("source", &self.source)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("email", &self.email)
            .field("read_only", &self.read_only)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[ts(export)]
pub enum SaveForgeAccountOutcome {
    Saved {
        // Boxed: an account is much larger than the other variant.
        account: Box<ForgeAccount>,
        /// The account was saved, but its old Keychain item could not be
        /// deleted yet; Settings → Secrets retries.
        warning: Option<String>,
    },
    /// The token can read but not write. Nothing was saved; repeat the
    /// request with `read_only` to save it as a read-only account.
    ReadOnly { login: String, missing: Vec<String> },
}

/// A repository on GitHub or Bitbucket Cloud, named by its parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForgeTarget {
    pub kind: ForgeKind,
    /// A GitHub user or organization, or a Bitbucket workspace.
    pub owner: String,
    pub name: String,
}

/// How a repository's forge repository was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ForgeSource {
    /// Derived from where `origin` points.
    Origin,
    /// Chosen with **Change…**; kept when `origin` changes.
    Override,
}

/// Where a repository's pull requests come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RepositoryForge {
    pub kind: ForgeKind,
    pub owner: String,
    pub name: String,
    /// `github.com/acme/api`, as pull request references start.
    pub reference: String,
    pub source: ForgeSource,
}

/// Point a repository's pull requests at `forge`, or back at `origin` with `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SetRepositoryForgeRequest {
    pub repository_id: String,
    pub forge: Option<ForgeTarget>,
}

// --- The neutral pull request model (docs/architecture.md, Pull requests — v0.3) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PullRequestState {
    Open,
    Draft,
    Merged,
    /// Closed without merging; Bitbucket's declined.
    Closed,
}

/// Someone on a provider. The ID is the provider's stable one (GitHub's
/// numeric ID, Bitbucket's UUID), which accounts are matched by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForgeUser {
    pub id: String,
    pub login: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ReviewState {
    Requested,
    Approved,
    ChangesRequested,
    Commented,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Reviewer {
    pub user: ForgeUser,
    pub state: ReviewState,
    /// The account's own user.
    pub is_me: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CheckState {
    Pending,
    Success,
    Failure,
    /// Skipped, cancelled, or neutral: neither passed nor failed.
    Neutral,
}

/// One check run or commit status on the head commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Check {
    pub name: String,
    pub state: CheckState,
    pub description: Option<String>,
    /// Its log or page on the provider.
    pub url: Option<String>,
}

/// The checks on the head commit, in one line. `state` is `None` when there
/// are no checks at all (GitHub reports "pending" for that, which this is not).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChecksSummary {
    pub state: Option<CheckState>,
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub pending: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Mergeability {
    Mergeable,
    Conflicting,
    /// The provider is still working it out (GitHub's `mergeable: null`).
    Computing,
    /// The provider does not say (Bitbucket).
    Unknown,
}

/// Sizes and counts. `None` when the provider's list does not say and the
/// detail has not been read yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PullRequestCounts {
    pub comments: u32,
    pub unresolved_threads: Option<u32>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub changed_files: Option<u32>,
    pub commits: Option<u32>,
}

/// Whether an action is offered, and why not when it is not, so provider
/// differences and token limits reach the UI as data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActionAvailability {
    pub allowed: bool,
    pub reason: Option<String>,
}

impl ActionAvailability {
    pub fn allowed() -> Self {
        ActionAvailability {
            allowed: true,
            reason: None,
        }
    }

    pub fn not(reason: impl Into<String>) -> Self {
        ActionAvailability {
            allowed: false,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AvailableActions {
    /// Commenting on the pull request and replying to a thread.
    pub comment: ActionAvailability,
    pub review: ActionAvailability,
    pub approve: ActionAvailability,
    pub merge: ActionAvailability,
    /// Resolving and reopening a thread on a line.
    #[serde(default = "ActionAvailability::allowed")]
    pub resolve: ActionAvailability,
}

/// One pull request as both providers describe it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PullRequest {
    /// `github.com/acme/api#42`.
    pub reference: String,
    #[ts(type = "number")]
    pub number: u64,
    pub kind: ForgeKind,
    pub title: String,
    /// Markdown written by other people; sanitized when shown.
    pub description: String,
    /// `description` rendered by `forge::markdown`: raw HTML shown as text,
    /// images as links, so the WebView loads nothing from it.
    #[serde(default)]
    pub description_html: String,
    pub author: ForgeUser,
    pub state: PullRequestState,
    /// `owner/name` of the repository the source branch is in.
    pub source_repository: String,
    pub source_branch: String,
    /// The full head commit, or Bitbucket's short one until it is expanded.
    pub head_sha: String,
    /// The target branch's tip when the pull request was read; with the
    /// head, what a local diff compares (`base...head`).
    #[serde(default)]
    pub base_sha: String,
    pub target_branch: String,
    pub reviewers: Vec<Reviewer>,
    pub checks: ChecksSummary,
    pub mergeability: Mergeability,
    pub counts: PullRequestCounts,
    pub web_url: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    /// The provider's update time plus the head commit; opaque to callers.
    pub version: String,
    pub actions: AvailableActions,
    /// Written by the account's user.
    pub mine: bool,
    /// Its review was asked of the account's user and not given yet.
    pub awaiting_my_review: bool,
    /// The head commit the account's user last reviewed (GitHub; Bitbucket
    /// does not record it).
    #[serde(default)]
    pub reviewed_sha: Option<String>,
    /// Commits the head has that `reviewed_sha` does not, counted with local
    /// Git when both are on the Mac; `None` when unknown.
    #[serde(default)]
    pub commits_since_review: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChangedFileStatus {
    Added,
    Modified,
    Removed,
    Renamed,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChangedFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangedFileStatus,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

/// The files a pull request changes, for its head commit; or, since a commit
/// of it (the one last reviewed, or the one drafts were written on), the
/// files changed between that commit and the head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PullRequestFiles {
    pub reference: String,
    pub head_sha: String,
    /// The commit the files are compared from: the target branch's tip, or
    /// the `since` commit asked for.
    #[serde(default)]
    pub base_sha: String,
    /// Set when the list is the changes since a commit, not the whole pull request.
    #[serde(default)]
    pub partial: bool,
    pub files: Vec<ChangedFile>,
    pub fetched_at: String,
}

/// Which side of a diff a thread is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiffSide {
    Old,
    New,
}

/// Where in the diff a thread hangs (docs/architecture.md, Neutral model).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ThreadAnchor {
    pub path: String,
    pub side: DiffSide,
    /// The last line of the range, or the one line; `None` when the provider
    /// no longer places it.
    pub line: Option<u32>,
    pub start_line: Option<u32>,
    /// The commit the lines were counted on.
    pub commit: Option<String>,
}

/// One comment in a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Comment {
    /// The provider's ID, as replies and edits name it.
    pub id: String,
    pub author: ForgeUser,
    /// Markdown as written.
    pub body: String,
    /// `body` rendered and sanitized (`forge::markdown`).
    pub html: String,
    /// Set when the comment is the summary of a review with that verdict.
    pub review: Option<ReviewState>,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub mine: bool,
    pub web_url: Option<String>,
}

/// A thread: a comment on the pull request, a review's summary, or a
/// comment on a line with its replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Thread {
    /// Opaque: GitHub's node ID or Bitbucket's root comment ID.
    pub id: String,
    pub anchor: Option<ThreadAnchor>,
    pub resolved: bool,
    /// The lines it was on changed since.
    pub outdated: bool,
    pub comments: Vec<Comment>,
}

/// A pull request's threads, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Conversation {
    pub reference: String,
    pub threads: Vec<Thread>,
    pub fetched_at: String,
}

/// Where a pull request's diff came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiffSource {
    /// Local Git: both commits are on the Mac.
    Local,
    /// The provider's patch.
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PullRequestDiffRequest {
    pub reference: String,
    pub path: String,
    pub old_path: Option<String>,
    /// Compare from this commit of the pull request (the one last reviewed,
    /// or the one drafts were written on) instead of the target branch;
    /// local Git only.
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub options: DiffOptions,
}

/// One file's diff in a pull request, as `DiffView` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PullRequestDiff {
    pub reference: String,
    pub source: DiffSource,
    pub base_sha: String,
    pub head_sha: String,
    /// `repository_id` is the local checkout's; the selector is a `Range`.
    pub diff: DiffResult,
}

// --- Reviewing (SPEC.md, Reviewing) ------------------------------------------

/// A line comment written in Brainiac and kept on the Mac until the review
/// is finished (`review_drafts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReviewDraft {
    pub id: String,
    pub reference: String,
    /// Where it goes; `commit` is the head it was written on.
    pub anchor: ThreadAnchor,
    /// Markdown as written.
    pub body: String,
    /// `body` rendered and sanitized, for showing it in the diff.
    pub html: String,
    /// The provider's comment ID once this draft was sent, when a review is
    /// sent as several requests (Bitbucket) and was cut off midway.
    pub remote_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// The verdict a review gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ReviewVerdict {
    Comment,
    Approve,
    RequestChanges,
}

/// A submission cut off midway (a lost connection, Brainiac quitting): its
/// summary and verdict, kept so **Finish Review** can send what is left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PendingReview {
    pub body: String,
    pub verdict: ReviewVerdict,
    /// The commit the review was being sent for.
    pub head_sha: String,
    /// The summary was posted (Bitbucket); the verdict may still be due.
    pub summary_sent: bool,
}

/// A pull request's drafts, oldest first, with any submission under way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReviewDrafts {
    pub reference: String,
    pub drafts: Vec<ReviewDraft>,
    pub pending: Option<PendingReview>,
}

/// Write a draft, or change one (`id` set).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SaveReviewDraftRequest {
    pub reference: String,
    pub id: Option<String>,
    pub anchor: ThreadAnchor,
    pub body: String,
}

/// Comment on the whole pull request; posted at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommentRequest {
    pub reference: String,
    pub body: String,
}

/// Reply to a thread; posted at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReplyRequest {
    pub reference: String,
    pub thread_id: String,
    pub body: String,
}

/// Resolve or reopen a thread on a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResolveThreadRequest {
    pub reference: String,
    pub thread_id: String,
    pub resolved: bool,
}

/// **Finish Review**: send the drafts with a summary and a verdict, for the
/// head commit the user looked at. A head that moved is `CONFLICT` and
/// nothing is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SubmitReviewRequest {
    pub reference: String,
    pub body: String,
    pub verdict: ReviewVerdict,
    pub expected_head_sha: String,
}

/// What a write to the provider leaves behind: the conversation read again,
/// and the pull request, whose reviewers and counts may have changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WriteOutcome {
    pub pull_request: PullRequest,
    pub conversation: Conversation,
}

// --- Merging (SPEC.md, Merging) ----------------------------------------------

/// How the source branch goes into the target. GitHub has the first three;
/// Bitbucket all four (its rebase is `rebase_fast_forward`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MergeMethod {
    MergeCommit,
    Squash,
    Rebase,
    FastForward,
}

/// What the Merge confirmation offers, read from the provider when it opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MergeOptions {
    pub reference: String,
    /// The methods the repository allows, in the order they are offered.
    pub methods: Vec<MergeMethod>,
    pub default_method: MergeMethod,
    /// Whether the source branch can be deleted from here: it is in the
    /// pull request's own repository, not a fork's.
    pub can_delete_branch: bool,
    /// The repository's default for deleting the source branch.
    pub delete_branch: bool,
    /// GitHub deletes the branch itself when the repository says so; there
    /// is then nothing to choose.
    pub deletes_branch_itself: bool,
}

/// **Merge**, confirmed, for the head commit the user looked at. A head that
/// moved is `CONFLICT` and nothing is merged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MergeRequest {
    pub reference: String,
    pub method: MergeMethod,
    /// The merge commit's title; empty for a rebase or a fast-forward, and
    /// on Bitbucket, whose message is one text.
    pub commit_title: String,
    pub commit_message: String,
    pub delete_branch: bool,
    pub expected_head_sha: String,
}

/// A merge that went through: the pull request and conversation read again,
/// and what did not go through after it (deleting the branch), if anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MergeOutcome {
    pub pull_request: PullRequest,
    pub conversation: Conversation,
    pub warning: Option<String>,
}

/// The checks of a pull request's head commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PullRequestChecks {
    pub reference: String,
    pub head_sha: String,
    pub checks: Vec<Check>,
    pub fetched_at: String,
}

/// How much of the hour's request allowance an account has used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RequestBudget {
    pub kind: ForgeKind,
    pub used: u32,
    pub limit: u32,
    /// When the allowance is whole again.
    pub resets_at: Option<String>,
    /// Set while the provider refused requests or is unreachable; nothing is
    /// sent until then.
    pub retry_at: Option<String>,
}

/// What to list: a workspace's open pull requests, or one repository's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ListPullRequestsRequest {
    pub workspace_id: Option<String>,
    pub repository_id: Option<String>,
    /// One repository only: merged and closed in the last 30 days instead of
    /// open ones. Read when asked for, not kept up to date.
    pub closed: bool,
    /// Use what is cached when it is younger than this; read again otherwise.
    #[ts(type = "number")]
    pub max_age_seconds: u64,
}

/// One repository's pull requests in a list, with how that repository fared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PullRequestGroup {
    pub repository_id: String,
    pub repository_name: String,
    /// `github.com/acme/api`.
    pub forge: String,
    pub kind: ForgeKind,
    pub pull_requests: Vec<PullRequest>,
    /// When this repository's list was last read from the provider; `None` never.
    pub fetched_at: Option<String>,
    /// Why the last read failed, with the cached list kept.
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PullRequestList {
    /// Whether the workspace (or, for a repository, any of its workspaces) tracks pull requests.
    pub enabled: bool,
    /// Workspaces that track the repository, by name; one repository only.
    pub tracked_by: Vec<String>,
    /// Providers in the list that have no account yet.
    pub missing_accounts: Vec<ForgeKind>,
    pub groups: Vec<PullRequestGroup>,
    pub budgets: Vec<RequestBudget>,
}

/// A workspace's count of open pull requests waiting on the account's user,
/// for the sidebar (SPEC.md, Workspace → Pull requests). Counted from the
/// cache; only workspaces with pull requests on are listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReviewCount {
    pub workspace_id: String,
    pub awaiting: u32,
}

/// Emitted as `pr_changed` when a pull request was read anew or written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PullRequestChangedEvent {
    pub reference: String,
    pub version: String,
    pub origin: PullRequestChangeOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PullRequestChangeOrigin {
    /// Written from Brainiac.
    App,
    /// Read from the provider.
    Remote,
}

// ---------------------------------------------------------------------------
// v0.4: databases (SPEC.md, section 11)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DbKind {
    Sqlite,
    Postgres,
}

impl DbKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DbKind::Sqlite => "sqlite",
            DbKind::Postgres => "postgres",
        }
    }
}

/// Where a connection points; its color everywhere it appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DbEnvironment {
    Local,
    Development,
    Staging,
    Production,
}

impl DbEnvironment {
    pub fn as_str(self) -> &'static str {
        match self {
            DbEnvironment::Local => "local",
            DbEnvironment::Development => "development",
            DbEnvironment::Staging => "staging",
            DbEnvironment::Production => "production",
        }
    }
}

/// What statements on a connection may do. Every connection is read only
/// until writes arrive (SPEC.md, Databases: Safety).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DbAccess {
    ReadOnly,
    ReadWrite,
}

/// PostgreSQL TLS: verify the certificate and host name, encrypt without
/// verifying, or connect in plain text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DbTls {
    Verify,
    Require,
    Off,
}

/// A saved connection. Its password never leaves Rust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbConnection {
    pub id: String,
    pub name: String,
    pub kind: DbKind,
    pub environment: DbEnvironment,
    pub access: DbAccess,
    /// SQLite: the database file.
    pub file_path: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub user: Option<String>,
    pub tls: Option<DbTls>,
    /// A PEM file of certificate authorities to trust besides the Mac's.
    pub ca_file: Option<String>,
    /// Where the password comes from: Store (the Keychain item
    /// `brainiac/db:<id>`), Ask, None, an environment variable, or a command.
    pub password: SecretSource,
    /// Whether a statement can run without asking for the password first:
    /// false only for `Ask` before the password was entered in this run.
    pub password_ready: bool,
    pub credential: CredentialState,
    pub statement_timeout_seconds: u32,
    /// SQLite: the file's size, or `None` when it is missing.
    #[ts(type = "number | null")]
    pub file_size: Option<u64>,
    /// Repositories the connection is linked to, shown in their side panels.
    pub repository_ids: Vec<String>,
    /// Where a PostgreSQL server runs, for Health's machine metrics.
    pub runs_on: Option<RunsOn>,
    #[ts(type = "number")]
    pub version: i64,
}

/// New Connection… and Edit Connection….
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SaveDbConnectionRequest {
    /// `None` for a new connection.
    pub id: Option<String>,
    /// The version the form was opened on; a save over a newer one is refused.
    #[ts(type = "number | null")]
    pub expected_version: Option<i64>,
    pub name: String,
    pub kind: DbKind,
    pub environment: DbEnvironment,
    pub access: DbAccess,
    pub file_path: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub user: Option<String>,
    pub tls: Option<DbTls>,
    pub ca_file: Option<String>,
    /// Where the password comes from. SQLite takes only `None`.
    pub password_source: SecretSource,
    /// A typed password, for Store or Ask; `None` keeps the one in the Keychain.
    pub password: Option<String>,
    pub statement_timeout_seconds: u32,
    pub runs_on: Option<RunsOn>,
}

// Written by hand instead of derived so a typed password never reaches a log.
impl std::fmt::Debug for SaveDbConnectionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveDbConnectionRequest")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("host", &self.host)
            .field("database", &self.database)
            .field("password_source", &self.password_source)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Test Connection: what the server said about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbTestResult {
    /// Such as "PostgreSQL 16.4" or "SQLite 3.50.2".
    pub server_version: String,
}

/// The fields of a pasted `postgres://` URL, to fill the form.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbUrlFields {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub user: Option<String>,
    /// The URL's password, moved into the form's password field.
    pub password: Option<String>,
    pub tls: Option<DbTls>,
}

impl std::fmt::Debug for DbUrlFields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbUrlFields")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("tls", &self.tls)
            .finish()
    }
}

/// What a column holds, for alignment and the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ColumnKind {
    Bool,
    Number,
    Numeric,
    Text,
    Json,
    Temporal,
    Uuid,
    Bytes,
    Array,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResultColumn {
    pub name: String,
    /// The database's name for the type, such as `int4` or `timestamptz`.
    pub type_name: String,
    pub kind: ColumnKind,
}

/// One value of a result. Most values travel as JSON scalars: `null`, a
/// boolean, a number (integers only up to 2^53), or the text the database
/// would print; values that need more say so in an object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum Cell {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Value(CellValue),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum CellValue {
    /// Text cut for the trip to the window; `length` is the whole value's in bytes.
    Cut {
        text: String,
        #[ts(type = "number")]
        length: u64,
    },
    /// Binary: its size and the hex of its first 4 KiB.
    Bytes {
        #[ts(type = "number")]
        size: u64,
        hex: String,
    },
    /// A type Brainiac cannot show: its size, and how to see it.
    Other {
        type_name: String,
        #[ts(type = "number")]
        size: u64,
    },
}

/// Why a statement did not complete. Shown under the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DbFailureReason {
    /// The database refused the statement.
    Sql,
    /// The statement tried to write on a read-only connection.
    ReadOnly,
    Cancelled,
    Timeout,
    /// The connection could not be opened or was lost.
    Connection,
    /// The statement has parameters, such as `$1`, and none were given.
    Parameters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbFailure {
    pub reason: DbFailureReason,
    /// The database's own message.
    pub message: String,
    /// PostgreSQL's SQLSTATE or SQLite's result code name.
    pub code: Option<String>,
    pub detail: Option<String>,
    pub hint: Option<String>,
    /// Where in the tab's text the error is, in UTF-16 code units.
    pub position: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum StatementResult {
    Rows {
        columns: Vec<ResultColumn>,
        rows: Vec<Vec<Cell>>,
        /// More rows were available than were fetched.
        more: bool,
    },
    /// A statement that returns no rows, such as `SET`: its command tag.
    Command {
        tag: String,
    },
    /// Explain and Explain Analyze.
    Plan {
        plan: PlanNode,
        planning_ms: Option<f64>,
        execution_ms: Option<f64>,
    },
    Failed {
        failure: DbFailure,
    },
}

/// One node of a query plan, as an indented tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlanNode {
    /// Such as `Index Scan using invoices_pkey on invoices`.
    pub label: String,
    pub startup_cost: Option<f64>,
    pub total_cost: Option<f64>,
    /// The planner's estimate.
    pub rows: Option<f64>,
    /// Explain Analyze: rows per loop and time of the last row, in ms.
    pub actual_rows: Option<f64>,
    pub actual_ms: Option<f64>,
    pub loops: Option<f64>,
    /// Conditions and other details, such as `Filter: (paid_at IS NULL)`.
    pub details: Vec<String>,
    pub children: Vec<PlanNode>,
}

/// How a tab runs statements (SPEC.md, Databases: Safety).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunMode {
    /// Each statement in its own read-only transaction.
    ReadOnly,
    /// Each statement commits.
    AutoCommit,
    /// The first statement opens a transaction that stays open until Commit or Roll Back.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExplainMode {
    Plan,
    /// Runs the statement to measure it.
    Analyze,
}

/// A tab's open transaction, in Manual mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TransactionInfo {
    /// Statements run in it so far.
    pub statements: u32,
    pub since: String,
    /// A statement failed: PostgreSQL refuses more until Roll Back.
    pub failed: bool,
}

/// A value for a `:name` parameter, as typed; `None` is `NULL`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParamValue {
    pub name: String,
    pub value: Option<String>,
}

/// One statement that ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StatementRun {
    /// The statement's place in the tab's text, in UTF-16 code units.
    pub from: u32,
    pub to: u32,
    pub sql: String,
    pub result: StatementResult,
    pub elapsed_ms: u32,
    /// The session was opened again first, losing session settings such as `SET`.
    pub reconnected: bool,
    /// The statement ran in a read-only transaction, so running it again
    /// (Fetch All, Export) cannot repeat a write.
    pub ran_read_only: bool,
    /// The tab's transaction after the statement, when one is open.
    pub transaction: Option<TransactionInfo>,
}

/// Run (`Cmd+Enter`): the statement under the cursor, or each statement in
/// the selection. Run All (`Shift+Cmd+Enter`): every statement in the text.
/// Statements run in turn and stop at the first failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunStatementRequest {
    /// The tab, which owns the session.
    pub tab_id: String,
    pub connection_id: String,
    pub text: String,
    /// The selection in UTF-16 code units; `from == to` is the cursor.
    pub from: u32,
    pub to: u32,
    /// Every statement in `text`, whatever the selection.
    pub all: bool,
    /// Fetch up to the most rows a result may have instead of the usual cap.
    pub fetch_all: bool,
    pub mode: RunMode,
    /// Values for the statements' `:name` parameters.
    pub parameters: Vec<ParamValue>,
    /// Explain the statement instead of running it.
    pub explain: Option<ExplainMode>,
}

/// Export…: a statement run again, read only, with every row written to a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExportRequest {
    pub tab_id: String,
    pub connection_id: String,
    pub sql: String,
    pub parameters: Vec<ParamValue>,
    pub format: ExportFormat,
    /// Chosen in the save dialog.
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExportFormat {
    Csv,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbExportResult {
    #[ts(type = "number")]
    pub rows: u64,
    pub path: String,
}

/// A saved query (SPEC.md, Databases: Saved queries).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    /// Such as `Billing` or `Billing/Monthly`; empty at the top.
    pub folder: String,
    pub description: String,
    /// The connection it runs on, if it still exists.
    pub connection_id: Option<String>,
    pub sql: String,
    /// The last values of its parameters, kept because they rarely change.
    pub parameters: Vec<ParamValue>,
    #[ts(type = "number")]
    pub version: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SaveQueryRequest {
    pub id: Option<String>,
    #[ts(type = "number | null")]
    pub expected_version: Option<i64>,
    pub name: String,
    pub folder: String,
    pub description: String,
    pub connection_id: Option<String>,
    pub sql: String,
}

/// One run in a connection's history: never its rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HistoryEntry {
    pub id: String,
    pub connection_id: String,
    pub sql: String,
    pub ran_at: String,
    pub elapsed_ms: u32,
    /// Rows returned or changed, when it succeeded.
    #[ts(type = "number | null")]
    pub rows: Option<i64>,
    /// The failure's message, when it failed.
    pub error: Option<String>,
}

/// An open query tab, kept so tabs and their text survive a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QueryTab {
    pub id: String,
    pub connection_id: Option<String>,
    pub saved_query_id: Option<String>,
    /// The saved query's name, or "Untitled n".
    pub title: String,
    pub text: String,
    /// The saved query's version the text was opened from or saved as.
    #[ts(type = "number | null")]
    pub saved_version: Option<i64>,
    /// Whether the text differs from the saved query's.
    pub dirty: bool,
    pub mode: RunMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RelationKind {
    Table,
    View,
    MaterializedView,
    ForeignTable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbColumn {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbIndex {
    pub name: String,
    /// Such as `CREATE UNIQUE INDEX … USING btree (email)`, or SQLite's columns.
    pub definition: String,
    pub unique: bool,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbForeignKey {
    pub name: String,
    /// Such as `FOREIGN KEY (customer_id) REFERENCES customers(id)`.
    pub definition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbRelation {
    pub name: String,
    pub kind: RelationKind,
    /// PostgreSQL's estimate of the rows, when it has analyzed the table.
    #[ts(type = "number | null")]
    pub estimated_rows: Option<i64>,
    pub columns: Vec<DbColumn>,
    pub indexes: Vec<DbIndex>,
    pub foreign_keys: Vec<DbForeignKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbSchemaGroup {
    /// PostgreSQL's schema, or SQLite's `main`.
    pub name: String,
    pub relations: Vec<DbRelation>,
}

/// What the schema browser and completion show for a connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbSchema {
    pub connection_id: String,
    pub schemas: Vec<DbSchemaGroup>,
    /// The schema an unqualified name means first, such as `public`.
    pub default_schema: Option<String>,
    pub read_at: String,
}

/// Where a PostgreSQL server runs, for the machine metrics PostgreSQL does
/// not report (SPEC.md, Databases: Health).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum RunsOn {
    /// Google Cloud SQL, read from Cloud Monitoring.
    CloudSql { project: String, instance: String },
    /// A Docker container on this Mac, read from Docker's API.
    Docker {
        container: String,
        /// The Docker socket; `None` tries Docker Desktop's, then the system one.
        socket: Option<String>,
    },
}

/// A running Docker container, for choosing what a connection runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DockerContainer {
    pub id: String,
    pub name: String,
    pub image: String,
    /// Such as `5432→5432`.
    pub ports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DockerContainers {
    pub socket: String,
    pub containers: Vec<DockerContainer>,
}

/// One session in Health's list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthSession {
    pub pid: i32,
    pub user: Option<String>,
    pub application: String,
    pub client: Option<String>,
    /// `active`, `idle`, `idle in transaction`, …
    pub state: Option<String>,
    /// Seconds in its state, and since its transaction began.
    pub state_seconds: Option<f64>,
    pub transaction_seconds: Option<f64>,
    /// Such as `Lock: transactionid`.
    pub wait: Option<String>,
    /// Sessions it waits for.
    pub blocked_by: Vec<i32>,
    /// Its current statement; `None` when the role may not see it.
    pub query: Option<String>,
    /// Brainiac's own sessions.
    pub own: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthStatement {
    pub query: String,
    #[ts(type = "number")]
    pub calls: i64,
    pub total_ms: f64,
    pub mean_ms: f64,
    #[ts(type = "number")]
    pub rows: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthTable {
    pub schema: String,
    pub name: String,
    /// Size with indexes and TOAST.
    #[ts(type = "number")]
    pub total_bytes: i64,
    #[ts(type = "number")]
    pub live_rows: i64,
    pub dead_ratio: Option<f64>,
    pub last_autovacuum: Option<String>,
}

/// Memory, CPU, and disk from where the server runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MachineSample {
    /// Such as "Docker" or "Cloud Monitoring · 1 min behind".
    pub source: String,
    #[ts(type = "number | null")]
    pub memory_used: Option<u64>,
    #[ts(type = "number | null")]
    pub memory_limit: Option<u64>,
    pub memory_ratio: Option<f64>,
    /// Share of the CPUs available to it, 0 to 1.
    pub cpu_ratio: Option<f64>,
    #[ts(type = "number | null")]
    pub disk_used: Option<u64>,
    #[ts(type = "number | null")]
    pub disk_quota: Option<u64>,
    /// Bytes per second (Docker reports I/O, not space used).
    pub disk_read_rate: Option<f64>,
    pub disk_write_rate: Option<f64>,
    /// Why the source could not be read this time.
    pub problem: Option<String>,
}

/// One reading of a server, every 10 seconds while Health is open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthSample {
    pub connection_id: String,
    pub at: String,
    pub max_connections: u32,
    pub active: u32,
    pub idle: u32,
    pub idle_in_transaction: u32,
    /// Every connection to the server, all databases.
    pub total_connections: u32,
    /// Blocks found in memory, 0 to 1, since the previous sample.
    pub cache_hit_ratio: Option<f64>,
    pub transactions_per_second: Option<f64>,
    pub rollbacks_per_second: Option<f64>,
    /// Since the previous sample.
    #[ts(type = "number")]
    pub deadlocks: i64,
    /// Temporary files written in the last hour, from queries that ran out of `work_mem`.
    #[ts(type = "number")]
    pub temp_files_hour: i64,
    pub longest_transaction_seconds: Option<f64>,
    pub sessions: Vec<HealthSession>,
    /// `None` when `pg_stat_statements` is not installed.
    pub statements: Option<Vec<HealthStatement>>,
    pub tables: Vec<HealthTable>,
    /// Whether the role sees other users' statements (`pg_monitor`).
    pub sees_all: bool,
    pub machine: Option<MachineSample>,
    /// Why the server could not be read this time.
    pub problem: Option<String>,
}

/// The values Health draws as a line over the last hour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthPoint {
    pub at: String,
    pub connections: u32,
    pub cache_hit_ratio: Option<f64>,
    pub transactions_per_second: Option<f64>,
    pub memory_ratio: Option<f64>,
    pub cpu_ratio: Option<f64>,
}

/// What Health shows when it opens: the hour so far and the latest sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthSnapshot {
    pub points: Vec<HealthPoint>,
    pub latest: Option<HealthSample>,
}

/// Emitted as `db_health_sample`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthEvent {
    pub sample: HealthSample,
    pub point: HealthPoint,
}

// ---------------------------------------------------------------------------
// v0.4.x: where secrets come from (SPEC.md, Secrets)
// ---------------------------------------------------------------------------

/// Where an account's token or a connection's password comes from. Saved
/// with the account or connection: it says where to look, never the value.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum SecretSource {
    /// Brainiac's own Keychain item, `brainiac/<owner>`: the one source it writes.
    Store,
    /// PostgreSQL: typed once per run of Brainiac and kept in memory.
    Ask,
    /// No secret: SQLite, or a server that trusts local users.
    None,
    /// A variable of Brainiac's own environment, as it was at launch.
    Environment { name: String },
    /// What a program prints: its full path and each argument on its own,
    /// started without a shell.
    Command { program: String, args: Vec<String> },
}

// Written by hand so a command's arguments, which may name where a secret
// is, never reach a log through a derived `Debug`.
impl std::fmt::Debug for SecretSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretSource::Store => f.write_str("Store"),
            SecretSource::Ask => f.write_str("Ask"),
            SecretSource::None => f.write_str("None"),
            SecretSource::Environment { name } => {
                f.debug_struct("Environment").field("name", name).finish()
            }
            SecretSource::Command { program, args } => f
                .debug_struct("Command")
                .field("program", program)
                .field("args", &format_args!("<{} redacted>", args.len()))
                .finish(),
        }
    }
}

/// A change to a credential that did not finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CredentialPending {
    /// A save that writes the Keychain item was cut off. Nothing is read
    /// until the account or connection is saved again.
    Save,
    /// The old Keychain item is still to be deleted after a move to another
    /// source; the new source is in use.
    Cleanup,
    /// Being removed; its Keychain item is still to be deleted.
    Removal,
}

/// A credential's state, never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CredentialState {
    /// Restored from a backup: the source is not read until the user allows it.
    pub needs_approval: bool,
    pub pending: Option<CredentialPending>,
    /// Advanced when the source, where the secret goes, or the stored secret
    /// changes; approving names the revision that was shown.
    #[ts(type = "number")]
    pub revision: i64,
}

/// Whose secret: an account or a connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum CredentialOwner {
    ForgeAccount {
        provider: ForgeKind,
    },
    DbConnection {
        id: String,
    },
    /// Settings → Agents: the token or key a profile's runs are paid with.
    AgentProfile {
        id: String,
    },
}

/// The outcome of the last Test of a saved source, in this run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CredentialTest {
    pub at: String,
    pub ok: bool,
    /// What the test found, without any secret: "Connected: PostgreSQL 16.4."
    pub message: String,
}

/// One row of Settings → Secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretEntry {
    pub owner: CredentialOwner,
    /// "GitHub", or the connection's name.
    pub label: String,
    /// Where Brainiac sends the secret: "api.github.com as octo",
    /// "db.example.com:5432/app as app".
    pub destination: String,
    pub source: SecretSource,
    pub state: CredentialState,
    /// Ask: nothing typed in this run yet.
    pub input_required: bool,
    pub last_test: Option<CredentialTest>,
}

/// Settings → Secrets: the store in use and every account and connection
/// with a secret. Listing it reads no secret and runs no program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretsOverview {
    /// Such as "macOS login keychain".
    pub store: String,
    pub entries: Vec<SecretEntry>,
}

/// Test on an account's form: who the token belongs to and what it lacks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForgeAccountTestResult {
    pub login: String,
    pub missing: Vec<String>,
}

// ---------------------------------------------------------------------------
// Agent runs — v0.5 (SPEC.md, section 13)
// ---------------------------------------------------------------------------

/// How a profile's runs are paid for: exactly one of them reaches a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentPayment {
    /// A token the user made with `claude setup-token`, on their Claude plan.
    ClaudePlan,
    /// An Anthropic API key.
    ApiKey,
}

impl AgentPayment {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentPayment::ClaudePlan => "claude_plan",
            AgentPayment::ApiKey => "api_key",
        }
    }
}

/// A new run's default permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunPermissions {
    /// Ask before actions: the agent waits for the user before it runs a
    /// command or edits a file.
    Ask,
    /// Act without asking: anything inside its container.
    Act,
}

impl RunPermissions {
    pub fn as_str(self) -> &'static str {
        match self {
            RunPermissions::Ask => "ask",
            RunPermissions::Act => "act",
        }
    }
}

/// The image built for a profile on its engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentImage {
    /// Such as `brainiac-claude:3f9c41e1a2b0`.
    pub name: String,
    /// The engine's image ID, `sha256:…`.
    pub id: String,
    pub built_at: String,
    /// Built from this version of Brainiac's Dockerfile and its files; false
    /// after an update changed them, until it is rebuilt.
    pub current: bool,
}

/// Settings → Agents for one agent (phase 1: Claude Code). Never holds the
/// token or key, only where it is read from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentProfile {
    pub id: String,
    /// The Docker socket of this Mac's engine; `None` until chosen.
    pub engine_socket: Option<String>,
    pub payment: AgentPayment,
    /// Where the token or key is read from; `None` when there is none yet.
    pub credential_source: SecretSource,
    pub credential: CredentialState,
    /// When the token or key was last saved.
    pub credential_saved_at: Option<String>,
    /// A Claude plan token saved eleven months ago or more: tokens last a year.
    pub credential_ageing: bool,
    /// The user agreed that runs send code and prompts to the provider under
    /// this payment.
    pub sends_code_agreed: bool,
    pub permissions: RunPermissions,
    pub time_limit_minutes: u32,
    pub cpus: u32,
    pub memory_mib: u32,
    pub workspace_gib: u32,
    /// The model new runs ask Claude Code for: an alias such as `sonnet` or
    /// a full name; empty for Claude Code's own default.
    pub model: String,
    pub image: Option<AgentImage>,
    /// When the last test passed.
    pub test_passed_at: Option<String>,
    /// That test ran with the current token or key, image, and engine, so
    /// runs may start.
    pub test_current: bool,
    #[ts(type = "number")]
    pub version: i64,
}

/// Settings → Agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentSettings {
    pub profile: AgentProfile,
    /// Whether this build offers paying with a Claude plan (SPEC.md,
    /// Settings → Agents: it ships only once Anthropic's answer is recorded).
    pub plan_offered: bool,
    /// What still stops a run from starting, in the order to do it.
    pub missing: Vec<String>,
    /// This Mac, then every remote host (SPEC.md, Remote hosts).
    pub hosts: Vec<AgentHost>,
}

/// A machine that can run an agent (SPEC.md, Remote hosts). Never a private
/// key and never the agent's token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentHost {
    pub id: String,
    /// `local` or `ssh`.
    pub kind: String,
    pub name: String,
    pub ssh_user: Option<String>,
    pub ssh_host: Option<String>,
    pub ssh_port: Option<u16>,
    pub identity_path: Option<String>,
    /// The host key the user approved.
    pub fingerprint: Option<String>,
    /// The user confirmed this host. A restored host waits until they do.
    pub approved: bool,
    /// The controller Deploy installed, once it has answered.
    pub installed: bool,
    pub engine_name: Option<String>,
    /// The engine can attach the workspace's loop devices.
    pub loop_devices: bool,
    pub image: Option<AgentImage>,
    pub test_current: bool,
    /// A command the host's administrator can run to stop containers.
    pub emergency_stop: Option<String>,
    /// Remove finished, and the host still holds a container or a volume.
    pub state_kept: bool,
    #[ts(type = "number")]
    pub version: i64,
}

/// The host key a new host presented, before anything is saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentHostPreview {
    pub fingerprint: String,
    /// What Deploy will run with `sudo`, shown first.
    pub actions: Vec<String>,
}

/// Approve a host key and save the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApproveAgentHostRequest {
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    pub identity_path: Option<String>,
    /// The fingerprint the preview showed.
    pub fingerprint: String,
    /// Set when the host already has a different fingerprint.
    pub accept_changed_key: bool,
}

/// Settings → Agents, everything but the token or key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SaveAgentSettingsRequest {
    /// The version the pane shows; a save over a newer one is refused.
    #[ts(type = "number")]
    pub expected_version: i64,
    pub engine_socket: Option<String>,
    pub sends_code_agreed: bool,
    pub permissions: RunPermissions,
    pub time_limit_minutes: u32,
    pub cpus: u32,
    pub memory_mib: u32,
    pub workspace_gib: u32,
    pub model: String,
}

/// Settings → Agents, **Pay with**: the token or key and where it comes from.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SaveAgentCredentialRequest {
    #[ts(type = "number")]
    pub expected_version: i64,
    pub payment: AgentPayment,
    /// The Keychain, an environment variable, or a command.
    pub source: SecretSource,
    /// The Keychain: the pasted token or key; `None` keeps the one saved.
    pub secret: Option<String>,
}

// Written by hand instead of derived so a pasted token never reaches a log.
impl std::fmt::Debug for SaveAgentCredentialRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveAgentCredentialRequest")
            .field("expected_version", &self.expected_version)
            .field("payment", &self.payment)
            .field("source", &self.source)
            .field("secret", &self.secret.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// A Docker engine found on this Mac, by its socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentEngine {
    pub socket: String,
    /// "OrbStack", "Docker Desktop", or what the engine calls itself.
    pub name: String,
    /// Answered on this socket.
    pub reachable: bool,
    /// Runs can use it: a tested engine that answered.
    pub supported: bool,
    /// Why it cannot be used, when it cannot.
    pub problem: Option<String>,
    pub server_version: Option<String>,
    pub api_version: Option<String>,
    pub cpus: Option<u32>,
    #[ts(type = "number | null")]
    pub memory_bytes: Option<u64>,
}

/// New run, **Start from**: the commit a run would get, read from the
/// repository without changing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunStartPreview {
    pub repository_id: String,
    /// The full commit ID the start resolved to, once.
    pub commit: String,
    pub subject: String,
    pub author: String,
    pub committed_at: String,
    /// Commits the container gets: this one and its history.
    pub history_commits: u32,
}

/// What the agent is doing (SPEC.md, The run: States).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunActivity {
    Preparing,
    Working,
    /// Waiting for you: a permission.
    Permission,
    /// Ready for your prompt.
    Idle,
    /// Plan limit reached; the session is still open.
    PlanLimit,
    Stopping,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunPhase {
    Preparing,
    Running,
    Stopping,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunOutcome {
    Finished,
    Cancelled,
    Expired,
    Failed,
    /// The engine, the container, or the run controller stopped unexpectedly.
    Interrupted,
}

/// What the run produced (SPEC.md, The run: States).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunCollection {
    /// Not collected: the run is live, or it was interrupted and waits for Collect work.
    None,
    Collecting,
    /// Ready to review: a snapshot commit on top of the start.
    Ready,
    NoChanges,
    Failed,
}

impl RunCollection {
    pub fn as_str(self) -> &'static str {
        match self {
            RunCollection::None => "none",
            RunCollection::Collecting => "collecting",
            RunCollection::Ready => "ready",
            RunCollection::NoChanges => "no_changes",
            RunCollection::Failed => "failed",
        }
    }
}

/// A new file the snapshot left out, and why (SPEC.md, Ending and collecting: Left out).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LeftOutFile {
    pub path: String,
    pub reason: String,
}

/// A permission the agent waits for (SPEC.md, The run: Permission requests).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunPermissionRequest {
    pub permission_id: String,
    pub turn: u32,
    pub title: String,
    pub kind: Option<String>,
    /// The command or the file, when the agent said which.
    pub detail: Option<String>,
    pub asked_at: String,
    /// The edit asked for, as the agent describes it. Only the conversation
    /// carries it; a run's pending list leaves it out.
    #[serde(default)]
    pub diffs: Vec<RunFileDiff>,
}

/// An edit as the agent reports it (SPEC.md, The run: Conversation): a
/// file's text before and after, or only the part it replaces. Reported by
/// the agent, not read from its workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunFileDiff {
    /// As the agent names it, inside its container (`/workspace/…`).
    pub path: String,
    /// None for a new or rewritten file.
    pub old_text: Option<String>,
    pub new_text: String,
    /// A text was cut short.
    #[serde(default)]
    pub truncated: bool,
}

/// One agent run (SPEC.md, section 13): its start, what it runs with, the
/// run controller's last confirmed state, and what was collected. Never the
/// token or key, only where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentRun {
    pub id: String,
    pub repository_id: String,
    pub repository_name: String,
    /// The first prompt's first line, cut short.
    pub title: String,
    pub start_commit: String,
    pub start_subject: String,
    pub payment: AgentPayment,
    /// Where the credential came from, in words ("the Keychain").
    pub credential_source: String,
    pub host_id: String,
    pub host_name: String,
    pub engine_name: String,
    pub image_name: String,
    pub permissions: RunPermissions,
    pub time_limit_minutes: u32,
    pub cpus: u32,
    pub memory_mib: u32,
    pub workspace_gib: u32,
    /// The model the run asked for; empty for Claude Code's default.
    pub model: String,
    /// The model the agent reported once its session opened.
    pub model_used: Option<String>,
    pub phase: RunPhase,
    pub activity: RunActivity,
    pub turn: u32,
    pub outcome: Option<RunOutcome>,
    /// The engine confirmed that nothing of the run runs.
    pub stop_confirmed: bool,
    /// The stopped container and its files are still on the engine.
    pub kept: bool,
    pub accepted_at: Option<String>,
    /// When the time limit ends the run.
    pub deadline_at: Option<String>,
    pub ended_at: Option<String>,
    /// The deadline passed while this Mac slept.
    pub expired_asleep: bool,
    /// Why the run failed or could not stop, in words safe to show.
    pub error: Option<String>,
    pub pending_permissions: Vec<RunPermissionRequest>,
    /// Brainiac reaches the run controller now; otherwise `reported_at`
    /// says when it last did.
    pub connected: bool,
    pub reported_at: Option<String>,
    /// Cancel was asked for while the run could not be reached; it is sent on reconnect.
    pub cancel_requested: bool,
    pub collection: RunCollection,
    pub collection_error: Option<String>,
    /// The snapshot commit in Brainiac's repository; the start when nothing changed.
    pub result_commit: Option<String>,
    pub changed_files: Option<u32>,
    pub left_out: Vec<LeftOutFile>,
    pub left_out_more: u32,
    /// Keep this snapshot, or a patch export, confirmed the left-out list.
    pub snapshot_accepted: bool,
    /// A container, volume, or file that could not be removed.
    pub cleanup_pending: Option<String>,
    /// The journal sequence mirrored so far.
    #[ts(type = "number")]
    pub cursor: u64,
    pub created_at: String,
    pub updated_at: String,
    #[ts(type = "number")]
    pub version: i64,
}

/// Runs, and whether the run controller is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentRunList {
    pub runs: Vec<AgentRun>,
    pub controller_running: bool,
}

/// New run, **Start run** (SPEC.md, New run).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StartRunRequest {
    pub repository_id: String,
    /// From the preview: the commit the start resolved to.
    pub start_commit: String,
    pub prompt: String,
    pub permissions: RunPermissions,
    pub time_limit_minutes: u32,
    pub cpus: u32,
    pub memory_mib: u32,
    pub workspace_gib: u32,
    /// The model for this run; empty for Claude Code's default.
    pub model: String,
    /// Empty means this Mac.
    #[serde(default)]
    pub host_id: String,
}

/// One entry of a run's conversation, as the journal recorded it:
/// normalized and filtered, never the agent's raw protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunEvent {
    #[ts(type = "number")]
    pub seq: u64,
    pub at: String,
    #[serde(flatten)]
    pub body: RunEventBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunPlanEntry {
    pub content: String,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum RunEventBody {
    Accepted {
        deadline_at: String,
        permissions: RunPermissions,
    },
    Ready {
        session_id: String,
        agent: String,
        version: String,
        /// The model the session opened with, when the agent reports it.
        model: Option<String>,
    },
    Prompt {
        turn: u32,
        command_id: String,
        text: String,
    },
    /// A piece of the agent's reply; consecutive pieces of one turn join.
    Message {
        turn: u32,
        text: String,
    },
    Thought {
        turn: u32,
        text: String,
    },
    Plan {
        turn: u32,
        entries: Vec<RunPlanEntry>,
    },
    /// A tool call, or a change to one: fields the update did not carry are absent.
    Tool {
        turn: u32,
        tool_id: String,
        title: Option<String>,
        kind: Option<String>,
        status: Option<String>,
        locations: Vec<String>,
        output: Option<String>,
        /// The edits the tool reports; empty when the update carried none.
        #[serde(default)]
        diffs: Vec<RunFileDiff>,
    },
    Permission(RunPermissionRequest),
    PermissionAnswered {
        permission_id: String,
        /// `allowed`, `rejected`, or `cancelled`.
        outcome: String,
        /// `user`, `auto` (Act without asking), or `run` (the run ended).
        by: String,
    },
    TurnEnded {
        turn: u32,
        reason: String,
        message: Option<String>,
    },
    Notice {
        text: String,
    },
    Stopping {
        outcome: RunOutcome,
    },
    Ended {
        outcome: RunOutcome,
        message: Option<String>,
    },
}

/// A page of a run's conversation after a sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunEventPage {
    pub run_id: String,
    pub events: Vec<RunEvent>,
    /// The last sequence mirrored; more follow while `events` stops before it.
    #[ts(type = "number")]
    pub cursor: u64,
}

/// Changes: the collected snapshot against the start (SPEC.md, Review).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunChanges {
    pub run_id: String,
    pub start_commit: String,
    pub result_commit: String,
    pub files: Vec<CommitFile>,
}

/// One file's diff of a run's snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunDiffRequest {
    pub run_id: String,
    pub path: String,
    pub old_path: Option<String>,
    #[serde(default)]
    pub options: DiffOptions,
    /// From the live run's latest preview (Changes so far), not the collected snapshot.
    #[serde(default)]
    pub preview: bool,
}

/// **Changes so far** (SPEC.md, The run): a provisional snapshot of a live
/// run's working tree, taken while the agent works.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunPreview {
    pub run_id: String,
    pub start_commit: String,
    /// The provisional snapshot commit in Brainiac's repository; the start
    /// when nothing had changed. None until one was taken.
    pub commit: Option<String>,
    pub taken_at: Option<String>,
    /// The run's turn when it was taken.
    pub turn: u32,
    pub files: Vec<CommitFile>,
    /// New files the collection would leave out, by the same rules.
    pub left_out: u32,
    /// One is being taken now.
    pub busy: bool,
    /// Why the last one could not be taken; the earlier one stays.
    pub error: Option<String>,
}

/// **Copy branch command** (SPEC.md, Review): a command the user runs in
/// their own repository; Brainiac does not run it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunBranchCommand {
    pub branch: String,
    pub command: String,
}

/// Settings → Agents, **Test**: what passed, and when (SPEC.md, Settings → Agents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentTestResult {
    /// Each step in order, with whether it passed.
    pub steps: Vec<AgentTestStep>,
    pub passed: bool,
    pub tested_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentTestStep {
    pub name: String,
    pub passed: bool,
    pub detail: Option<String>,
}

/// Settings → Agents: whether the run controller is running (SPEC.md, Settings → Agents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunControllerStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub live_runs: u32,
}

/// An event telling the window that a run changed: its state, its
/// conversation, or its collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentRunChangedEvent {
    pub run_id: String,
    /// The run was deleted.
    pub deleted: bool,
}
