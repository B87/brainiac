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
}

impl DiffSelector {
    pub fn path(&self) -> &str {
        match self {
            DiffSelector::IndexVsHead { path }
            | DiffSelector::WorktreeVsIndex { path }
            | DiffSelector::WorktreeVsHead { path }
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
