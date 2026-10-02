/**
 * Typed wrappers around Tauri `invoke`. Types come from `generated/`, which
 * `cargo test` exports from the Rust DTOs; do not edit those files by hand.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ActivitySettings } from "./generated/ActivitySettings";
import type { AppError } from "./generated/AppError";
import type { AppSnapshot } from "./generated/AppSnapshot";
import type { ChangesResult } from "./generated/ChangesResult";
import type { CommitDetail } from "./generated/CommitDetail";
import type { CommitPage } from "./generated/CommitPage";
import type { CreateNoteRequest } from "./generated/CreateNoteRequest";
import type { CreateWorkspaceRequest } from "./generated/CreateWorkspaceRequest";
import type { DiffOptions } from "./generated/DiffOptions";
import type { DiffResult } from "./generated/DiffResult";
import type { DiffSelector } from "./generated/DiffSelector";
import type { ExportResult } from "./generated/ExportResult";
import type { FetchResult } from "./generated/FetchResult";
import type { FolderListing } from "./generated/FolderListing";
import type { IndexStatus } from "./generated/IndexStatus";
import type { ListCommitsRequest } from "./generated/ListCommitsRequest";
import type { MenuEvent } from "./generated/MenuEvent";
import type { NoteChangedEvent } from "./generated/NoteChangedEvent";
import type { NoteContent } from "./generated/NoteContent";
import type { NoteContext } from "./generated/NoteContext";
import type { NoteLists } from "./generated/NoteLists";
import type { NoteMissingEvent } from "./generated/NoteMissingEvent";
import type { NoteRevision } from "./generated/NoteRevision";
import type { NoteSummary } from "./generated/NoteSummary";
import type { PinEntityType } from "./generated/PinEntityType";
import type { RefsResult } from "./generated/RefsResult";
import type { RelocateRepositoryRequest } from "./generated/RelocateRepositoryRequest";
import type { RelocationOutcome } from "./generated/RelocationOutcome";
import type { RenameNoteRequest } from "./generated/RenameNoteRequest";
import type { RenamePreview } from "./generated/RenamePreview";
import type { RenameResult } from "./generated/RenameResult";
import type { RepositoryChangedEvent } from "./generated/RepositoryChangedEvent";
import type { RepositoryNotes } from "./generated/RepositoryNotes";
import type { RepositorySummary } from "./generated/RepositorySummary";
import type { RepositoryTab } from "./generated/RepositoryTab";
import type { ResolvedLink } from "./generated/ResolvedLink";
import type { RestorePreview } from "./generated/RestorePreview";
import type { RestoreRequest } from "./generated/RestoreRequest";
import type { RestoreResult } from "./generated/RestoreResult";
import type { SaveNoteRequest } from "./generated/SaveNoteRequest";
import type { SaveNoteResult } from "./generated/SaveNoteResult";
import type { SearchRequest } from "./generated/SearchRequest";
import type { SearchResults } from "./generated/SearchResults";
import type { Settings } from "./generated/Settings";
import type { Task } from "./generated/Task";
import type { TaskChangedEvent } from "./generated/TaskChangedEvent";
import type { TaskFields } from "./generated/TaskFields";
import type { TaskFilter } from "./generated/TaskFilter";
import type { TeamPulse } from "./generated/TeamPulse";
import type { TodayView } from "./generated/TodayView";
import type { TrashedNote } from "./generated/TrashedNote";
import type { UpdateTaskRequest } from "./generated/UpdateTaskRequest";
import type { UpdateWorkspaceMembershipRequest } from "./generated/UpdateWorkspaceMembershipRequest";
import type { VaultState } from "./generated/VaultState";
import type { Workspace } from "./generated/Workspace";
import type { WorkspaceActivity } from "./generated/WorkspaceActivity";
import type { WorkspacePreview } from "./generated/WorkspacePreview";
import type { WorkspaceRescan } from "./generated/WorkspaceRescan";

export type { ActivityCommit } from "./generated/ActivityCommit";
export type { ActivityItem } from "./generated/ActivityItem";
export type { ActivityKind } from "./generated/ActivityKind";
export type { Backlink } from "./generated/Backlink";
export type { ChangeEntry } from "./generated/ChangeEntry";
export type { CommitFile } from "./generated/CommitFile";
export type { CommitSummary } from "./generated/CommitSummary";
export type { DiffContent } from "./generated/DiffContent";
export type { DiffLine } from "./generated/DiffLine";
export type { DiscoveryMode } from "./generated/DiscoveryMode";
export type { FolderEntry } from "./generated/FolderEntry";
export type { Hunk } from "./generated/Hunk";
export type { IndexState } from "./generated/IndexState";
export type { LinkedRepository } from "./generated/LinkedRepository";
export type { MemberOrigin } from "./generated/MemberOrigin";
export type { MemberStatus } from "./generated/MemberStatus";
export type { NoteDraft } from "./generated/NoteDraft";
export type { NoteTextState } from "./generated/NoteTextState";
export type { Pin } from "./generated/Pin";
export type { PreviewStatus } from "./generated/PreviewStatus";
export type { RefEntry } from "./generated/RefEntry";
export type { RelocationConcern } from "./generated/RelocationConcern";
export type { RepositoryFreshness } from "./generated/RepositoryFreshness";
export type { RepositorySuggestion } from "./generated/RepositorySuggestion";
export type { SearchGroup } from "./generated/SearchGroup";
export type { SearchHit } from "./generated/SearchHit";
export type { SearchKind } from "./generated/SearchKind";
export type { SuggestedMove } from "./generated/SuggestedMove";
export type { TaskNote } from "./generated/TaskNote";
export type { TaskStatus } from "./generated/TaskStatus";
export type { TextPart } from "./generated/TextPart";
export type { UnresolvedLink } from "./generated/UnresolvedLink";
export type { VaultInfo } from "./generated/VaultInfo";
export type { WorkspaceMember } from "./generated/WorkspaceMember";
export type { WorkspacePreviewEntry } from "./generated/WorkspacePreviewEntry";
export type {
  ActivitySettings,
  AppError,
  AppSnapshot,
  ChangesResult,
  CommitDetail,
  CommitPage,
  CreateNoteRequest,
  CreateWorkspaceRequest,
  DiffOptions,
  DiffResult,
  DiffSelector,
  ExportResult,
  FetchResult,
  FolderListing,
  IndexStatus,
  ListCommitsRequest,
  NoteChangedEvent,
  NoteContent,
  NoteContext,
  NoteLists,
  NoteMissingEvent,
  NoteRevision,
  NoteSummary,
  PinEntityType,
  RefsResult,
  RelocateRepositoryRequest,
  RelocationOutcome,
  RenameNoteRequest,
  RenamePreview,
  RenameResult,
  RepositoryNotes,
  RepositorySummary,
  RepositoryTab,
  ResolvedLink,
  RestorePreview,
  RestoreRequest,
  RestoreResult,
  SaveNoteRequest,
  SaveNoteResult,
  SearchRequest,
  SearchResults,
  Settings,
  Task,
  TaskChangedEvent,
  TaskFields,
  TaskFilter,
  TeamPulse,
  TodayView,
  TrashedNote,
  UpdateTaskRequest,
  UpdateWorkspaceMembershipRequest,
  VaultState,
  Workspace,
  WorkspaceActivity,
  WorkspacePreview,
  WorkspaceRescan,
};

export function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

export function errorMessage(e: unknown): string {
  if (isAppError(e)) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

export const ipc = {
  getAppSnapshot: () => invoke<AppSnapshot>("get_app_snapshot"),
  registerRepository: (path: string) =>
    invoke<RepositorySummary>("register_repository", { path }),
  removeRepository: (repositoryId: string) =>
    invoke<void>("remove_repository", { repositoryId }),
  relocateRepository: (request: RelocateRepositoryRequest) =>
    invoke<RelocationOutcome>("relocate_repository", { request }),
  refreshRepository: (repositoryId: string) =>
    invoke<RepositorySummary>("refresh_repository", { repositoryId }),
  openRepository: (repositoryId: string) =>
    invoke<void>("open_repository", { repositoryId }),
  setRepositoryTab: (repositoryId: string, tab: RepositoryTab) =>
    invoke<void>("set_repository_tab", { repositoryId, tab }),
  listChanges: (repositoryId: string) =>
    invoke<ChangesResult>("list_changes", { repositoryId }),
  getDiff: (
    repositoryId: string,
    selector: DiffSelector,
    options: DiffOptions = { ignore_whitespace: false },
  ) => invoke<DiffResult>("get_diff", { repositoryId, selector, options }),
  /** Missing optional fields default to null. */
  listCommits: (
    request: Pick<ListCommitsRequest, "repository_id"> &
      Partial<ListCommitsRequest>,
  ) =>
    invoke<CommitPage>("list_commits", {
      request: {
        ref: null,
        filter: null,
        cursor: null,
        limit: null,
        author: null,
        exclude: null,
        ...request,
      },
    }),
  getCommit: (repositoryId: string, commitId: string, parentIndex?: number) =>
    invoke<CommitDetail>("get_commit", {
      repositoryId,
      commitId,
      parentIndex: parentIndex ?? null,
    }),
  listRefs: (repositoryId: string) =>
    invoke<RefsResult>("list_refs", { repositoryId }),
  openInEditor: (repositoryId: string, path?: string, line?: number) =>
    invoke<void>("open_in_editor", {
      repositoryId,
      path: path ?? null,
      line: line ?? null,
    }),
  revealInFinder: (repositoryId: string, path?: string) =>
    invoke<void>("reveal_in_finder", { repositoryId, path: path ?? null }),
  discoverRepositories: (folderPath: string, discoveryPath?: string) =>
    invoke<WorkspacePreview>("discover_repositories", {
      folderPath,
      discoveryPath: discoveryPath ?? null,
    }),
  rescanWorkspace: (workspaceId: string) =>
    invoke<WorkspaceRescan>("rescan_workspace", { workspaceId }),
  createWorkspace: (request: CreateWorkspaceRequest) =>
    invoke<Workspace>("create_workspace", { request }),
  updateWorkspaceMembership: (request: UpdateWorkspaceMembershipRequest) =>
    invoke<Workspace>("update_workspace_membership", { request }),
  renameWorkspace: (workspaceId: string, name: string) =>
    invoke<Workspace>("rename_workspace", { workspaceId, name }),
  removeWorkspace: (workspaceId: string) =>
    invoke<void>("remove_workspace", { workspaceId }),
  setPinned: (entityType: PinEntityType, entityId: string, pinned: boolean) =>
    invoke<void>("set_pinned", { entityType, entityId, pinned }),
  fetchRepository: (repositoryId: string) =>
    invoke<FetchResult>("fetch_repository", { repositoryId }),
  getWorkspaceActivity: (workspaceId: string) =>
    invoke<WorkspaceActivity>("get_workspace_activity", { workspaceId }),
  getTeamPulse: (workspaceId: string) =>
    invoke<TeamPulse>("get_team_pulse", { workspaceId }),
  /** Without `eventIds`, marks every unread event of the workspace. */
  markActivitySeen: (workspaceId: string, eventIds?: string[]) =>
    invoke<void>("mark_activity_seen", {
      workspaceId,
      eventIds: eventIds ?? null,
    }),
  updateActivitySettings: (workspaceId: string, settings: ActivitySettings) =>
    invoke<Workspace>("update_activity_settings", { workspaceId, settings }),
  updateSettings: (settings: Settings) =>
    invoke<Settings>("update_settings", { settings }),

  // v0.2: vault and notes
  getVaultState: () => invoke<VaultState>("get_vault_state"),
  /** `create` makes a new, empty vault folder at `path`. */
  selectVault: (path: string, create: boolean) =>
    invoke<VaultState>("select_vault", { path, create }),
  listFolder: (folder?: string) =>
    invoke<FolderListing>("list_folder", { folder: folder ?? null }),
  getNoteLists: () => invoke<NoteLists>("get_note_lists"),
  readNote: (noteId: string) => invoke<NoteContent>("read_note", { noteId }),
  markNoteOpened: (noteId: string) =>
    invoke<void>("mark_note_opened", { noteId }),
  saveNote: (request: SaveNoteRequest) =>
    invoke<SaveNoteResult>("save_note", { request }),
  createNote: (request: Partial<CreateNoteRequest>) =>
    invoke<NoteSummary>("create_note", {
      request: { folder: null, title: null, repository_id: null, ...request },
    }),
  previewRename: (noteId: string, newPath: string) =>
    invoke<RenamePreview>("preview_rename", { noteId, newPath }),
  renameNote: (request: RenameNoteRequest) =>
    invoke<RenameResult>("rename_note", { request }),
  followNoteTitle: (noteId: string, fromTitle: string) =>
    invoke<NoteSummary>("follow_note_title", { noteId, fromTitle }),
  trashNote: (noteId: string) => invoke<void>("trash_note", { noteId }),
  listTrash: () => invoke<TrashedNote[]>("list_trash"),
  restoreNote: (noteId: string, overwrite: boolean) =>
    invoke<NoteSummary>("restore_note", { noteId, overwrite }),
  recreateNote: (noteId: string, text: string) =>
    invoke<NoteSummary>("recreate_note", { noteId, text }),
  relinkNote: (noteId: string, path: string) =>
    invoke<NoteSummary>("relink_note", { noteId, path }),
  listRevisions: (noteId: string) =>
    invoke<NoteRevision[]>("list_revisions", { noteId }),
  readRevision: (revisionId: string) =>
    invoke<string>("read_revision", { revisionId }),
  restoreRevision: (
    noteId: string,
    revisionId: string,
    expectedVersion: string,
  ) =>
    invoke<SaveNoteResult>("restore_revision", {
      noteId,
      revisionId,
      expectedVersion,
    }),
  /** Keep edits that cannot be saved now as the note's draft. */
  saveDraft: (noteId: string, baseVersion: string, text: string) =>
    invoke<void>("save_draft", { noteId, baseVersion, text }),
  discardDraft: (noteId: string) => invoke<void>("discard_draft", { noteId }),
  saveDraftAsCopy: (noteId: string, text: string) =>
    invoke<NoteSummary>("save_draft_as_copy", { noteId, text }),
  openVaultFile: (relativePath: string) =>
    invoke<void>("open_vault_file", { relativePath }),
  revealVaultPath: (relativePath?: string) =>
    invoke<void>("reveal_vault_path", { relativePath: relativePath ?? null }),
  getNoteContext: (noteId: string) =>
    invoke<NoteContext>("get_note_context", { noteId }),
  resolveLink: (noteId: string, target: string, wikilink: boolean) =>
    invoke<ResolvedLink>("resolve_link", { noteId, target, wikilink }),
  getRepositoryNotes: (repositoryId: string) =>
    invoke<RepositoryNotes>("get_repository_notes", { repositoryId }),
  linkRepository: (noteId: string, repositoryId: string) =>
    invoke<void>("link_repository", { noteId, repositoryId }),
  unlinkRepository: (noteId: string, repositoryId: string) =>
    invoke<void>("unlink_repository", { noteId, repositoryId }),
  dismissSuggestion: (noteId: string, repositoryId: string) =>
    invoke<void>("dismiss_suggestion", { noteId, repositoryId }),
  reconnectRepository: (from: string, to: string) =>
    invoke<void>("reconnect_repository", { from, to }),

  // v0.2: tasks
  listTasks: (filter?: Partial<TaskFilter>) =>
    invoke<Task[]>("list_tasks", {
      filter: {
        statuses: null,
        to_sort: null,
        note_id: null,
        repository_id: null,
        ...filter,
      },
    }),
  getTask: (taskId: string) => invoke<Task>("get_task", { taskId }),
  getToday: () => invoke<TodayView>("get_today"),
  createTask: (fields: TaskFields) => invoke<Task>("create_task", { fields }),
  updateTask: (request: UpdateTaskRequest) =>
    invoke<Task>("update_task", { request }),
  deleteTask: (taskId: string, expectedVersion: number) =>
    invoke<void>("delete_task", { taskId, expectedVersion }),

  // v0.2: search and backups
  search: (request: SearchRequest) =>
    invoke<SearchResults>("search", { request }),
  rebuildSearch: () => invoke<void>("rebuild_search"),
  exportBackup: (folder: string) =>
    invoke<ExportResult>("export_backup", { folder }),
  previewRestore: (path: string) =>
    invoke<RestorePreview>("preview_restore", { path }),
  restoreBackup: (request: RestoreRequest) =>
    invoke<RestoreResult>("restore_backup", { request }),
};

export function onRepositoryChanged(
  handler: (e: RepositoryChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<RepositoryChangedEvent>("repository_changed", (ev) =>
    handler(ev.payload),
  );
}

export function onMenu(handler: (e: MenuEvent) => void): Promise<UnlistenFn> {
  return listen<MenuEvent>("menu", (ev) => handler(ev.payload));
}

export function onNoteChanged(
  handler: (e: NoteChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<NoteChangedEvent>("note_changed", (ev) => handler(ev.payload));
}

export function onNoteMissing(
  handler: (e: NoteMissingEvent) => void,
): Promise<UnlistenFn> {
  return listen<NoteMissingEvent>("note_missing", (ev) => handler(ev.payload));
}

export function onTaskChanged(
  handler: (e: TaskChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<TaskChangedEvent>("task_changed", (ev) => handler(ev.payload));
}

export function onIndexStatusChanged(
  handler: (e: IndexStatus) => void,
): Promise<UnlistenFn> {
  return listen<IndexStatus>("index_status_changed", (ev) =>
    handler(ev.payload),
  );
}

/** Subscribe in an effect; returns the cleanup. Late subscriptions are undone. */
export function subscribe(
  ...subscriptions: Array<Promise<UnlistenFn>>
): () => void {
  let disposed = false;
  const unlisteners: UnlistenFn[] = [];
  for (const s of subscriptions)
    void s.then((u) => (disposed ? u() : unlisteners.push(u)));
  return () => {
    disposed = true;
    for (const u of unlisteners) u();
  };
}
