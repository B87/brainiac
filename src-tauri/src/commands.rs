//! Thin Tauri command handlers. Each one validates nothing beyond types and
//! delegates to `RepositoryService`; business rules live there.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;

use crate::models::{
    AppResult, AppSnapshot, ChangesResult, CommitDetail, CommitPage, DiffResult, DiffSelector,
    ListCommitsRequest, RefsResult, RepositorySummary, RepositoryTab,
};
use crate::watcher::RepositoryWatcher;
use crate::workspaces::RepositoryService;

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
    if let Err(e) = watcher.watch(
        &row.id,
        &PathBuf::from(&row.canonical_root),
        &PathBuf::from(&row.git_dir),
    ) {
        tracing::warn!(repository = %row.display_path, error = %e, "could not watch repository");
    }
    Ok(summary)
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
    service: State<'_, Service>,
) -> AppResult<DiffResult> {
    service.diff(&repository_id, selector).await
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
