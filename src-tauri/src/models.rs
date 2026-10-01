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

/// Stable error codes shared with the frontend (SPEC §14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export)]
pub enum ErrorCode {
    Validation,
    NotFound,
    Conflict,
    PermissionDenied,
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
    /// Argument template for opening a repository root; `{path}` is substituted.
    pub repo_args: Vec<String>,
    /// Argument template for opening a file; `{path}` and `{line}` are substituted.
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PinEntityType {
    Repository,
    Workspace,
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
}

impl DiffSelector {
    pub fn path(&self) -> &str {
        match self {
            DiffSelector::IndexVsHead { path }
            | DiffSelector::WorktreeVsIndex { path }
            | DiffSelector::UntrackedPreview { path }
            | DiffSelector::Commit { path, .. } => path,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RefsResult {
    pub repository_id: String,
    pub refs: Vec<RefEntry>,
}

// ---------------------------------------------------------------------------
// Workspaces (types only in M0; behavior arrives in v0.1)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiscoveryMode {
    /// Members were found inside a chosen folder (SPEC §7).
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

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
