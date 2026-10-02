//! Thin Tauri command handlers. Each one validates nothing beyond types and
//! delegates to a service (`RepositoryService`, `NoteService`,
//! `TaskService`); business rules live there.

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
use crate::models::{
    CreateNoteRequest, ExportResult, FolderListing, NoteContent, NoteContext, NoteLists,
    NoteRevision, NoteSummary, RenameNoteRequest, RenamePreview, RenameResult, RepositoryNotes,
    RestorePreview, RestoreRequest, RestoreResult, SaveNoteRequest, SaveNoteResult, SearchRequest,
    SearchResults, Task, TaskFields, TaskFilter, TodayView, TrashedNote, UpdateTaskRequest,
    VaultState,
};
use crate::notes::NoteService;
use crate::tasks::TaskService;
use crate::watcher::RepositoryWatcher;
use crate::workspaces::{RepositoryService, WorkspaceChange};

pub type Service = Arc<RepositoryService>;
pub type Agent = Arc<crate::mcp::AgentServer>;

/// Settings → Agent access: the mode, connected agents, and the command to add Brainiac.
#[tauri::command]
pub async fn get_agent_access_status(
    agent: State<'_, Agent>,
) -> AppResult<crate::models::AgentAccessStatus> {
    Ok(agent.status())
}

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

#[tauri::command]
pub async fn update_settings(
    settings: crate::models::Settings,
    service: State<'_, Service>,
    notes: State<'_, Notes>,
    agent: State<'_, Agent>,
) -> AppResult<crate::models::Settings> {
    let saved = service.update_settings(settings).await?;
    notes.set_write_note_ids(saved.write_note_ids);
    agent.set_access(saved.agent_access);
    Ok(saved)
}

// ---------------------------------------------------------------------------
// v0.2: vault, notes, tasks, search, backups
// ---------------------------------------------------------------------------

pub type Notes = Arc<NoteService>;
pub type Tasks = Arc<TaskService>;

#[tauri::command]
pub async fn get_vault_state(notes: State<'_, Notes>) -> AppResult<VaultState> {
    Ok(notes.vault_state().await)
}

/// Use the folder at `path` as the vault; with `create`, make a new, empty one there.
#[tauri::command]
pub async fn select_vault(
    path: String,
    create: bool,
    notes: State<'_, Notes>,
) -> AppResult<VaultState> {
    notes.select_vault(&path, create).await
}

#[tauri::command]
pub async fn list_folder(
    folder: Option<String>,
    notes: State<'_, Notes>,
) -> AppResult<FolderListing> {
    notes.list_folder(folder).await
}

#[tauri::command]
pub async fn get_note_lists(notes: State<'_, Notes>) -> AppResult<NoteLists> {
    notes.lists().await
}

#[tauri::command]
pub async fn read_note(note_id: String, notes: State<'_, Notes>) -> AppResult<NoteContent> {
    notes.read(&note_id).await
}

#[tauri::command]
pub async fn mark_note_opened(note_id: String, notes: State<'_, Notes>) -> AppResult<()> {
    notes.mark_opened(&note_id).await
}

#[tauri::command]
pub async fn save_note(
    request: SaveNoteRequest,
    notes: State<'_, Notes>,
) -> AppResult<SaveNoteResult> {
    notes.save(request).await
}

#[tauri::command]
pub async fn create_note(
    request: CreateNoteRequest,
    notes: State<'_, Notes>,
) -> AppResult<NoteSummary> {
    notes.create(request).await
}

#[tauri::command]
pub async fn preview_rename(
    note_id: String,
    new_path: String,
    notes: State<'_, Notes>,
) -> AppResult<RenamePreview> {
    notes.preview_rename(&note_id, &new_path).await
}

#[tauri::command]
pub async fn rename_note(
    request: RenameNoteRequest,
    notes: State<'_, Notes>,
) -> AppResult<RenameResult> {
    notes.rename(request).await
}

#[tauri::command]
pub async fn follow_note_title(
    note_id: String,
    from_title: String,
    notes: State<'_, Notes>,
) -> AppResult<NoteSummary> {
    notes.follow_title(&note_id, &from_title).await
}

#[tauri::command]
pub async fn trash_note(note_id: String, notes: State<'_, Notes>) -> AppResult<()> {
    notes.trash(&note_id).await
}

#[tauri::command]
pub async fn list_trash(notes: State<'_, Notes>) -> AppResult<Vec<TrashedNote>> {
    notes.list_trash().await
}

#[tauri::command]
pub async fn restore_note(
    note_id: String,
    overwrite: bool,
    notes: State<'_, Notes>,
) -> AppResult<NoteSummary> {
    notes.restore(&note_id, overwrite).await
}

#[tauri::command]
pub async fn recreate_note(
    note_id: String,
    text: String,
    notes: State<'_, Notes>,
) -> AppResult<NoteSummary> {
    notes.recreate(&note_id, text).await
}

#[tauri::command]
pub async fn relink_note(
    note_id: String,
    path: String,
    notes: State<'_, Notes>,
) -> AppResult<NoteSummary> {
    notes.relink(&note_id, &path).await
}

#[tauri::command]
pub async fn list_revisions(
    note_id: String,
    notes: State<'_, Notes>,
) -> AppResult<Vec<NoteRevision>> {
    notes.revisions(&note_id).await
}

#[tauri::command]
pub async fn read_revision(revision_id: String, notes: State<'_, Notes>) -> AppResult<String> {
    notes.revision_text(&revision_id).await
}

#[tauri::command]
pub async fn restore_revision(
    note_id: String,
    revision_id: String,
    expected_version: String,
    notes: State<'_, Notes>,
) -> AppResult<SaveNoteResult> {
    notes
        .restore_revision(&note_id, &revision_id, &expected_version)
        .await
}

/// Keep unsaved edits as a draft while the note cannot be saved (changed on disk, or gone).
#[tauri::command]
pub async fn save_draft(
    note_id: String,
    base_version: String,
    text: String,
    notes: State<'_, Notes>,
) -> AppResult<()> {
    notes.keep_draft(&note_id, &base_version, text).await
}

#[tauri::command]
pub async fn discard_draft(note_id: String, notes: State<'_, Notes>) -> AppResult<()> {
    notes.discard_draft(&note_id).await
}

#[tauri::command]
pub async fn save_draft_as_copy(
    note_id: String,
    text: String,
    notes: State<'_, Notes>,
) -> AppResult<NoteSummary> {
    notes.save_copy(&note_id, text).await
}

/// Open a vault file in its default application (files Brainiac does not edit).
#[tauri::command]
pub async fn open_vault_file(relative_path: String, notes: State<'_, Notes>) -> AppResult<()> {
    let path = notes.absolute(&relative_path)?;
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| {
        crate::models::AppError::io("Could not open the file.").with_details(e.to_string())
    })
}

/// Reveal a vault file or folder in Finder; the vault itself without a path.
#[tauri::command]
pub async fn reveal_vault_path(
    relative_path: Option<String>,
    notes: State<'_, Notes>,
) -> AppResult<()> {
    let target = match relative_path.filter(|p| !p.is_empty()) {
        Some(p) => notes.absolute(&p)?,
        None => notes
            .vault()
            .map(|v| v.root)
            .ok_or_else(|| crate::models::AppError::not_found("Choose a vault folder first."))?,
    };
    tauri_plugin_opener::reveal_item_in_dir(&target).map_err(|e| {
        crate::models::AppError::io("Could not reveal the item in Finder.")
            .with_details(e.to_string())
    })
}

#[tauri::command]
pub async fn get_note_context(note_id: String, notes: State<'_, Notes>) -> AppResult<NoteContext> {
    notes.context(&note_id).await
}

#[tauri::command]
pub async fn resolve_link(
    note_id: String,
    target: String,
    wikilink: bool,
    notes: State<'_, Notes>,
) -> AppResult<crate::models::ResolvedLink> {
    notes.resolve_link(&note_id, &target, wikilink).await
}

#[tauri::command]
pub async fn get_repository_notes(
    repository_id: String,
    notes: State<'_, Notes>,
) -> AppResult<RepositoryNotes> {
    notes.repository_notes(&repository_id).await
}

#[tauri::command]
pub async fn link_repository(
    note_id: String,
    repository_id: String,
    notes: State<'_, Notes>,
) -> AppResult<()> {
    notes.link_repository(&note_id, &repository_id).await
}

#[tauri::command]
pub async fn unlink_repository(
    note_id: String,
    repository_id: String,
    notes: State<'_, Notes>,
) -> AppResult<()> {
    notes.unlink_repository(&note_id, &repository_id).await
}

#[tauri::command]
pub async fn dismiss_suggestion(
    note_id: String,
    repository_id: String,
    notes: State<'_, Notes>,
) -> AppResult<()> {
    notes.dismiss_suggestion(&note_id, &repository_id).await
}

/// Move the links of a removed repository to a registered one with the same remote.
#[tauri::command]
pub async fn reconnect_repository(
    from: String,
    to: String,
    notes: State<'_, Notes>,
) -> AppResult<()> {
    notes.reconnect_repository(&from, &to).await
}

#[tauri::command]
pub async fn list_tasks(
    filter: Option<TaskFilter>,
    tasks: State<'_, Tasks>,
) -> AppResult<Vec<Task>> {
    tasks.list(filter.unwrap_or_default()).await
}

#[tauri::command]
pub async fn get_task(task_id: String, tasks: State<'_, Tasks>) -> AppResult<Task> {
    tasks.get(&task_id).await
}

#[tauri::command]
pub async fn get_today(tasks: State<'_, Tasks>) -> AppResult<TodayView> {
    tasks.today().await
}

#[tauri::command]
pub async fn create_task(fields: TaskFields, tasks: State<'_, Tasks>) -> AppResult<Task> {
    tasks.create(fields).await
}

#[tauri::command]
pub async fn update_task(request: UpdateTaskRequest, tasks: State<'_, Tasks>) -> AppResult<Task> {
    tasks.update(request).await
}

#[tauri::command]
pub async fn delete_task(
    task_id: String,
    expected_version: i64,
    tasks: State<'_, Tasks>,
) -> AppResult<()> {
    tasks.delete(&task_id, expected_version).await
}

#[tauri::command]
pub async fn search(request: SearchRequest, notes: State<'_, Notes>) -> AppResult<SearchResults> {
    crate::index::search(&notes, request).await
}

#[tauri::command]
pub async fn rebuild_search(notes: State<'_, Notes>) -> AppResult<()> {
    notes.rebuild_search().await
}

/// Write an export into a new folder inside `folder`.
#[tauri::command]
pub async fn export_backup(folder: String, notes: State<'_, Notes>) -> AppResult<ExportResult> {
    crate::backup::export(&notes, std::path::Path::new(&folder)).await
}

#[tauri::command]
pub async fn preview_restore(path: String) -> AppResult<RestorePreview> {
    tokio::task::spawn_blocking(move || crate::backup::preview(std::path::Path::new(&path)))
        .await
        .map_err(|e| {
            crate::models::AppError::io("Reading the export stopped.").with_details(e.to_string())
        })?
}

/// Stage a restore; the frontend relaunches Brainiac to apply it.
#[tauri::command]
pub async fn restore_backup(
    request: RestoreRequest,
    notes: State<'_, Notes>,
) -> AppResult<RestoreResult> {
    crate::backup::restore(&notes, request).await
}
