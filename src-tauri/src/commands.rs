//! Thin Tauri command handlers. Each one validates nothing beyond types and
//! delegates to `RepositoryService`; business rules live there.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;

use crate::db::RepositoryRow;
use crate::models::{
    ActivitySettings, AppResult, AppSnapshot, ChangesResult, CommitDetail, CommitPage,
    CreateWorkspaceRequest, DiffOptions, DiffResult, DiffSelector, FetchResult, ListCommitsRequest,
    PinEntityType, RefsResult, RelocateRepositoryRequest, RelocationOutcome, RepositorySummary,
    RepositoryTab, TeamPulse, UpdateWorkspaceMembershipRequest, Workspace, WorkspaceActivity,
    WorkspacePreview, WorkspaceRescan,
};
use crate::watcher::RepositoryWatcher;
use crate::workspaces::{RepositoryService, WorkspaceChange};

pub type Service = Arc<RepositoryService>;

#[tauri::command]
pub async fn get_app_snapshot(service: State<'_, Service>) -> AppResult<AppSnapshot> {
    service.snapshot().await
}

#[tauri::command]
pub async fn register_repository(
    path: String,
    service: State<'_, Service>,
    watcher: State<'_, RepositoryWatcher>,
) -> AppResult<RepositorySummary> {
    let summary = service.register(&PathBuf::from(path)).await?;
    let row = service.row(&summary.id).await?;
    watch(&watcher, &row);
    Ok(summary)
}

/// Start watching a newly registered repository. A watcher failure is logged,
/// not returned: the repository still works with timer and manual refreshes.
fn watch(watcher: &RepositoryWatcher, row: &RepositoryRow) {
    if let Err(e) = watcher.watch(
        &row.id,
        &PathBuf::from(&row.canonical_root),
        &PathBuf::from(&row.git_dir),
    ) {
        tracing::warn!(repository = %row.display_path, error = %e, "could not watch repository");
    }
}

/// Watch the repositories a workspace change registered and return the workspace.
fn watch_new(watcher: &RepositoryWatcher, change: WorkspaceChange) -> Workspace {
    for row in &change.new_repositories {
        watch(watcher, row);
    }
    change.workspace
}

#[tauri::command]
pub async fn remove_repository(
    repository_id: String,
    service: State<'_, Service>,
    watcher: State<'_, RepositoryWatcher>,
) -> AppResult<()> {
    watcher.unwatch(&repository_id);
    service.remove(&repository_id).await
}

/// Point a registration at the folder its repository moved to, then watch
/// every registration that moved at its new folder.
#[tauri::command]
pub async fn relocate_repository(
    request: RelocateRepositoryRequest,
    service: State<'_, Service>,
    watcher: State<'_, RepositoryWatcher>,
) -> AppResult<RelocationOutcome> {
    let relocation = service.relocate_repository(request).await?;
    for row in &relocation.moved {
        watcher.unwatch(&row.id);
        watch(&watcher, row);
    }
    Ok(relocation.outcome)
}

#[tauri::command]
pub async fn refresh_repository(
    repository_id: String,
    service: State<'_, Service>,
) -> AppResult<RepositorySummary> {
    service
        .refresh(&repository_id, crate::models::ChangeOrigin::Refresh)
        .await
}

#[tauri::command]
pub async fn open_repository(repository_id: String, service: State<'_, Service>) -> AppResult<()> {
    service.mark_opened(&repository_id).await
}

#[tauri::command]
pub async fn set_repository_tab(
    repository_id: String,
    tab: RepositoryTab,
    service: State<'_, Service>,
) -> AppResult<()> {
    service.set_tab(&repository_id, tab).await
}

#[tauri::command]
pub async fn list_changes(
    repository_id: String,
    service: State<'_, Service>,
) -> AppResult<ChangesResult> {
    service.changes(&repository_id).await
}

#[tauri::command]
pub async fn get_diff(
    repository_id: String,
    selector: DiffSelector,
    options: Option<DiffOptions>,
    service: State<'_, Service>,
) -> AppResult<DiffResult> {
    service
        .diff(&repository_id, selector, options.unwrap_or_default())
        .await
}

#[tauri::command]
pub async fn list_commits(
    request: ListCommitsRequest,
    service: State<'_, Service>,
) -> AppResult<CommitPage> {
    service.commits(request).await
}

#[tauri::command]
pub async fn get_commit(
    repository_id: String,
    commit_id: String,
    parent_index: Option<u32>,
    service: State<'_, Service>,
) -> AppResult<CommitDetail> {
    service
        .commit(&repository_id, &commit_id, parent_index)
        .await
}

#[tauri::command]
pub async fn list_refs(
    repository_id: String,
    service: State<'_, Service>,
) -> AppResult<RefsResult> {
    service.refs(&repository_id).await
}

#[tauri::command]
pub async fn open_in_editor(
    repository_id: String,
    path: Option<String>,
    line: Option<u64>,
    service: State<'_, Service>,
) -> AppResult<()> {
    service
        .open_in_editor(&repository_id, path.as_deref(), line)
        .await
}

#[tauri::command]
pub async fn reveal_in_finder(
    repository_id: String,
    path: Option<String>,
    service: State<'_, Service>,
) -> AppResult<()> {
    let root = service.repository_root(&repository_id).await?;
    let target = match path {
        Some(p) => {
            crate::git::validate_repo_path(&p)?;
            root.join(p)
        }
        None => root,
    };
    tauri_plugin_opener::reveal_item_in_dir(&target).map_err(|e| {
        crate::models::AppError::io("Could not reveal the item in Finder.")
            .with_details(e.to_string())
    })
}

#[tauri::command]
pub async fn discover_repositories(
    folder_path: String,
    discovery_path: Option<String>,
    service: State<'_, Service>,
) -> AppResult<WorkspacePreview> {
    service
        .discover_repositories(&folder_path, discovery_path.as_deref())
        .await
}

#[tauri::command]
pub async fn rescan_workspace(
    workspace_id: String,
    service: State<'_, Service>,
) -> AppResult<WorkspaceRescan> {
    service.rescan_workspace(&workspace_id).await
}

#[tauri::command]
pub async fn create_workspace(
    request: CreateWorkspaceRequest,
    service: State<'_, Service>,
    watcher: State<'_, RepositoryWatcher>,
) -> AppResult<Workspace> {
    let change = service.create_workspace(request).await?;
    Ok(watch_new(&watcher, change))
}

#[tauri::command]
pub async fn update_workspace_membership(
    request: UpdateWorkspaceMembershipRequest,
    service: State<'_, Service>,
    watcher: State<'_, RepositoryWatcher>,
) -> AppResult<Workspace> {
    let change = service.update_workspace_membership(request).await?;
    Ok(watch_new(&watcher, change))
}

#[tauri::command]
pub async fn rename_workspace(
    workspace_id: String,
    name: String,
    service: State<'_, Service>,
) -> AppResult<Workspace> {
    service.rename_workspace(&workspace_id, &name).await
}

#[tauri::command]
pub async fn remove_workspace(workspace_id: String, service: State<'_, Service>) -> AppResult<()> {
    service.remove_workspace(&workspace_id).await
}

#[tauri::command]
pub async fn set_pinned(
    entity_type: PinEntityType,
    entity_id: String,
    pinned: bool,
    service: State<'_, Service>,
) -> AppResult<()> {
    service.set_pinned(entity_type, &entity_id, pinned).await
}

#[tauri::command]
pub async fn fetch_repository(
    repository_id: String,
    service: State<'_, Service>,
) -> AppResult<FetchResult> {
    service.fetch(&repository_id, false).await
}

#[tauri::command]
pub async fn get_workspace_activity(
    workspace_id: String,
    service: State<'_, Service>,
) -> AppResult<WorkspaceActivity> {
    service.workspace_activity(&workspace_id).await
}

#[tauri::command]
pub async fn mark_activity_seen(
    workspace_id: String,
    event_ids: Option<Vec<String>>,
    service: State<'_, Service>,
) -> AppResult<()> {
    service
        .mark_activity_seen(&workspace_id, event_ids.as_deref())
        .await
}

#[tauri::command]
pub async fn update_activity_settings(
    workspace_id: String,
    settings: ActivitySettings,
    service: State<'_, Service>,
) -> AppResult<Workspace> {
    service
        .update_activity_settings(&workspace_id, settings)
        .await
}

#[tauri::command]
pub async fn get_team_pulse(
    workspace_id: String,
    service: State<'_, Service>,
) -> AppResult<TeamPulse> {
    service.team_pulse(&workspace_id).await
}
