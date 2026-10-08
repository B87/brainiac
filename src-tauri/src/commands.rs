//! Thin Tauri command handlers. Each one validates nothing beyond types and
//! delegates to a service (`RepositoryService`, `NoteService`,
//! `TaskService`, `AccountService`); business rules live there.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;

use crate::db::RepositoryRow;
use crate::forge::{AccountService, PullRequestService};
use crate::models::{
    ActivitySettings, AppResult, AppSnapshot, ChangesResult, CommitDetail, CommitPage,
    CreateWorkspaceRequest, DiffOptions, DiffResult, DiffSelector, FetchResult, ListCommitsRequest,
    PinEntityType, RefsResult, RelocateRepositoryRequest, RelocationOutcome, RepositorySummary,
    RepositoryTab, TeamPulse, UpdateWorkspaceMembershipRequest, Workspace, WorkspaceActivity,
    WorkspacePreview, WorkspaceRescan,
};
use crate::models::{
    CommentRequest, Conversation, ListPullRequestsRequest, MergeOptions, MergeOutcome,
    MergeRequest, PullRequest, PullRequestChecks, PullRequestDiff, PullRequestDiffRequest,
    PullRequestFiles, PullRequestList, ReplyRequest, ResolveThreadRequest, ReviewCount,
    ReviewDrafts, SaveReviewDraftRequest, SubmitReviewRequest, WriteOutcome,
};
use crate::models::{
    CreateNoteRequest, ExportResult, FolderListing, NoteContent, NoteContext, NoteLists,
    NoteRevision, NoteSummary, RenameNoteRequest, RenamePreview, RenameResult, RepositoryNotes,
    RestorePreview, RestoreRequest, RestoreResult, SaveNoteRequest, SaveNoteResult, SearchRequest,
    SearchResults, Task, TaskFields, TaskFilter, TodayView, TrashedNote, UpdateTaskRequest,
    VaultState,
};
use crate::models::{
    ForgeAccountSlot, ForgeKind, SaveForgeAccountOutcome, SaveForgeAccountRequest,
    SetRepositoryForgeRequest,
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
    databases: State<'_, Databases>,
) -> AppResult<crate::models::Settings> {
    let saved = service.update_settings(settings).await?;
    notes.set_write_note_ids(saved.write_note_ids);
    agent.set_access(saved.agent_access);
    databases.set_history(saved.query_history);
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

// ---------------------------------------------------------------------------
// v0.3: pull requests
// ---------------------------------------------------------------------------

pub type Accounts = Arc<AccountService>;

/// Settings → Accounts: one entry per provider.
#[tauri::command]
pub async fn list_forge_accounts(
    accounts: State<'_, Accounts>,
) -> AppResult<Vec<ForgeAccountSlot>> {
    accounts.list().await
}

#[tauri::command]
pub async fn save_forge_account(
    request: SaveForgeAccountRequest,
    accounts: State<'_, Accounts>,
) -> AppResult<SaveForgeAccountOutcome> {
    accounts.save(request).await
}

#[tauri::command]
pub async fn remove_forge_account(
    kind: ForgeKind,
    accounts: State<'_, Accounts>,
) -> AppResult<Vec<ForgeAccountSlot>> {
    accounts.remove(kind).await
}

/// Test on an account's form: check the form's token without saving it.
#[tauri::command]
pub async fn test_forge_account(
    request: SaveForgeAccountRequest,
    accounts: State<'_, Accounts>,
) -> AppResult<crate::models::ForgeAccountTestResult> {
    accounts.test(request).await
}

// v0.4.x: where secrets come from (SPEC.md, Secrets).

pub type Secrets = Arc<crate::secrets::SecretsService>;

/// Settings → Secrets. Reads no secret and runs no program.
#[tauri::command]
pub async fn list_secrets(
    secrets: State<'_, Secrets>,
) -> AppResult<crate::models::SecretsOverview> {
    secrets.overview().await
}

/// **Allow This Source**: confirm a restored source at the revision shown.
#[tauri::command]
pub async fn approve_secret_source(
    owner: crate::models::CredentialOwner,
    revision: i64,
    secrets: State<'_, Secrets>,
) -> AppResult<crate::models::SecretsOverview> {
    secrets.approve(&owner, revision).await?;
    secrets.overview().await
}

/// **Refresh**: forget the secret kept for this run, so it is read or asked for again.
#[tauri::command]
pub async fn refresh_credential(
    owner: crate::models::CredentialOwner,
    secrets: State<'_, Secrets>,
) -> AppResult<()> {
    secrets.refresh(&owner);
    Ok(())
}

/// **Retry** a cleanup or removal of a Keychain item that did not finish.
#[tauri::command]
pub async fn retry_credential_cleanup(
    owner: crate::models::CredentialOwner,
    secrets: State<'_, Secrets>,
) -> AppResult<crate::models::SecretsOverview> {
    secrets.retry(&owner).await?;
    secrets.overview().await
}

/// **Find…** on a command source: the full path of a program, by name.
#[tauri::command]
pub async fn find_secret_program(name: String) -> AppResult<String> {
    crate::credentials::find_program(&name).map(|p| p.display().to_string())
}

// v0.5: agent runs (SPEC.md, section 13).

pub type AgentSettingsState = Arc<crate::agents::AgentSettingsService>;
pub type Artifacts = Arc<crate::agents::RunArtifacts>;

/// Settings → Agents.
#[tauri::command]
pub async fn get_agent_settings(
    agents: State<'_, AgentSettingsState>,
) -> AppResult<crate::models::AgentSettings> {
    agents.get().await
}

/// Settings → Agents, one profile: everything but the token or key.
#[tauri::command]
pub async fn save_agent_settings(
    request: crate::models::SaveAgentSettingsRequest,
    agents: State<'_, AgentSettingsState>,
) -> AppResult<crate::models::AgentSettings> {
    agents.save(request).await
}

/// Settings → Agents, **Pay with**: the token or key and where it comes from.
#[tauri::command]
pub async fn save_agent_credential(
    request: crate::models::SaveAgentCredentialRequest,
    agents: State<'_, AgentSettingsState>,
) -> AppResult<crate::models::AgentSettings> {
    agents.save_credential(request).await
}

/// Settings → Agents, **Remove** a profile's token or key.
#[tauri::command]
pub async fn remove_agent_credential(
    id: String,
    expected_version: i64,
    agents: State<'_, AgentSettingsState>,
) -> AppResult<crate::models::AgentSettings> {
    agents.remove_credential(&id, expected_version).await
}

/// Settings → Agents, This Mac, **Engine**: the Docker engine runs use here.
#[tauri::command]
pub async fn choose_agent_engine(
    socket: Option<String>,
    agents: State<'_, AgentSettingsState>,
) -> AppResult<crate::models::AgentSettings> {
    agents.choose_engine(socket).await
}

/// Settings → Agents, **Confirm** a setup restored from a backup.
#[tauri::command]
pub async fn approve_agent_settings(
    id: String,
    revision: i64,
    agents: State<'_, AgentSettingsState>,
) -> AppResult<crate::models::AgentSettings> {
    agents.approve(&id, revision).await
}

/// Settings → Agents, **Where runs execute**: this Mac's Docker engines.
#[tauri::command]
pub async fn list_agent_engines(
    agents: State<'_, AgentSettingsState>,
) -> AppResult<Vec<crate::models::AgentEngine>> {
    let chosen = agents.get().await?.engine_socket;
    Ok(crate::agents::engine::list(chosen.as_deref()).await)
}

/// Settings → Agents, **View Dockerfile**.
#[tauri::command]
pub async fn agent_dockerfile() -> AppResult<String> {
    Ok(crate::agents::image::dockerfile().to_string())
}

/// Settings → Agents, **Build image** or **Rebuild…**.
#[tauri::command]
pub async fn build_agent_image(
    agents: State<'_, AgentSettingsState>,
) -> AppResult<crate::models::AgentSettings> {
    agents.build_image().await
}

/// New run, **Start from**: the commit a run would get, read without changing the repository.
#[tauri::command]
pub async fn preview_run_start(
    repository_id: String,
    start: String,
    service: State<'_, Service>,
    artifacts: State<'_, Artifacts>,
) -> AppResult<crate::models::RunStartPreview> {
    let root = service.repository_root(&repository_id).await?;
    artifacts.preview(&repository_id, &root, &start).await
}

pub type Runs = Arc<crate::agents::AgentRunService>;
pub type AgentHosts = Arc<crate::agents::hosts::AgentHostService>;

#[tauri::command]
pub async fn preview_agent_host(
    host: String,
    port: u16,
    hosts: State<'_, AgentHosts>,
) -> AppResult<crate::models::AgentHostPreview> {
    hosts.preview(&host, port).await
}

#[tauri::command]
pub async fn approve_agent_host(
    request: crate::models::ApproveAgentHostRequest,
    hosts: State<'_, AgentHosts>,
) -> AppResult<crate::models::AgentHost> {
    hosts.approve(request).await
}

pub type HostJobs = Arc<crate::agents::host_jobs::HostJobService>;

/// Install, Upgrade, Build image, Test, or Add host's setup on a remote
/// host: starts the job and returns it at once (SPEC.md, Host jobs).
#[tauri::command]
pub async fn start_agent_host_job(
    id: String,
    kind: crate::models::HostJobKind,
    profile_id: Option<String>,
    jobs: State<'_, HostJobs>,
) -> AppResult<crate::models::HostJob> {
    jobs.start(&id, kind, profile_id).await
}

#[tauri::command]
pub async fn cancel_agent_host_job(
    id: String,
    jobs: State<'_, HostJobs>,
) -> AppResult<crate::models::HostJob> {
    jobs.cancel(&id)
}

/// Each host's last job, running or ended.
#[tauri::command]
pub async fn list_agent_host_jobs(
    jobs: State<'_, HostJobs>,
) -> AppResult<Vec<crate::models::HostJob>> {
    Ok(jobs.list())
}

/// The whole output of a host's last job, for Show the whole log and Copy.
#[tauri::command]
pub async fn get_agent_host_job_log(id: String, jobs: State<'_, HostJobs>) -> AppResult<String> {
    jobs.log(&id)
}

#[tauri::command]
pub async fn remove_agent_host(id: String, jobs: State<'_, HostJobs>) -> AppResult<()> {
    jobs.remove(&id).await
}

/// Runs: every run, newest first, and whether the controller is running.
#[tauri::command]
pub async fn list_agent_runs(runs: State<'_, Runs>) -> AppResult<crate::models::AgentRunList> {
    runs.list().await
}

#[tauri::command]
pub async fn get_agent_run(
    id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentRun> {
    runs.get(&id).await
}

/// New run, **Start run**.
#[tauri::command]
pub async fn start_agent_run(
    request: crate::models::StartRunRequest,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentRun> {
    runs.start(request).await
}

/// An agent's reply, drawn as Markdown. The journal keeps the text.
#[tauri::command]
pub fn render_markdown(text: String) -> String {
    crate::forge::markdown::render(&text)
}

/// The conversation after a sequence, from the mirrored journal.
#[tauri::command]
pub async fn list_run_events(
    id: String,
    after: u64,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::RunEventPage> {
    runs.events(&id, after).await
}

/// **Next prompt**, **Send**.
#[tauri::command]
pub async fn send_run_prompt(
    id: String,
    text: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentRun> {
    runs.prompt(&id, &text).await
}

/// **Allow once** or **Reject** a permission request.
#[tauri::command]
pub async fn answer_run_permission(
    id: String,
    permission_id: String,
    allow: bool,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentRun> {
    runs.permit(&id, &permission_id, allow).await
}

/// **Cancel run…**
#[tauri::command]
pub async fn cancel_agent_run(
    id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentRun> {
    runs.cancel(&id).await
}

/// **Finish and collect**
#[tauri::command]
pub async fn finish_agent_run(
    id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentRun> {
    runs.finish(&id).await
}

/// **Collect work**, **Retry collection**, or **Choose files to add…** with `include`.
#[tauri::command]
pub async fn collect_agent_run(
    id: String,
    include: Vec<String>,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentRun> {
    runs.collect(&id, include).await
}

/// **Keep this snapshot**
#[tauri::command]
pub async fn accept_run_snapshot(
    id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentRun> {
    runs.accept_snapshot(&id).await
}

/// **Discard work…**
#[tauri::command]
pub async fn discard_agent_run(
    id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentRun> {
    runs.discard(&id).await
}

/// **Retry cleanup**
#[tauri::command]
pub async fn retry_run_cleanup(
    id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentRun> {
    runs.retry_cleanup(&id).await
}

/// **Delete run…**
#[tauri::command]
pub async fn delete_agent_run(id: String, runs: State<'_, Runs>) -> AppResult<()> {
    runs.delete(&id).await
}

/// **Changes**: the collected snapshot's files.
#[tauri::command]
pub async fn get_run_changes(
    id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::RunChanges> {
    runs.changes(&id).await
}

#[tauri::command]
pub async fn get_run_diff(
    request: crate::models::RunDiffRequest,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::DiffResult> {
    runs.diff(request).await
}

/// **Changes so far**: the live run's last preview.
#[tauri::command]
pub async fn get_run_preview(
    id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::RunPreview> {
    runs.preview_state(&id).await
}

/// **Refresh** in Changes so far: a new preview, now.
#[tauri::command]
pub async fn refresh_run_preview(
    id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::RunPreview> {
    runs.preview(&id).await
}

/// **Copy branch command**: a command for the user to run; Brainiac does not.
#[tauri::command]
pub async fn get_run_branch_command(
    id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::RunBranchCommand> {
    runs.branch_command(&id).await
}

/// **Copy patch**: the whole snapshot as a patch.
#[tauri::command]
pub async fn copy_run_patch(id: String, runs: State<'_, Runs>) -> AppResult<String> {
    runs.patch(&id, None).await
}

/// **Save patch…**: the patch to the chosen file.
#[tauri::command]
pub async fn save_run_patch(id: String, path: String, runs: State<'_, Runs>) -> AppResult<()> {
    runs.patch(&id, Some(std::path::Path::new(&path))).await?;
    Ok(())
}

/// Settings → Agents, This Mac, **Test** a profile.
#[tauri::command]
pub async fn test_agent_setup(
    profile_id: String,
    runs: State<'_, Runs>,
) -> AppResult<crate::models::AgentTestResult> {
    runs.test(&profile_id).await
}

/// Settings → Agents: whether the run controller is running.
#[tauri::command]
pub async fn get_run_controller_status(
    runs: State<'_, Runs>,
) -> AppResult<crate::models::RunControllerStatus> {
    Ok(runs.controller_status().await)
}

/// Repository → Pull requests, **Change…**: where a repository's pull requests come from.
#[tauri::command]
pub async fn set_repository_forge(
    request: SetRepositoryForgeRequest,
    service: State<'_, Service>,
) -> AppResult<RepositorySummary> {
    service.set_forge(request).await
}

/// A workspace's pull request switch, off by default.
#[tauri::command]
pub async fn update_workspace_pull_requests(
    workspace_id: String,
    enabled: bool,
    service: State<'_, Service>,
) -> AppResult<Workspace> {
    service.set_pull_requests(&workspace_id, enabled).await
}

pub type PullRequests = Arc<PullRequestService>;

/// The workspace's or a repository's Pull requests tab.
#[tauri::command]
pub async fn list_pull_requests(
    request: ListPullRequestsRequest,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<PullRequestList> {
    pull_requests.list(request).await
}

/// One pull request, cached when younger than `max_age_seconds`.
#[tauri::command]
pub async fn get_pull_request(
    reference: String,
    max_age_seconds: u64,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<PullRequest> {
    pull_requests.get(&reference, max_age_seconds).await
}

/// The files a pull request changes; `since` a commit of it, only those changed after it.
#[tauri::command]
pub async fn list_pull_request_files(
    reference: String,
    since: Option<String>,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<PullRequestFiles> {
    pull_requests.files(&reference, since.as_deref()).await
}

/// The local checkout of a pull request's repository, for the side panel.
#[tauri::command]
pub async fn get_pull_request_repository(
    reference: String,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<RepositorySummary> {
    pull_requests.repository(&reference).await
}

// Reviewing (SPEC.md, Reviewing): drafts stay on the Mac; every other write
// goes to the provider on this explicit action.

#[tauri::command]
pub async fn list_review_drafts(
    reference: String,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<ReviewDrafts> {
    pull_requests.drafts(&reference).await
}

#[tauri::command]
pub async fn save_review_draft(
    request: SaveReviewDraftRequest,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<ReviewDrafts> {
    pull_requests.save_draft(request).await
}

#[tauri::command]
pub async fn delete_review_draft(
    reference: String,
    id: String,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<ReviewDrafts> {
    pull_requests.delete_draft(&reference, &id).await
}

/// The drafts go on the head now: the user looked at the new commits.
#[tauri::command]
pub async fn move_review_drafts(
    reference: String,
    head_sha: String,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<ReviewDrafts> {
    pull_requests.move_drafts(&reference, &head_sha).await
}

#[tauri::command]
pub async fn comment_on_pull_request(
    request: CommentRequest,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<WriteOutcome> {
    pull_requests.comment(request).await
}

#[tauri::command]
pub async fn reply_to_thread(
    request: ReplyRequest,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<WriteOutcome> {
    pull_requests.reply(request).await
}

#[tauri::command]
pub async fn resolve_thread(
    request: ResolveThreadRequest,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<WriteOutcome> {
    pull_requests.resolve(request).await
}

#[tauri::command]
pub async fn submit_review(
    request: SubmitReviewRequest,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<WriteOutcome> {
    pull_requests.submit_review(request).await
}

// Merging (SPEC.md, Merging): confirmed in the UI, checked against the
// branch's tip just before.

#[tauri::command]
pub async fn get_merge_options(
    reference: String,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<MergeOptions> {
    pull_requests.merge_options(&reference).await
}

#[tauri::command]
pub async fn merge_pull_request(
    request: MergeRequest,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<MergeOutcome> {
    pull_requests.merge(request).await
}

/// Each workspace's count of pull requests waiting on your review, for the sidebar.
#[tauri::command]
pub async fn get_review_counts(
    pull_requests: State<'_, PullRequests>,
) -> AppResult<Vec<ReviewCount>> {
    pull_requests.review_counts().await
}

#[tauri::command]
pub async fn get_pull_request_conversation(
    reference: String,
    max_age_seconds: u64,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<Conversation> {
    pull_requests
        .conversation(&reference, max_age_seconds)
        .await
}

#[tauri::command]
pub async fn get_pull_request_diff(
    request: PullRequestDiffRequest,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<PullRequestDiff> {
    pull_requests.diff(request).await
}

#[tauri::command]
pub async fn get_pull_request_checks(
    reference: String,
    max_age_seconds: u64,
    pull_requests: State<'_, PullRequests>,
) -> AppResult<PullRequestChecks> {
    pull_requests.checks(&reference, max_age_seconds).await
}

// Databases (v0.4, SPEC.md section 11): connections, schema, and read-only
// statements in a session per tab.

pub type Databases = Arc<crate::databases::QuerySessions>;

#[tauri::command]
pub async fn list_db_connections(
    databases: State<'_, Databases>,
) -> AppResult<Vec<crate::models::DbConnection>> {
    databases.connections().list().await
}

#[tauri::command]
pub async fn save_db_connection(
    request: crate::models::SaveDbConnectionRequest,
    databases: State<'_, Databases>,
) -> AppResult<crate::models::DbConnection> {
    databases.connections().save(request).await
}

#[tauri::command]
pub async fn delete_db_connection(
    id: String,
    expected_version: i64,
    databases: State<'_, Databases>,
    health: State<'_, Health>,
) -> AppResult<()> {
    let deleted = databases.connections().delete(&id, expected_version).await;
    // Once the connection is gone or being removed, its sessions and Health
    // go too, even when the Keychain could not remove the password.
    if deleted.is_ok() || databases.connections().is_removed(&id).await {
        databases.close_connection(&id);
        health.forget(&id);
    }
    deleted
}

/// Test Connection: connect once with the form's fields.
#[tauri::command]
pub async fn test_db_connection(
    request: crate::models::SaveDbConnectionRequest,
    databases: State<'_, Databases>,
) -> AppResult<crate::models::DbTestResult> {
    databases.connections().test(request).await
}

/// The fields of a pasted `postgres://` URL.
#[tauri::command]
pub async fn parse_db_url(url: String) -> AppResult<crate::models::DbUrlFields> {
    crate::databases::connections::parse_url(&url)
}

/// The password of a connection that asks for it, kept for this run.
#[tauri::command]
pub async fn unlock_db_connection(
    id: String,
    password: String,
    expected_version: i64,
    databases: State<'_, Databases>,
) -> AppResult<crate::models::DbConnection> {
    databases
        .connections()
        .unlock(&id, &password, expected_version)
        .await
}

#[tauri::command]
pub async fn get_db_schema(
    connection_id: String,
    refresh: bool,
    databases: State<'_, Databases>,
) -> AppResult<crate::models::DbSchema> {
    databases.schema(&connection_id, refresh).await
}

/// Run (`Cmd+Enter`) and Run All (`Shift+Cmd+Enter`), read only.
#[tauri::command]
pub async fn run_statement(
    request: crate::models::RunStatementRequest,
    databases: State<'_, Databases>,
) -> AppResult<Vec<crate::models::StatementRun>> {
    databases.run(request).await
}

/// Cancel (`Cmd+.`) the statement a tab is running.
#[tauri::command]
pub async fn cancel_statement(tab_id: String, databases: State<'_, Databases>) -> AppResult<()> {
    databases.cancel(&tab_id).await;
    Ok(())
}

/// Close a tab's session when the tab closes.
#[tauri::command]
pub async fn close_db_session(tab_id: String, databases: State<'_, Databases>) -> AppResult<()> {
    databases.close(&tab_id);
    Ok(())
}

/// Link a connection to a repository, or unlink it.
#[tauri::command]
pub async fn link_db_connection(
    connection_id: String,
    repository_id: String,
    linked: bool,
    databases: State<'_, Databases>,
) -> AppResult<crate::models::DbConnection> {
    databases
        .connections()
        .link(&connection_id, &repository_id, linked)
        .await
}

/// The `:name` parameters a run would ask for.
#[tauri::command]
pub async fn statement_parameters(
    request: crate::models::RunStatementRequest,
    databases: State<'_, Databases>,
) -> AppResult<Vec<String>> {
    databases.parameters(&request).await
}

/// Commit or Roll Back a tab's transaction.
#[tauri::command]
pub async fn end_transaction(
    tab_id: String,
    commit: bool,
    databases: State<'_, Databases>,
) -> AppResult<()> {
    databases.end_transaction(&tab_id, commit).await
}

/// Export…: every row of a statement, run again read only, to a file.
#[tauri::command]
pub async fn export_result(
    request: crate::models::ExportRequest,
    databases: State<'_, Databases>,
) -> AppResult<crate::models::DbExportResult> {
    databases.export(request).await
}

/// The query tabs with a transaction open, which the window asks about
/// before it closes (SPEC.md, Databases: Safety).
#[tauri::command]
pub fn open_db_transactions(databases: State<'_, Databases>) -> Vec<String> {
    databases.open_transactions()
}

pub type SavedQueries = Arc<crate::databases::SavedQueryService>;

#[tauri::command]
pub async fn list_saved_queries(
    queries: State<'_, SavedQueries>,
) -> AppResult<Vec<crate::models::SavedQuery>> {
    queries.list().await
}

#[tauri::command]
pub async fn save_query(
    request: crate::models::SaveQueryRequest,
    queries: State<'_, SavedQueries>,
) -> AppResult<crate::models::SavedQuery> {
    queries.save(request).await
}

#[tauri::command]
pub async fn delete_query(
    id: String,
    expected_version: i64,
    queries: State<'_, SavedQueries>,
) -> AppResult<()> {
    queries.delete(&id, expected_version).await
}

/// Keep the parameter values a saved query last ran with.
#[tauri::command]
pub async fn remember_query_parameters(
    id: String,
    values: Vec<crate::models::ParamValue>,
    queries: State<'_, SavedQueries>,
) -> AppResult<crate::models::SavedQuery> {
    queries.remember_parameters(&id, values).await
}

fn history_store(databases: &Databases) -> AppResult<&crate::databases::history::QueryHistory> {
    databases
        .history()
        .ok_or_else(|| crate::models::AppError::io("The history is not available."))
}

#[tauri::command]
pub async fn query_history(
    connection_id: String,
    search: String,
    offset: u32,
    limit: u32,
    databases: State<'_, Databases>,
) -> AppResult<Vec<crate::models::HistoryEntry>> {
    history_store(&databases)?
        .list(&connection_id, &search, offset, limit)
        .await
}

#[tauri::command]
pub async fn clear_query_history(
    connection_id: String,
    databases: State<'_, Databases>,
) -> AppResult<()> {
    history_store(&databases)?.clear(&connection_id).await
}

/// The query tabs open when Brainiac quit, with their text.
#[tauri::command]
pub async fn list_query_tabs(
    databases: State<'_, Databases>,
) -> AppResult<Vec<crate::models::QueryTab>> {
    history_store(&databases)?.tabs().await
}

#[tauri::command]
pub async fn save_query_tabs(
    tabs: Vec<crate::models::QueryTab>,
    databases: State<'_, Databases>,
) -> AppResult<()> {
    history_store(&databases)?.save_tabs(tabs).await
}

pub type Health = Arc<crate::databases::health::HealthService>;

/// Health became visible: sample every 10 seconds; returns the hour so far.
#[tauri::command]
pub async fn start_db_health(
    connection_id: String,
    health: State<'_, Health>,
) -> AppResult<crate::models::HealthSnapshot> {
    health.inner().start(&connection_id).await
}

/// Health was hidden or closed.
#[tauri::command]
pub async fn stop_db_health(connection_id: String, health: State<'_, Health>) -> AppResult<()> {
    health.stop(&connection_id);
    Ok(())
}

/// Cancel Query (`terminate` false) or End Session (`terminate` true) on a
/// server session, after the window asked.
#[tauri::command]
pub async fn signal_db_backend(
    connection_id: String,
    pid: i32,
    terminate: bool,
    health: State<'_, Health>,
) -> AppResult<bool> {
    health.signal_backend(&connection_id, pid, terminate).await
}

/// Running Docker containers, for a connection's Runs on.
#[tauri::command]
pub async fn list_docker_containers(
    socket: Option<String>,
) -> AppResult<crate::models::DockerContainers> {
    crate::databases::health::list_containers(socket.as_deref()).await
}
