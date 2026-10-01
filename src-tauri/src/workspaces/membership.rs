//! Workspaces: discovery previews, creation, membership changes, and pins (SPEC.md, Workspaces and repositories).
//!
//! This is a child module of `workspaces`, so the `impl RepositoryService`
//! block below can use the service's private fields (`db`, `git`) directly:
//! Rust privacy is per module, and child modules see their parent's private items.
//!
//! Rules decided here:
//! - A path is a repository member only when Git reports it as its own
//!   working-tree root (normal checkout, linked worktree, or submodule). A
//!   folder that is not in any repository, or that only inherits an enclosing
//!   repository's Git context, becomes a non-Git member (`repository_id` NULL).
//! - Paths must exist and be absolute folders; otherwise the whole request
//!   fails with `NOT_FOUND`/`VALIDATION` and nothing is written.
//! - Repository registrations are reused by display path (the same rule as
//!   `register`) and are never deleted by workspace operations.
//! - There is no workspace change event. Commands return the definitive
//!   `Workspace`; the snapshot version is bumped so a later snapshot supersedes
//!   older ones. Newly registered repositories get a background refresh, which
//!   emits the usual `repository_changed` events.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use super::RepositoryService;
use crate::db::{self, MemberRow, RepositoryRow, WorkspaceRow};
use crate::git::ResolvedRepository;
use crate::models::{
    now_rfc3339, ActivitySettings, AppError, AppResult, ChangeOrigin, CreateWorkspaceRequest,
    DiscoveryMode, ErrorCode, MemberOrigin, MemberStatus, PinEntityType, PreviewStatus,
    UpdateWorkspaceMembershipRequest, Workspace, WorkspaceMember, WorkspacePreview,
    WorkspacePreviewEntry,
};

/// What a workspace mutation produced. The command layer starts file watchers
/// for `new_repositories`, which this service cannot do without Tauri state.
#[derive(Debug)]
pub struct WorkspaceChange {
    pub workspace: Workspace,
    pub new_repositories: Vec<RepositoryRow>,
}

/// A requested folder after validation and Git classification.
struct PlannedMember {
    canonical_path: PathBuf,
    display_name: String,
    origin: MemberOrigin,
    /// `Some` when the folder is its own Git working-tree root.
    repository: Option<ResolvedRepository>,
}

/// Where a discovered workspace looks for members: the selected folder and the
/// scanned folder inside it. Used to tell discovered from manual additions.
struct DiscoveryScope {
    root: PathBuf,
    scan: PathBuf,
}

impl DiscoveryScope {
    fn origin_of(&self, path: &Path) -> MemberOrigin {
        if path == self.root || path.parent() == Some(self.scan.as_path()) {
            MemberOrigin::Discovered
        } else {
            MemberOrigin::Manual
        }
    }
}

impl RepositoryService {
    // -----------------------------------------------------------------------
    // Discovery preview
    // -----------------------------------------------------------------------

    /// Preview a workspace built from `folder_path`.
    ///
    /// `entries` holds the root first (when the folder is a repository, `ok`;
    /// when it sits inside another repository, `nested`), then every immediate
    /// child folder of the discovery folder sorted by name. Children that cannot
    /// be tracked stay in `entries` with a non-`ok` status and a `message`, so
    /// the UI can explain why they were skipped; `candidates` is left empty.
    /// Hidden (dot) folders and plain files are omitted. Symbolic links are not
    /// followed and are reported as `unsupported`. A missing discovery folder is
    /// reported as a `missing` entry rather than an error, so the root can still
    /// be tracked.
    pub async fn discover_repositories(
        &self,
        folder_path: &str,
        discovery_path: Option<&str>,
    ) -> AppResult<WorkspacePreview> {
        let root = existing_folder(folder_path).await?;
        let rel = normalize_discovery_path(discovery_path)?;
        let git = self.git()?;

        // `HashMap` from display path to ID, built once instead of one query per child.
        let registered: HashMap<String, String> = self
            .rows()
            .await?
            .into_iter()
            .map(|r| (r.display_path, r.id))
            .collect();
        let existing_id = |root: &Path| registered.get(&root.display().to_string()).cloned();

        let mut entries = Vec::new();
        let mut seen_roots: HashSet<PathBuf> = HashSet::new();

        // The selected folder itself.
        let enclosing = resolve_optional(git, &root).await?;
        if let Some(r) = &enclosing {
            let is_root = r.root == root;
            entries.push(WorkspacePreviewEntry {
                display_name: folder_name(&root),
                configured_path: root.display().to_string(),
                resolved_path: Some(root.display().to_string()),
                status: if is_root {
                    PreviewStatus::Ok
                } else {
                    PreviewStatus::Nested
                },
                repository_root: Some(r.root.display().to_string()),
                existing_repository_id: existing_id(&r.root),
                message: (!is_root).then(|| {
                    format!(
                        "This folder is inside the repository at {}; it is not a repository root.",
                        r.root.display()
                    )
                }),
            });
            if is_root {
                seen_roots.insert(r.root.clone());
            }
        }

        // The folder to scan.
        let scan = match &rel {
            None => root.clone(),
            Some(rel) => {
                let joined = root.join(rel);
                match tokio::fs::canonicalize(&joined).await {
                    Ok(c) if !c.starts_with(&root) => {
                        return Err(AppError::validation(
                            "The discovery folder must stay inside the selected folder.",
                        ));
                    }
                    Ok(c) if !c.is_dir() => {
                        return Err(AppError::validation(format!(
                            "{} is not a folder.",
                            joined.display()
                        )));
                    }
                    Ok(c) => c,
                    Err(_) => {
                        entries.push(WorkspacePreviewEntry {
                            display_name: rel.clone(),
                            configured_path: joined.display().to_string(),
                            resolved_path: None,
                            status: PreviewStatus::Missing,
                            repository_root: None,
                            existing_repository_id: None,
                            message: Some(format!(
                                "The discovery folder {} does not exist.",
                                joined.display()
                            )),
                        });
                        return Ok(preview(&root, entries));
                    }
                }
            }
        };
        // Which repository a plain child folder would belong to, for the skip message.
        let scan_owner = if scan == root {
            enclosing.map(|r| r.root)
        } else {
            resolve_optional(git, &scan).await?.map(|r| r.root)
        };

        let mut children = Vec::new();
        let mut dir = tokio::fs::read_dir(&scan).await?;
        while let Some(entry) = dir.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            // `file_type` does not follow symbolic links, so a link is seen as a link.
            let file_type = entry.file_type().await?;
            if file_type.is_symlink() {
                if tokio::fs::metadata(entry.path())
                    .await
                    .is_ok_and(|m| m.is_dir())
                {
                    children.push((name, true));
                }
            } else if file_type.is_dir() {
                children.push((name, false));
            }
        }
        children.sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then(a.0.cmp(&b.0))
        });

        for (name, is_link) in children {
            let path = scan.join(&name);
            let mut entry = WorkspacePreviewEntry {
                display_name: name,
                configured_path: path.display().to_string(),
                resolved_path: Some(path.display().to_string()),
                status: PreviewStatus::NotGit,
                repository_root: None,
                existing_repository_id: None,
                message: None,
            };
            if is_link {
                entry.resolved_path = None;
                entry.status = PreviewStatus::Unsupported;
                entry.message = Some(
                    "Symbolic links are not followed during discovery. Add the target folder manually."
                        .into(),
                );
                entries.push(entry);
                continue;
            }
            // A working-tree root always has a `.git` directory or file, so
            // folders without one are classified without starting Git.
            let resolved = if tokio::fs::symlink_metadata(path.join(".git")).await.is_ok() {
                match git.resolve(&path).await {
                    Ok(r) => Some(r),
                    Err(e) if e.code == ErrorCode::Validation => None,
                    Err(e) => {
                        entry.status = PreviewStatus::Unsupported;
                        entry.message = Some(e.message);
                        entries.push(entry);
                        continue;
                    }
                }
            } else {
                None
            };
            match resolved {
                Some(r) if r.root == path => {
                    entry.repository_root = Some(r.root.display().to_string());
                    entry.existing_repository_id = existing_id(&r.root);
                    if seen_roots.insert(r.root.clone()) {
                        entry.status = PreviewStatus::Ok;
                    } else {
                        entry.status = PreviewStatus::Duplicate;
                        entry.message =
                            Some("This repository is already listed in this preview.".into());
                    }
                }
                other => {
                    let owner = other.map(|r| r.root).or_else(|| scan_owner.clone());
                    entry.message = Some(match owner {
                        Some(owner) => format!(
                            "Not a separate repository; it belongs to {}.",
                            owner.display()
                        ),
                        None => "Not a Git repository.".into(),
                    });
                }
            }
            entries.push(entry);
        }
        Ok(preview(&root, entries))
    }

    // -----------------------------------------------------------------------
    // Reads
    // -----------------------------------------------------------------------

    /// All workspaces with their members, ordered by name ignoring case.
    pub async fn workspaces(&self) -> AppResult<Vec<Workspace>> {
        let (rows, members, mut activity, unseen) = self
            .db
            .call(|conn| {
                Ok((
                    db::list_workspaces(conn)?,
                    db::list_members(conn, None)?,
                    db::activity_settings(conn)?,
                    Unread {
                        stores: db::repository_stores(conn)?,
                        events: db::unseen_events(conn)?,
                    },
                ))
            })
            .await?;
        let mut by_workspace: HashMap<String, Vec<MemberRow>> = HashMap::new();
        for m in members {
            by_workspace
                .entry(m.workspace_id.clone())
                .or_default()
                .push(m);
        }
        Ok(rows
            .into_iter()
            .map(|row| {
                let members = by_workspace.remove(&row.id).unwrap_or_default();
                let settings = activity.remove(&row.id).unwrap_or_default();
                to_workspace(row, members, settings, &unseen)
            })
            .collect())
    }

    pub async fn workspace(&self, id: &str) -> AppResult<Workspace> {
        let id2 = id.to_string();
        let (row, members, mut activity, unseen) = self
            .db
            .call(move |conn| {
                let row = db::get_workspace(conn, &id2)?;
                let members = db::list_members(conn, Some(&id2))?;
                Ok((
                    row,
                    members,
                    db::activity_settings(conn)?,
                    Unread {
                        stores: db::repository_stores(conn)?,
                        events: db::unseen_events(conn)?,
                    },
                ))
            })
            .await?;
        let row = row.ok_or_else(workspace_not_found)?;
        let settings = activity.remove(&row.id).unwrap_or_default();
        Ok(to_workspace(row, members, settings, &unseen))
    }

    // -----------------------------------------------------------------------
    // Mutations
    // -----------------------------------------------------------------------

    /// Create a workspace, registering every selected repository that is not
    /// registered yet. Everything is written in one transaction.
    pub async fn create_workspace(
        self: &Arc<Self>,
        request: CreateWorkspaceRequest,
    ) -> AppResult<WorkspaceChange> {
        let name = valid_name(&request.name)?;
        // Discovery fields only mean something for a discovered workspace; a
        // manual workspace ignores them.
        let (scope, discovery_path) = match request.discovery_mode {
            DiscoveryMode::Manual => (None, None),
            DiscoveryMode::Discovered => {
                let root = request.discovery_root.as_deref().ok_or_else(|| {
                    AppError::validation("A discovered workspace needs a discovery folder.")
                })?;
                let root = existing_folder(root).await?;
                let rel = normalize_discovery_path(request.discovery_path.as_deref())?;
                let scan = match &rel {
                    Some(rel) => canonical_or_same(root.join(rel)).await,
                    None => root.clone(),
                };
                (Some(DiscoveryScope { root, scan }), rel)
            }
        };
        let planned = self.plan_members(&request.paths, scope.as_ref()).await?;
        let root_path = scope.as_ref().map(|s| s.root.clone());

        let workspace = WorkspaceRow {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            discovery_mode: request.discovery_mode,
            root_repository_id: None,
            discovery_root: root_path.as_ref().map(|p| p.display().to_string()),
            discovery_path,
            created_at: now_rfc3339(),
        };
        let workspace_id = workspace.id.clone();
        // `move` hands ownership of `workspace` and `planned` to the closure,
        // which runs later on the database thread.
        let new_repositories = self
            .db
            .call(move |conn| {
                // Dropping a transaction without `commit` rolls it back, so an
                // error returned by `?` leaves the database untouched.
                let tx = conn.transaction()?;
                let mut workspace = workspace;
                db::insert_workspace(&tx, &workspace)?;
                let (new_rows, root_id) =
                    insert_members(&tx, &workspace.id, planned, root_path.as_deref())?;
                if root_id.is_some() {
                    workspace.root_repository_id = root_id;
                    db::set_workspace_root(
                        &tx,
                        &workspace.id,
                        workspace.root_repository_id.as_deref(),
                    )?;
                }
                tx.commit()?;
                Ok(new_rows)
            })
            .await?;
        self.finish_change(&workspace_id, new_repositories).await
    }

    /// Add and remove members. Removal drops the membership only; the
    /// repository stays registered. Removing the root member clears the
    /// workspace root; adding the discovery root back restores it. Paths in
    /// `remove` that are not members are ignored. Removals apply before additions.
    pub async fn update_workspace_membership(
        self: &Arc<Self>,
        request: UpdateWorkspaceMembershipRequest,
    ) -> AppResult<WorkspaceChange> {
        let workspace_id = request.workspace_id.clone();
        let row = {
            let id = workspace_id.clone();
            self.db
                .call(move |conn| db::get_workspace(conn, &id))
                .await?
                .ok_or_else(workspace_not_found)?
        };
        let scope = match (&row.discovery_mode, &row.discovery_root) {
            (DiscoveryMode::Discovered, Some(root)) => {
                let root = PathBuf::from(root);
                let scan = match &row.discovery_path {
                    Some(rel) => canonical_or_same(root.join(rel)).await,
                    None => root.clone(),
                };
                Some(DiscoveryScope { root, scan })
            }
            _ => None,
        };
        let planned = self.plan_members(&request.add, scope.as_ref()).await?;
        let root_path = scope.map(|s| s.root);
        let remove = request.remove;

        let new_repositories = self
            .db
            .call(move |conn| {
                let tx = conn.transaction()?;
                let mut root_id = row.root_repository_id.clone();
                for path in &remove {
                    if let Some(m) = db::delete_member(&tx, &row.id, path)? {
                        if m.repository_id.is_some() && m.repository_id == root_id {
                            root_id = None;
                        }
                    }
                }
                let (new_rows, added_root) =
                    insert_members(&tx, &row.id, planned, root_path.as_deref())?;
                if root_id.is_none() {
                    root_id = added_root;
                }
                if root_id != row.root_repository_id {
                    db::set_workspace_root(&tx, &row.id, root_id.as_deref())?;
                }
                tx.commit()?;
                Ok(new_rows)
            })
            .await?;
        self.finish_change(&workspace_id, new_repositories).await
    }

    pub async fn rename_workspace(&self, id: &str, name: &str) -> AppResult<Workspace> {
        let name = valid_name(name)?;
        let id2 = id.to_string();
        let renamed = self
            .db
            .call(move |conn| db::rename_workspace(conn, &id2, &name))
            .await?;
        if !renamed {
            return Err(workspace_not_found());
        }
        self.bump_version();
        self.workspace(id).await
    }

    /// Delete a workspace and its pin. Member repositories stay registered.
    pub async fn remove_workspace(&self, id: &str) -> AppResult<()> {
        let id2 = id.to_string();
        let removed = self
            .db
            .call(move |conn| db::delete_workspace(conn, &id2))
            .await?;
        if !removed {
            return Err(workspace_not_found());
        }
        self.bump_version();
        Ok(())
    }

    /// Pin or unpin a repository or workspace. Pinning appends to the end.
    pub async fn set_pinned(
        &self,
        entity_type: PinEntityType,
        entity_id: &str,
        pinned: bool,
    ) -> AppResult<()> {
        let id = entity_id.to_string();
        let found = self
            .db
            .call(move |conn| {
                let exists = match entity_type {
                    PinEntityType::Repository => db::get_repository(conn, &id)?.is_some(),
                    PinEntityType::Workspace => db::get_workspace(conn, &id)?.is_some(),
                };
                // Unpinning something already deleted is harmless; pinning it is not.
                if exists || !pinned {
                    db::set_pinned(conn, entity_type, &id, pinned)?;
                }
                Ok(exists || !pinned)
            })
            .await?;
        if !found {
            return Err(AppError::not_found("Nothing to pin: it no longer exists."));
        }
        self.bump_version();
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Validate and classify requested folders. Duplicate paths collapse into one.
    async fn plan_members(
        &self,
        paths: &[String],
        scope: Option<&DiscoveryScope>,
    ) -> AppResult<Vec<PlannedMember>> {
        let git = self.git()?;
        let mut seen = HashSet::new();
        let mut planned = Vec::new();
        for raw in paths {
            let path = existing_folder(raw).await?;
            if !seen.insert(path.clone()) {
                continue;
            }
            let repository = resolve_optional(git, &path)
                .await?
                .filter(|r| r.root == path);
            planned.push(PlannedMember {
                display_name: folder_name(&path),
                origin: scope.map_or(MemberOrigin::Manual, |s| s.origin_of(&path)),
                canonical_path: path,
                repository,
            });
        }
        Ok(planned)
    }

    /// Bump the snapshot version, refresh newly registered repositories in the
    /// background, and return the stored workspace.
    async fn finish_change(
        self: &Arc<Self>,
        workspace_id: &str,
        new_repositories: Vec<RepositoryRow>,
    ) -> AppResult<WorkspaceChange> {
        self.bump_version();
        let workspace = self.workspace(workspace_id).await?;
        // Every member, not only new registrations: a repository that joins
        // this workspace may now be watched for more refs, and the refresh
        // takes that baseline before any news arrives.
        let mut ids: Vec<&String> = workspace
            .members
            .iter()
            .filter_map(|m| m.repository_id.as_ref())
            .collect();
        ids.sort();
        ids.dedup();
        for id in ids {
            let origin = if new_repositories.iter().any(|r| &r.id == id) {
                ChangeOrigin::Registration
            } else {
                ChangeOrigin::Refresh
            };
            self.request_refresh(id, origin);
        }
        Ok(WorkspaceChange {
            workspace,
            new_repositories,
        })
    }
}

/// Register (or reuse) each planned repository and insert the member rows.
/// Returns the newly registered repositories and, when one of the members is
/// the repository at `discovery_root`, its ID.
fn insert_members(
    conn: &rusqlite::Connection,
    workspace_id: &str,
    planned: Vec<PlannedMember>,
    discovery_root: Option<&Path>,
) -> AppResult<(Vec<RepositoryRow>, Option<String>)> {
    let now = now_rfc3339();
    let mut new_rows = Vec::new();
    let mut root_id = None;
    for member in planned {
        let repository_id = match &member.repository {
            Some(r) => {
                let row = RepositoryRow {
                    id: uuid::Uuid::new_v4().to_string(),
                    canonical_root: r.root.display().to_string(),
                    display_path: r.root.display().to_string(),
                    git_dir: r.git_dir.display().to_string(),
                    common_git_dir: r.common_git_dir.display().to_string(),
                    created_at: now.clone(),
                    // Joining a workspace is not opening: keep the recent list honest.
                    last_opened_at: None,
                    last_checked_at: None,
                    last_tab: None,
                    status: None,
                    error: None,
                    last_fetch_at: None,
                    last_fetch_error: None,
                };
                let (id, created) = db::ensure_repository(conn, &row)?;
                if created {
                    new_rows.push(row);
                }
                Some(id)
            }
            None => None,
        };
        if repository_id.is_some() && discovery_root == Some(member.canonical_path.as_path()) {
            root_id = repository_id.clone();
        }
        db::insert_member(
            conn,
            &MemberRow {
                id: uuid::Uuid::new_v4().to_string(),
                workspace_id: workspace_id.to_string(),
                display_name: member.display_name,
                canonical_path: member.canonical_path.display().to_string(),
                origin: member.origin,
                repository_id,
            },
        )?;
    }
    Ok((new_rows, root_id))
}

/// Build the DTO: root member first, then the rest by display name.
/// What per-workspace unread counts are computed from.
struct Unread {
    /// Repository ID to shared Git directory.
    stores: HashMap<String, String>,
    events: Vec<db::EventRow>,
}

fn to_workspace(
    row: WorkspaceRow,
    members: Vec<MemberRow>,
    activity: ActivitySettings,
    unread: &Unread,
) -> Workspace {
    let root_id = row.root_repository_id.clone();
    // Events belong to Git directories, which several members can share.
    let stores: HashSet<String> = members
        .iter()
        .filter_map(|m| m.repository_id.as_ref())
        .filter_map(|id| unread.stores.get(id).cloned())
        .collect();
    let unseen_activity = crate::activity::count_unseen(&activity, &stores, &unread.events);
    let mut members: Vec<WorkspaceMember> = members
        .into_iter()
        .map(|m| {
            let status = if !Path::new(&m.canonical_path).is_dir() {
                MemberStatus::Missing
            } else if m.repository_id.is_none() {
                MemberStatus::NotGit
            } else {
                MemberStatus::Ok
            };
            WorkspaceMember {
                origin: m.origin,
                display_name: m.display_name,
                canonical_path: m.canonical_path,
                repository_id: m.repository_id,
                status,
            }
        })
        .collect();
    // `sort_by_key` with a tuple sorts by the first element, then the next:
    // `false` (the root) before `true`, then names ignoring case.
    members.sort_by_key(|m| {
        (
            root_id.is_none() || m.repository_id != root_id,
            m.display_name.to_lowercase(),
            m.canonical_path.clone(),
        )
    });
    Workspace {
        id: row.id,
        name: row.name,
        discovery_mode: row.discovery_mode,
        root_repository_id: row.root_repository_id,
        discovery_root: row.discovery_root,
        discovery_path: row.discovery_path,
        members,
        activity,
        unseen_activity,
    }
}

fn preview(root: &Path, entries: Vec<WorkspacePreviewEntry>) -> WorkspacePreview {
    WorkspacePreview {
        name: folder_name(root),
        discovery_mode: DiscoveryMode::Discovered,
        entries,
        candidates: None,
    }
}

/// Resolve the repository containing `path`, or `None` when it is not in one.
/// Other failures (timeout, permissions, Git missing) are real errors.
async fn resolve_optional(
    git: &crate::git::GitService,
    path: &Path,
) -> AppResult<Option<ResolvedRepository>> {
    match git.resolve(path).await {
        Ok(r) => Ok(Some(r)),
        Err(e) if e.code == ErrorCode::Validation => Ok(None),
        Err(e) => Err(e),
    }
}

/// Canonicalize an absolute folder path: `NOT_FOUND` if it is missing,
/// `VALIDATION` if it is relative or not a folder.
async fn existing_folder(raw: &str) -> AppResult<PathBuf> {
    let path = Path::new(raw);
    if raw.is_empty() || !path.is_absolute() {
        return Err(AppError::validation(format!(
            "{raw:?} is not an absolute folder path."
        )));
    }
    let canonical = tokio::fs::canonicalize(path).await.map_err(|e| {
        AppError::not_found(format!(
            "The folder {raw} does not exist or cannot be read."
        ))
        .with_details(e.to_string())
    })?;
    if !canonical.is_dir() {
        return Err(AppError::validation(format!("{raw} is not a folder.")));
    }
    Ok(canonical)
}

/// Normalize a discovery folder relative to the selected folder: forward
/// slashes, no `.` parts, `None` for the folder itself. Absolute paths and
/// `..` are rejected so discovery cannot leave the selected folder.
fn normalize_discovery_path(raw: Option<&str>) -> AppResult<Option<String>> {
    // `let ... else` binds the value or runs the `else` branch, which must return.
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let mut parts = Vec::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => {
                return Err(AppError::validation(
                    "The discovery folder must be a subfolder of the selected folder.",
                ))
            }
        }
    }
    Ok((!parts.is_empty()).then(|| parts.join("/")))
}

/// The canonical form of `path` when it exists, so it compares equal to the
/// canonical member paths; otherwise `path` unchanged.
async fn canonical_or_same(path: PathBuf) -> PathBuf {
    tokio::fs::canonicalize(&path).await.unwrap_or(path)
}

fn valid_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::validation("A workspace needs a name."));
    }
    Ok(name.to_string())
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn workspace_not_found() -> AppError {
    AppError::not_found("That workspace no longer exists.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_paths_stay_inside_the_folder() {
        assert_eq!(normalize_discovery_path(None).unwrap(), None);
        assert_eq!(normalize_discovery_path(Some("  ")).unwrap(), None);
        assert_eq!(normalize_discovery_path(Some(".")).unwrap(), None);
        assert_eq!(
            normalize_discovery_path(Some("./services/")).unwrap(),
            Some("services".into())
        );
        assert_eq!(
            normalize_discovery_path(Some("a/b")).unwrap(),
            Some("a/b".into())
        );
        for bad in ["..", "a/../..", "/abs", "a/../b"] {
            let err = normalize_discovery_path(Some(bad)).unwrap_err();
            assert_eq!(err.code, ErrorCode::Validation, "{bad}");
        }
    }
}
