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
import type { CreateWorkspaceRequest } from "./generated/CreateWorkspaceRequest";
import type { DiffOptions } from "./generated/DiffOptions";
import type { DiffResult } from "./generated/DiffResult";
import type { DiffSelector } from "./generated/DiffSelector";
import type { FetchResult } from "./generated/FetchResult";
import type { ListCommitsRequest } from "./generated/ListCommitsRequest";
import type { MenuEvent } from "./generated/MenuEvent";
import type { PinEntityType } from "./generated/PinEntityType";
import type { RefsResult } from "./generated/RefsResult";
import type { RelocateRepositoryRequest } from "./generated/RelocateRepositoryRequest";
import type { RelocationOutcome } from "./generated/RelocationOutcome";
import type { RepositoryChangedEvent } from "./generated/RepositoryChangedEvent";
import type { RepositorySummary } from "./generated/RepositorySummary";
import type { RepositoryTab } from "./generated/RepositoryTab";
import type { TeamPulse } from "./generated/TeamPulse";
import type { UpdateWorkspaceMembershipRequest } from "./generated/UpdateWorkspaceMembershipRequest";
import type { Workspace } from "./generated/Workspace";
import type { WorkspaceActivity } from "./generated/WorkspaceActivity";
import type { WorkspacePreview } from "./generated/WorkspacePreview";
import type { WorkspaceRescan } from "./generated/WorkspaceRescan";

export type { ActivityCommit } from "./generated/ActivityCommit";
export type { ActivityItem } from "./generated/ActivityItem";
export type { ActivityKind } from "./generated/ActivityKind";
export type { ChangeEntry } from "./generated/ChangeEntry";
export type { CommitFile } from "./generated/CommitFile";
export type { CommitSummary } from "./generated/CommitSummary";
export type { DiffContent } from "./generated/DiffContent";
export type { DiffLine } from "./generated/DiffLine";
export type { DiscoveryMode } from "./generated/DiscoveryMode";
export type { Hunk } from "./generated/Hunk";
export type { MemberOrigin } from "./generated/MemberOrigin";
export type { MemberStatus } from "./generated/MemberStatus";
export type { Pin } from "./generated/Pin";
export type { PreviewStatus } from "./generated/PreviewStatus";
export type { RefEntry } from "./generated/RefEntry";
export type { RelocationConcern } from "./generated/RelocationConcern";
export type { RepositoryFreshness } from "./generated/RepositoryFreshness";
export type { SuggestedMove } from "./generated/SuggestedMove";
export type { WorkspaceMember } from "./generated/WorkspaceMember";
export type { WorkspacePreviewEntry } from "./generated/WorkspacePreviewEntry";
export type {
  ActivitySettings,
  AppError,
  AppSnapshot,
  ChangesResult,
  CommitDetail,
  CommitPage,
  CreateWorkspaceRequest,
  DiffOptions,
  DiffResult,
  DiffSelector,
  FetchResult,
  ListCommitsRequest,
  PinEntityType,
  RefsResult,
  RelocateRepositoryRequest,
  RelocationOutcome,
  RepositorySummary,
  RepositoryTab,
  TeamPulse,
  UpdateWorkspaceMembershipRequest,
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
