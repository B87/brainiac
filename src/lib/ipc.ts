/**
 * Typed wrappers around Tauri `invoke`. Types come from `generated/`, which
 * `cargo test` exports from the Rust DTOs; do not edit those files by hand.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ActivitySettings } from "./generated/ActivitySettings";
import type { AgentAccessStatus } from "./generated/AgentAccessStatus";
import type { AgentEngine } from "./generated/AgentEngine";
import type { AgentHost } from "./generated/AgentHost";
import type { AgentHostPreview } from "./generated/AgentHostPreview";
import type { AgentProvider } from "./generated/AgentProvider";
import type { AgentRun } from "./generated/AgentRun";
import type { AgentRunChangedEvent } from "./generated/AgentRunChangedEvent";
import type { AgentRunList } from "./generated/AgentRunList";
import type { AgentSettings } from "./generated/AgentSettings";
import type { AgentTestResult } from "./generated/AgentTestResult";
import type { AnswerExplainRequest } from "./generated/AnswerExplainRequest";
import type { AppError } from "./generated/AppError";
import type { ApproveAgentHostRequest } from "./generated/ApproveAgentHostRequest";
import type { AppSnapshot } from "./generated/AppSnapshot";
import type { BranchComparison } from "./generated/BranchComparison";
import type { Cell } from "./generated/Cell";
import type { CellValue } from "./generated/CellValue";
import type { ChangesResult } from "./generated/ChangesResult";
import type { ColumnKind } from "./generated/ColumnKind";
import type { CommentRequest } from "./generated/CommentRequest";
import type { CommitDetail } from "./generated/CommitDetail";
import type { CommitPage } from "./generated/CommitPage";
import type { ConceptKind } from "./generated/ConceptKind";
import type { Conversation } from "./generated/Conversation";
import type { CreateNoteRequest } from "./generated/CreateNoteRequest";
import type { CreateWorkspaceRequest } from "./generated/CreateWorkspaceRequest";
import type { CredentialOwner } from "./generated/CredentialOwner";
import type { CredentialPending } from "./generated/CredentialPending";
import type { CredentialState } from "./generated/CredentialState";
import type { CredentialTest } from "./generated/CredentialTest";
import type { DbAccess } from "./generated/DbAccess";
import type { DbColumn } from "./generated/DbColumn";
import type { DbConnection } from "./generated/DbConnection";
import type { DbEnvironment } from "./generated/DbEnvironment";
import type { DbExportResult } from "./generated/DbExportResult";
import type { DbFailure } from "./generated/DbFailure";
import type { DbFailureReason } from "./generated/DbFailureReason";
import type { DbForeignKey } from "./generated/DbForeignKey";
import type { DbIndex } from "./generated/DbIndex";
import type { DbKind } from "./generated/DbKind";
import type { DbRelation } from "./generated/DbRelation";
import type { DbSchema } from "./generated/DbSchema";
import type { DbSchemaGroup } from "./generated/DbSchemaGroup";
import type { DbTestResult } from "./generated/DbTestResult";
import type { DbTls } from "./generated/DbTls";
import type { DbUrlFields } from "./generated/DbUrlFields";
import type { DiffOptions } from "./generated/DiffOptions";
import type { DiffResult } from "./generated/DiffResult";
import type { DiffSelector } from "./generated/DiffSelector";
import type { DockerContainer } from "./generated/DockerContainer";
import type { DockerContainers } from "./generated/DockerContainers";
import type { ExplainAnswerScope } from "./generated/ExplainAnswerScope";
import type { ExplainDialog } from "./generated/ExplainDialog";
import type { ExplainMode } from "./generated/ExplainMode";
import type { ExplainSubject } from "./generated/ExplainSubject";
import type { ExplanationChangedEvent } from "./generated/ExplanationChangedEvent";
import type { ExplanationPlacement } from "./generated/ExplanationPlacement";
import type { ExplanationRecord } from "./generated/ExplanationRecord";
import type { ExplanationSettings } from "./generated/ExplanationSettings";
import type { ExplanationSettingsView } from "./generated/ExplanationSettingsView";
import type { ExportFormat } from "./generated/ExportFormat";
import type { ExportRequest } from "./generated/ExportRequest";
import type { ExportResult } from "./generated/ExportResult";
import type { FetchResult } from "./generated/FetchResult";
import type { FolderListing } from "./generated/FolderListing";
import type { ForgeAccountSlot } from "./generated/ForgeAccountSlot";
import type { ForgeAccountTestResult } from "./generated/ForgeAccountTestResult";
import type { ForgeKind } from "./generated/ForgeKind";
import type { HealthEvent } from "./generated/HealthEvent";
import type { HealthPoint } from "./generated/HealthPoint";
import type { HealthSample } from "./generated/HealthSample";
import type { HealthSession } from "./generated/HealthSession";
import type { HealthSnapshot } from "./generated/HealthSnapshot";
import type { HealthStatement } from "./generated/HealthStatement";
import type { HealthTable } from "./generated/HealthTable";
import type { HistoryEntry } from "./generated/HistoryEntry";
import type { HostJob } from "./generated/HostJob";
import type { HostJobKind } from "./generated/HostJobKind";
import type { IndexStatus } from "./generated/IndexStatus";
import type { ListCommitsRequest } from "./generated/ListCommitsRequest";
import type { ListPullRequestsRequest } from "./generated/ListPullRequestsRequest";
import type { MachineSample } from "./generated/MachineSample";
import type { MenuEvent } from "./generated/MenuEvent";
import type { MergeMethod } from "./generated/MergeMethod";
import type { MergeOptions } from "./generated/MergeOptions";
import type { MergeOutcome } from "./generated/MergeOutcome";
import type { MergeRequest } from "./generated/MergeRequest";
import type { NoteChangedEvent } from "./generated/NoteChangedEvent";
import type { NoteContent } from "./generated/NoteContent";
import type { NoteContext } from "./generated/NoteContext";
import type { NoteLists } from "./generated/NoteLists";
import type { NoteMissingEvent } from "./generated/NoteMissingEvent";
import type { NoteRevision } from "./generated/NoteRevision";
import type { NoteSummary } from "./generated/NoteSummary";
import type { ParamValue } from "./generated/ParamValue";
import type { PinEntityType } from "./generated/PinEntityType";
import type { PlanNode } from "./generated/PlanNode";
import type { PullRequest } from "./generated/PullRequest";
import type { PullRequestChangedEvent } from "./generated/PullRequestChangedEvent";
import type { PullRequestChecks } from "./generated/PullRequestChecks";
import type { PullRequestDiff } from "./generated/PullRequestDiff";
import type { PullRequestDiffRequest } from "./generated/PullRequestDiffRequest";
import type { PullRequestFiles } from "./generated/PullRequestFiles";
import type { PullRequestList } from "./generated/PullRequestList";
import type { QueryTab } from "./generated/QueryTab";
import type { RefsResult } from "./generated/RefsResult";
import type { RelationKind } from "./generated/RelationKind";
import type { RelocateRepositoryRequest } from "./generated/RelocateRepositoryRequest";
import type { RelocationOutcome } from "./generated/RelocationOutcome";
import type { RenameNoteRequest } from "./generated/RenameNoteRequest";
import type { RenamePreview } from "./generated/RenamePreview";
import type { RenameResult } from "./generated/RenameResult";
import type { ReplyRequest } from "./generated/ReplyRequest";
import type { RepositoryChangedEvent } from "./generated/RepositoryChangedEvent";
import type { RepositoryNotes } from "./generated/RepositoryNotes";
import type { RepositorySummary } from "./generated/RepositorySummary";
import type { RepositoryTab } from "./generated/RepositoryTab";
import type { ResolvedLink } from "./generated/ResolvedLink";
import type { ResolveThreadRequest } from "./generated/ResolveThreadRequest";
import type { RestorePreview } from "./generated/RestorePreview";
import type { RestoreRequest } from "./generated/RestoreRequest";
import type { RestoreResult } from "./generated/RestoreResult";
import type { ResultColumn } from "./generated/ResultColumn";
import type { ReviewCount } from "./generated/ReviewCount";
import type { ReviewDrafts } from "./generated/ReviewDrafts";
import type { RunBranchCommand } from "./generated/RunBranchCommand";
import type { RunChanges } from "./generated/RunChanges";
import type { RunControllerStatus } from "./generated/RunControllerStatus";
import type { RunDiffRequest } from "./generated/RunDiffRequest";
import type { RunEventPage } from "./generated/RunEventPage";
import type { RunMode } from "./generated/RunMode";
import type { RunPreview } from "./generated/RunPreview";
import type { RunStartPreview } from "./generated/RunStartPreview";
import type { RunStatementRequest } from "./generated/RunStatementRequest";
import type { RunsOn } from "./generated/RunsOn";
import type { SaveAgentCredentialRequest } from "./generated/SaveAgentCredentialRequest";
import type { SaveAgentSettingsRequest } from "./generated/SaveAgentSettingsRequest";
import type { SaveDbConnectionRequest } from "./generated/SaveDbConnectionRequest";
import type { SavedQuery } from "./generated/SavedQuery";
import type { SaveForgeAccountOutcome } from "./generated/SaveForgeAccountOutcome";
import type { SaveForgeAccountRequest } from "./generated/SaveForgeAccountRequest";
import type { SaveNoteRequest } from "./generated/SaveNoteRequest";
import type { SaveNoteResult } from "./generated/SaveNoteResult";
import type { SaveQueryRequest } from "./generated/SaveQueryRequest";
import type { SaveReviewDraftRequest } from "./generated/SaveReviewDraftRequest";
import type { SearchRequest } from "./generated/SearchRequest";
import type { SearchResults } from "./generated/SearchResults";
import type { SecretEntry } from "./generated/SecretEntry";
import type { SecretSource } from "./generated/SecretSource";
import type { SecretsOverview } from "./generated/SecretsOverview";
import type { SetRepositoryForgeRequest } from "./generated/SetRepositoryForgeRequest";
import type { Settings } from "./generated/Settings";
import type { StartExplanationRequest } from "./generated/StartExplanationRequest";
import type { StartRunRequest } from "./generated/StartRunRequest";
import type { StatementResult } from "./generated/StatementResult";
import type { StatementRun } from "./generated/StatementRun";
import type { SubmitReviewRequest } from "./generated/SubmitReviewRequest";
import type { Task } from "./generated/Task";
import type { TaskChangedEvent } from "./generated/TaskChangedEvent";
import type { TaskFields } from "./generated/TaskFields";
import type { TaskFilter } from "./generated/TaskFilter";
import type { TeamPulse } from "./generated/TeamPulse";
import type { TodayView } from "./generated/TodayView";
import type { TransactionInfo } from "./generated/TransactionInfo";
import type { TrashedNote } from "./generated/TrashedNote";
import type { UpdateTaskRequest } from "./generated/UpdateTaskRequest";
import type { UpdateWorkspaceMembershipRequest } from "./generated/UpdateWorkspaceMembershipRequest";
import type { VaultState } from "./generated/VaultState";
import type { Workspace } from "./generated/Workspace";
import type { WorkspaceActivity } from "./generated/WorkspaceActivity";
import type { WorkspacePreview } from "./generated/WorkspacePreview";
import type { WorkspaceRescan } from "./generated/WorkspaceRescan";
import type { WriteOutcome } from "./generated/WriteOutcome";

export type { ActionAvailability } from "./generated/ActionAvailability";
export type { ActivityCommit } from "./generated/ActivityCommit";
export type { ActivityItem } from "./generated/ActivityItem";
export type { ActivityKind } from "./generated/ActivityKind";
export type { AgentAccess } from "./generated/AgentAccess";
export type { AgentEngine } from "./generated/AgentEngine";
export type { AgentHost } from "./generated/AgentHost";
export type { AgentHostPreview } from "./generated/AgentHostPreview";
export type { AgentImage } from "./generated/AgentImage";
export type { AgentKind } from "./generated/AgentKind";
export type { AgentPayment } from "./generated/AgentPayment";
export type { AgentProfile } from "./generated/AgentProfile";
export type { AgentProvider } from "./generated/AgentProvider";
export type { AgentRun } from "./generated/AgentRun";
export type { AgentRunChangedEvent } from "./generated/AgentRunChangedEvent";
export type { AgentRunList } from "./generated/AgentRunList";
export type { AgentSettings } from "./generated/AgentSettings";
export type { AgentTest } from "./generated/AgentTest";
export type { AgentTestResult } from "./generated/AgentTestResult";
export type { AgentTestStep } from "./generated/AgentTestStep";
export type { AnswerExplainRequest } from "./generated/AnswerExplainRequest";
export type { Backlink } from "./generated/Backlink";
export type { BranchComparison } from "./generated/BranchComparison";
export type { ChangedFile } from "./generated/ChangedFile";
export type { ChangedFileStatus } from "./generated/ChangedFileStatus";
export type { ChangeEntry } from "./generated/ChangeEntry";
export type { Check } from "./generated/Check";
export type { CheckState } from "./generated/CheckState";
export type { ChecksSummary } from "./generated/ChecksSummary";
export type { CitedQuote } from "./generated/CitedQuote";
export type { Comment } from "./generated/Comment";
export type { CommitFile } from "./generated/CommitFile";
export type { CommitSummary } from "./generated/CommitSummary";
export type { Concept } from "./generated/Concept";
export type { ConceptKind } from "./generated/ConceptKind";
export type { DiffContent } from "./generated/DiffContent";
export type { DiffLine } from "./generated/DiffLine";
export type { DiffSide } from "./generated/DiffSide";
export type { DiffSource } from "./generated/DiffSource";
export type { Disagreement } from "./generated/Disagreement";
export type { DiscoveryMode } from "./generated/DiscoveryMode";
export type { ExplainAnswer } from "./generated/ExplainAnswer";
export type { ExplainAnswerScope } from "./generated/ExplainAnswerScope";
export type { ExplainConsent } from "./generated/ExplainConsent";
export type { ExplainConsentState } from "./generated/ExplainConsentState";
export type { ExplainCost } from "./generated/ExplainCost";
export type { ExplainDepth } from "./generated/ExplainDepth";
export type { ExplainDialog } from "./generated/ExplainDialog";
export type { ExplainEstimate } from "./generated/ExplainEstimate";
export type { ExplainHostOption } from "./generated/ExplainHostOption";
export type { ExplainProfileOption } from "./generated/ExplainProfileOption";
export type { ExplainStep } from "./generated/ExplainStep";
export type { ExplainSubject } from "./generated/ExplainSubject";
export type { ExplainSubjectKind } from "./generated/ExplainSubjectKind";
export type { ExplainWorkspaceOption } from "./generated/ExplainWorkspaceOption";
export type { Explanation } from "./generated/Explanation";
export type { ExplanationChangedEvent } from "./generated/ExplanationChangedEvent";
export type { ExplanationChecks } from "./generated/ExplanationChecks";
export type { ExplanationNote } from "./generated/ExplanationNote";
export type { ExplanationPlacement } from "./generated/ExplanationPlacement";
export type { ExplanationRecord } from "./generated/ExplanationRecord";
export type { ExplanationSettings } from "./generated/ExplanationSettings";
export type { ExplanationSettingsView } from "./generated/ExplanationSettingsView";
export type { ExplanationState } from "./generated/ExplanationState";
export type { ExplanationSummary } from "./generated/ExplanationSummary";
export type { FolderEntry } from "./generated/FolderEntry";
export type { ForgeAccount } from "./generated/ForgeAccount";
export type { ForgeSource } from "./generated/ForgeSource";
export type { ForgeTarget } from "./generated/ForgeTarget";
export type { ForgeTokenKind } from "./generated/ForgeTokenKind";
export type { ForgeUser } from "./generated/ForgeUser";
export type { HostJob } from "./generated/HostJob";
export type { HostJobKind } from "./generated/HostJobKind";
export type { HostJobState } from "./generated/HostJobState";
export type { HostJobStep } from "./generated/HostJobStep";
export type { HostJobStepState } from "./generated/HostJobStepState";
export type { Hunk } from "./generated/Hunk";
export type { IndexState } from "./generated/IndexState";
export type { KnownConcept } from "./generated/KnownConcept";
export type { LanguageLevel } from "./generated/LanguageLevel";
export type { LanguageSetting } from "./generated/LanguageSetting";
export type { LeftOutFile } from "./generated/LeftOutFile";
export type { LinkedRepository } from "./generated/LinkedRepository";
export type { MemberOrigin } from "./generated/MemberOrigin";
export type { MemberStatus } from "./generated/MemberStatus";
export type { NoteDraft } from "./generated/NoteDraft";
export type { NotePlacement } from "./generated/NotePlacement";
export type { NoteTextState } from "./generated/NoteTextState";
export type { PendingReview } from "./generated/PendingReview";
export type { Pin } from "./generated/Pin";
export type { PreviewStatus } from "./generated/PreviewStatus";
export type { PullRequestGroup } from "./generated/PullRequestGroup";
export type { PullRequestState } from "./generated/PullRequestState";
export type { RefEntry } from "./generated/RefEntry";
export type { RelocationConcern } from "./generated/RelocationConcern";
export type { RepositoryForge } from "./generated/RepositoryForge";
export type { RepositoryFreshness } from "./generated/RepositoryFreshness";
export type { RepositorySuggestion } from "./generated/RepositorySuggestion";
export type { RequestBudget } from "./generated/RequestBudget";
export type { ReviewDraft } from "./generated/ReviewDraft";
export type { Reviewer } from "./generated/Reviewer";
export type { ReviewState } from "./generated/ReviewState";
export type { ReviewVerdict } from "./generated/ReviewVerdict";
export type { RunActivity } from "./generated/RunActivity";
export type { RunBranchCommand } from "./generated/RunBranchCommand";
export type { RunChanges } from "./generated/RunChanges";
export type { RunCollection } from "./generated/RunCollection";
export type { RunControllerStatus } from "./generated/RunControllerStatus";
export type { RunDiffRequest } from "./generated/RunDiffRequest";
export type { RunEvent } from "./generated/RunEvent";
export type { RunEventBody } from "./generated/RunEventBody";
export type { RunEventPage } from "./generated/RunEventPage";
export type { RunFileDiff } from "./generated/RunFileDiff";
export type { RunOutcome } from "./generated/RunOutcome";
export type { RunPermissionRequest } from "./generated/RunPermissionRequest";
export type { RunPermissions } from "./generated/RunPermissions";
export type { RunPhase } from "./generated/RunPhase";
export type { RunPlanEntry } from "./generated/RunPlanEntry";
export type { RunPreview } from "./generated/RunPreview";
export type { RunStartPreview } from "./generated/RunStartPreview";
export type { RunStartStep } from "./generated/RunStartStep";
export type { SearchGroup } from "./generated/SearchGroup";
export type { SearchHit } from "./generated/SearchHit";
export type { SearchKind } from "./generated/SearchKind";
export type { StartExplanationRequest } from "./generated/StartExplanationRequest";
export type { StartRunRequest } from "./generated/StartRunRequest";
export type { SuggestedMove } from "./generated/SuggestedMove";
export type { TaskNote } from "./generated/TaskNote";
export type { TaskStatus } from "./generated/TaskStatus";
export type { TextPart } from "./generated/TextPart";
export type { Thread } from "./generated/Thread";
export type { ThreadAnchor } from "./generated/ThreadAnchor";
export type { TourStop } from "./generated/TourStop";
export type { UnresolvedLink } from "./generated/UnresolvedLink";
export type { VaultInfo } from "./generated/VaultInfo";
export type { WorkspaceMember } from "./generated/WorkspaceMember";
export type { WorkspacePreviewEntry } from "./generated/WorkspacePreviewEntry";
export type {
  ActivitySettings,
  AgentAccessStatus,
  AppError,
  AppSnapshot,
  Cell,
  CellValue,
  ChangesResult,
  ColumnKind,
  CommentRequest,
  CommitDetail,
  CommitPage,
  Conversation,
  CreateNoteRequest,
  CreateWorkspaceRequest,
  CredentialOwner,
  CredentialPending,
  CredentialState,
  CredentialTest,
  DbAccess,
  DbColumn,
  DbConnection,
  DbEnvironment,
  DbExportResult,
  DbFailure,
  DbFailureReason,
  DbForeignKey,
  DbIndex,
  DbKind,
  DbRelation,
  DbSchema,
  DbSchemaGroup,
  DbTestResult,
  DbTls,
  DbUrlFields,
  DiffOptions,
  DiffResult,
  DiffSelector,
  DockerContainer,
  DockerContainers,
  ExplainMode,
  ExportFormat,
  ExportRequest,
  ExportResult,
  FetchResult,
  FolderListing,
  ForgeAccountSlot,
  ForgeAccountTestResult,
  ForgeKind,
  HealthEvent,
  HealthPoint,
  HealthSample,
  HealthSession,
  HealthSnapshot,
  HealthStatement,
  HealthTable,
  HistoryEntry,
  IndexStatus,
  ListCommitsRequest,
  ListPullRequestsRequest,
  MachineSample,
  MergeMethod,
  MergeOptions,
  MergeOutcome,
  MergeRequest,
  NoteChangedEvent,
  NoteContent,
  NoteContext,
  NoteLists,
  NoteMissingEvent,
  NoteRevision,
  NoteSummary,
  ParamValue,
  PinEntityType,
  PlanNode,
  PullRequest,
  PullRequestChangedEvent,
  PullRequestChecks,
  PullRequestDiff,
  PullRequestDiffRequest,
  PullRequestFiles,
  PullRequestList,
  QueryTab,
  RefsResult,
  RelationKind,
  RelocateRepositoryRequest,
  RelocationOutcome,
  RenameNoteRequest,
  RenamePreview,
  RenameResult,
  ReplyRequest,
  RepositoryNotes,
  RepositorySummary,
  RepositoryTab,
  ResolvedLink,
  ResolveThreadRequest,
  RestorePreview,
  RestoreRequest,
  RestoreResult,
  ResultColumn,
  ReviewCount,
  ReviewDrafts,
  RunMode,
  RunStatementRequest,
  RunsOn,
  SaveDbConnectionRequest,
  SavedQuery,
  SaveForgeAccountOutcome,
  SaveForgeAccountRequest,
  SaveNoteRequest,
  SaveNoteResult,
  SaveQueryRequest,
  SaveReviewDraftRequest,
  SearchRequest,
  SearchResults,
  SecretEntry,
  SecretSource,
  SecretsOverview,
  SetRepositoryForgeRequest,
  Settings,
  StatementResult,
  StatementRun,
  SubmitReviewRequest,
  Task,
  TaskChangedEvent,
  TaskFields,
  TaskFilter,
  TeamPulse,
  TodayView,
  TransactionInfo,
  TrashedNote,
  UpdateTaskRequest,
  UpdateWorkspaceMembershipRequest,
  VaultState,
  Workspace,
  WorkspaceActivity,
  WorkspacePreview,
  WorkspaceRescan,
  WriteOutcome,
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
  /** Settings → Agent access: the mode, connected agents, and the helper's path. */
  getAgentAccessStatus: () =>
    invoke<AgentAccessStatus>("get_agent_access_status"),

  // v0.3: pull request accounts
  /** Settings → Accounts: one entry per provider. */
  listForgeAccounts: () => invoke<ForgeAccountSlot[]>("list_forge_accounts"),
  /** Checks the token with one request before saving anything. */
  saveForgeAccount: (request: SaveForgeAccountRequest) =>
    invoke<SaveForgeAccountOutcome>("save_forge_account", { request }),
  /** Also deletes the token from the Keychain. */
  removeForgeAccount: (kind: ForgeKind) =>
    invoke<ForgeAccountSlot[]>("remove_forge_account", { kind }),
  /** Test on the account form: checks the form's token without saving it. */
  testForgeAccount: (request: SaveForgeAccountRequest) =>
    invoke<ForgeAccountTestResult>("test_forge_account", { request }),

  // v0.4.x: where secrets come from
  /** Settings → Secrets. Reads no secret and runs no program. */
  listSecrets: () => invoke<SecretsOverview>("list_secrets"),
  /** Allow This Source, for the revision that was shown. */
  approveSecretSource: (owner: CredentialOwner, revision: number) =>
    invoke<SecretsOverview>("approve_secret_source", { owner, revision }),
  /** Forget the secret kept for this run, so it is read or asked for again. */
  refreshCredential: (owner: CredentialOwner) =>
    invoke<void>("refresh_credential", { owner }),
  /** Retry deleting a Keychain item after a cleanup or removal that did not finish. */
  retryCredentialCleanup: (owner: CredentialOwner) =>
    invoke<SecretsOverview>("retry_credential_cleanup", { owner }),
  /** The full path of a program, by name or path. */
  findSecretProgram: (name: string) =>
    invoke<string>("find_secret_program", { name }),

  // v0.5: agent runs
  /** Settings → Agents. */
  getAgentSettings: () => invoke<AgentSettings>("get_agent_settings"),
  /** One profile: everything but the token or key. */
  saveAgentSettings: (request: SaveAgentSettingsRequest) =>
    invoke<AgentSettings>("save_agent_settings", { request }),
  /** Pay with: the token or key and where it comes from. */
  saveAgentCredential: (request: SaveAgentCredentialRequest) =>
    invoke<AgentSettings>("save_agent_credential", { request }),
  /** A profile's token or key; also deletes one kept in the Keychain. */
  removeAgentCredential: (id: string, expectedVersion: number) =>
    invoke<AgentSettings>("remove_agent_credential", { id, expectedVersion }),
  /** This Mac's Docker engine; another engine forgets the image. */
  chooseAgentEngine: (socket: string | null) =>
    invoke<AgentSettings>("choose_agent_engine", { socket }),
  /** Confirm a setup restored from a backup, at the revision shown. */
  approveAgentSettings: (id: string, revision: number) =>
    invoke<AgentSettings>("approve_agent_settings", { id, revision }),
  /** This Mac's Docker engines, each asked what it is. */
  listAgentEngines: () => invoke<AgentEngine[]>("list_agent_engines"),
  /** The Dockerfile Brainiac builds. */
  agentDockerfile: () => invoke<string>("agent_dockerfile"),
  /** Builds the image on the chosen engine; takes minutes. */
  buildAgentImage: () => invoke<AgentSettings>("build_agent_image"),
  /** New run, Start from: the commit a run would get. */
  previewRunStart: (repositoryId: string, start: string) =>
    invoke<RunStartPreview>("preview_run_start", { repositoryId, start }),
  /** Runs, newest first, and whether the run controller is running. */
  listAgentRuns: () => invoke<AgentRunList>("list_agent_runs"),
  getAgentRun: (id: string) => invoke<AgentRun>("get_agent_run", { id }),
  /** New run, Start run. */
  startAgentRun: (request: StartRunRequest) =>
    invoke<AgentRun>("start_agent_run", { request }),
  /** The conversation after a sequence, from the mirrored journal. */
  listRunEvents: (id: string, after: number) =>
    invoke<RunEventPage>("list_run_events", { id, after }),
  /** An agent's reply as sanitized HTML. The journal keeps the text. */
  renderMarkdown: (text: string) => invoke<string>("render_markdown", { text }),
  sendRunPrompt: (id: string, text: string) =>
    invoke<AgentRun>("send_run_prompt", { id, text }),
  answerRunPermission: (id: string, permissionId: string, allow: boolean) =>
    invoke<AgentRun>("answer_run_permission", { id, permissionId, allow }),
  cancelAgentRun: (id: string) => invoke<AgentRun>("cancel_agent_run", { id }),
  finishAgentRun: (id: string) => invoke<AgentRun>("finish_agent_run", { id }),
  /** Collect work, Retry collection, or Choose files to add… with `include`. */
  collectAgentRun: (id: string, include: string[]) =>
    invoke<AgentRun>("collect_agent_run", { id, include }),
  acceptRunSnapshot: (id: string) =>
    invoke<AgentRun>("accept_run_snapshot", { id }),
  discardAgentRun: (id: string) =>
    invoke<AgentRun>("discard_agent_run", { id }),
  retryRunCleanup: (id: string) =>
    invoke<AgentRun>("retry_run_cleanup", { id }),
  deleteAgentRun: (id: string) => invoke<void>("delete_agent_run", { id }),
  getRunChanges: (id: string) => invoke<RunChanges>("get_run_changes", { id }),
  getRunDiff: (request: RunDiffRequest) =>
    invoke<DiffResult>("get_run_diff", { request }),
  /** Changes so far: the live run's last preview. */
  getRunPreview: (id: string) => invoke<RunPreview>("get_run_preview", { id }),
  /** Changes so far, Refresh: a new preview, now. */
  refreshRunPreview: (id: string) =>
    invoke<RunPreview>("refresh_run_preview", { id }),
  /** Copy branch command: a command for the user to run in their repository. */
  getRunBranchCommand: (id: string) =>
    invoke<RunBranchCommand>("get_run_branch_command", { id }),
  /** The whole snapshot as a patch text. */
  copyRunPatch: (id: string) => invoke<string>("copy_run_patch", { id }),
  saveRunPatch: (id: string, path: string) =>
    invoke<void>("save_run_patch", { id, path }),
  /** This Mac's Test of one profile: a short run, a prompt, a cancel, a collection. */
  testAgentSetup: (profileId: string) =>
    invoke<AgentTestResult>("test_agent_setup", { profileId }),
  getRunControllerStatus: () =>
    invoke<RunControllerStatus>("get_run_controller_status"),

  /** Changes against main: a branch's files since it left the default branch. */
  getBranchComparison: (repositoryId: string, branch: string) =>
    invoke<BranchComparison>("get_branch_comparison", {
      repositoryId,
      branch,
    }),
  /** What the Explain dialog shows for a subject. */
  getExplainDialog: (repositoryId: string, subject: ExplainSubject) =>
    invoke<ExplainDialog>("get_explain_dialog", { repositoryId, subject }),
  /** The once-per-repository question, for the repository or its workspace. */
  answerExplain: (request: AnswerExplainRequest) =>
    invoke<void>("answer_explain", { request }),
  forgetExplainAnswer: (
    scope: ExplainAnswerScope,
    scopeId: string,
    provider: AgentProvider,
  ) => invoke<void>("forget_explain_answer", { scope, scopeId, provider }),
  startExplanation: (request: StartExplanationRequest) =>
    invoke<ExplanationRecord>("start_explanation", { request }),
  cancelExplanation: (id: string) =>
    invoke<ExplanationRecord>("cancel_explanation", { id }),
  getExplanation: (id: string) =>
    invoke<ExplanationRecord>("get_explanation", { id }),
  /** Every explanation of a subject, newest first. */
  listSubjectExplanations: (repositoryId: string, subject: ExplainSubject) =>
    invoke<ExplanationRecord[]>("list_subject_explanations", {
      repositoryId,
      subject,
    }),
  deleteExplanation: (id: string) => invoke<void>("delete_explanation", { id }),
  deleteAllExplanations: () => invoke<number>("delete_all_explanations"),
  /** Not a problem on a disagreement, or Show to bring it back. */
  setDisagreementHidden: (id: string, index: number, hidden: boolean) =>
    invoke<ExplanationRecord>("set_disagreement_hidden", {
      id,
      index,
      hidden,
    }),
  /** I know this; the concept's ID, for Undo. Given the explanation it is
   * in, the concept keeps its description and where it was learned. */
  learnConcept: (
    repositoryId: string,
    kind: ConceptKind,
    name: string,
    explanationId: string | null = null,
  ) =>
    invoke<string>("learn_concept", {
      repositoryId,
      kind,
      name,
      explanationId,
    }),
  forgetConcept: (id: string) => invoke<void>("forget_concept", { id }),
  mergeConcept: (from: string, into: string) =>
    invoke<void>("merge_concept", { from, into }),
  /** Where a branch explanation's notes are on the branch now. */
  placeExplanation: (id: string) =>
    invoke<ExplanationPlacement>("place_explanation", { id }),
  saveExplanationAsNote: (id: string) =>
    invoke<NoteSummary>("save_explanation_as_note", { id }),
  getExplanationSettings: () =>
    invoke<ExplanationSettingsView>("get_explanation_settings"),
  saveExplanationSettings: (settings: ExplanationSettings) =>
    invoke<void>("save_explanation_settings", { settings }),
  /** The host key, before anything is saved. */
  previewAgentHost: (host: string, port: number) =>
    invoke<AgentHostPreview>("preview_agent_host", { host, port }),
  /** Approve that fingerprint and save the host. */
  approveAgentHost: (request: ApproveAgentHostRequest) =>
    invoke<AgentHost>("approve_agent_host", { request }),
  removeAgentHost: (id: string) => invoke<void>("remove_agent_host", { id }),
  /**
   * Install, Upgrade, Build image, Test, or Add host's setup, in the
   * background. Test is of one profile; setup tests each ready one.
   */
  startAgentHostJob: (id: string, kind: HostJobKind, profileId?: string) =>
    invoke<HostJob>("start_agent_host_job", {
      id,
      kind,
      profileId: profileId ?? null,
    }),
  /** Until the install step begins. */
  cancelAgentHostJob: (id: string) =>
    invoke<HostJob>("cancel_agent_host_job", { id }),
  /** Each host's last job, running or ended. */
  listAgentHostJobs: () => invoke<HostJob[]>("list_agent_host_jobs"),
  /** The whole output of a host's last job. */
  getAgentHostJobLog: (id: string) =>
    invoke<string>("get_agent_host_job_log", { id }),

  /** Where a repository's pull requests come from; `forge: null` goes back to `origin`. */
  setRepositoryForge: (request: SetRepositoryForgeRequest) =>
    invoke<RepositorySummary>("set_repository_forge", { request }),
  /** A workspace's pull request switch. */
  updateWorkspacePullRequests: (workspaceId: string, enabled: boolean) =>
    invoke<Workspace>("update_workspace_pull_requests", {
      workspaceId,
      enabled,
    }),
  /** The workspace's or a repository's Pull requests tab. */
  listPullRequests: (request: ListPullRequestsRequest) =>
    invoke<PullRequestList>("list_pull_requests", { request }),
  /** One pull request, cached when younger than `maxAgeSeconds`. */
  getPullRequest: (reference: string, maxAgeSeconds: number) =>
    invoke<PullRequest>("get_pull_request", { reference, maxAgeSeconds }),
  /** The files changed, or only those since a commit of the pull request (local Git only). */
  listPullRequestFiles: (reference: string, since: string | null = null) =>
    invoke<PullRequestFiles>("list_pull_request_files", {
      reference,
      since,
    }),
  /** The threads and comments, cached when younger than `maxAgeSeconds`. */
  getPullRequestConversation: (reference: string, maxAgeSeconds: number) =>
    invoke<Conversation>("get_pull_request_conversation", {
      reference,
      maxAgeSeconds,
    }),
  /** One file's diff: local Git when both commits are on the Mac, else the provider's. */
  getPullRequestDiff: (request: PullRequestDiffRequest) =>
    invoke<PullRequestDiff>("get_pull_request_diff", { request }),
  getPullRequestChecks: (reference: string, maxAgeSeconds: number) =>
    invoke<PullRequestChecks>("get_pull_request_checks", {
      reference,
      maxAgeSeconds,
    }),
  /** The local checkout of a pull request's repository. */
  getPullRequestRepository: (reference: string) =>
    invoke<RepositorySummary>("get_pull_request_repository", { reference }),
  // Reviewing (SPEC.md, Reviewing): drafts stay on the Mac; the rest is
  // written to the provider on this explicit action.
  listReviewDrafts: (reference: string) =>
    invoke<ReviewDrafts>("list_review_drafts", { reference }),
  saveReviewDraft: (request: SaveReviewDraftRequest) =>
    invoke<ReviewDrafts>("save_review_draft", { request }),
  deleteReviewDraft: (reference: string, id: string) =>
    invoke<ReviewDrafts>("delete_review_draft", { reference, id }),
  /** The drafts go on the head now: the user looked at the new commits. */
  moveReviewDrafts: (reference: string, headSha: string) =>
    invoke<ReviewDrafts>("move_review_drafts", { reference, headSha }),
  commentOnPullRequest: (request: CommentRequest) =>
    invoke<WriteOutcome>("comment_on_pull_request", { request }),
  replyToThread: (request: ReplyRequest) =>
    invoke<WriteOutcome>("reply_to_thread", { request }),
  resolveThread: (request: ResolveThreadRequest) =>
    invoke<WriteOutcome>("resolve_thread", { request }),
  /** Finish Review: the drafts, a summary, and a verdict, for the head the user looked at. */
  submitReview: (request: SubmitReviewRequest) =>
    invoke<WriteOutcome>("submit_review", { request }),
  // Merging (SPEC.md, Merging): confirmed, for the head the user looked at.
  getMergeOptions: (reference: string) =>
    invoke<MergeOptions>("get_merge_options", { reference }),
  mergePullRequest: (request: MergeRequest) =>
    invoke<MergeOutcome>("merge_pull_request", { request }),
  /** Each workspace's count of pull requests waiting on your review, from the cache. */
  getReviewCounts: () => invoke<ReviewCount[]>("get_review_counts"),

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

  // v0.4: databases
  listDbConnections: () => invoke<DbConnection[]>("list_db_connections"),
  saveDbConnection: (request: SaveDbConnectionRequest) =>
    invoke<DbConnection>("save_db_connection", { request }),
  deleteDbConnection: (id: string, expectedVersion: number) =>
    invoke<void>("delete_db_connection", { id, expectedVersion }),
  testDbConnection: (request: SaveDbConnectionRequest) =>
    invoke<DbTestResult>("test_db_connection", { request }),
  parseDbUrl: (url: string) => invoke<DbUrlFields>("parse_db_url", { url }),
  unlockDbConnection: (id: string, password: string, expectedVersion: number) =>
    invoke<DbConnection>("unlock_db_connection", {
      id,
      password,
      expectedVersion,
    }),
  linkDbConnection: (
    connectionId: string,
    repositoryId: string,
    linked: boolean,
  ) =>
    invoke<DbConnection>("link_db_connection", {
      connectionId,
      repositoryId,
      linked,
    }),
  getDbSchema: (connectionId: string, refresh: boolean) =>
    invoke<DbSchema>("get_db_schema", { connectionId, refresh }),
  statementParameters: (request: RunStatementRequest) =>
    invoke<string[]>("statement_parameters", { request }),
  runStatement: (request: RunStatementRequest) =>
    invoke<StatementRun[]>("run_statement", { request }),
  cancelStatement: (tabId: string) =>
    invoke<void>("cancel_statement", { tabId }),
  closeDbSession: (tabId: string) =>
    invoke<void>("close_db_session", { tabId }),
  endTransaction: (tabId: string, commit: boolean) =>
    invoke<void>("end_transaction", { tabId, commit }),
  exportResult: (request: ExportRequest) =>
    invoke<DbExportResult>("export_result", { request }),
  openDbTransactions: () => invoke<string[]>("open_db_transactions"),
  listSavedQueries: () => invoke<SavedQuery[]>("list_saved_queries"),
  saveQuery: (request: SaveQueryRequest) =>
    invoke<SavedQuery>("save_query", { request }),
  deleteQuery: (id: string, expectedVersion: number) =>
    invoke<void>("delete_query", { id, expectedVersion }),
  rememberQueryParameters: (id: string, values: ParamValue[]) =>
    invoke<SavedQuery>("remember_query_parameters", { id, values }),
  queryHistory: (
    connectionId: string,
    search: string,
    offset: number,
    limit: number,
  ) =>
    invoke<HistoryEntry[]>("query_history", {
      connectionId,
      search,
      offset,
      limit,
    }),
  clearQueryHistory: (connectionId: string) =>
    invoke<void>("clear_query_history", { connectionId }),
  listQueryTabs: () => invoke<QueryTab[]>("list_query_tabs"),
  saveQueryTabs: (tabs: QueryTab[]) =>
    invoke<void>("save_query_tabs", { tabs }),
  startDbHealth: (connectionId: string) =>
    invoke<HealthSnapshot>("start_db_health", { connectionId }),
  stopDbHealth: (connectionId: string) =>
    invoke<void>("stop_db_health", { connectionId }),
  signalDbBackend: (connectionId: string, pid: number, terminate: boolean) =>
    invoke<boolean>("signal_db_backend", { connectionId, pid, terminate }),
  listDockerContainers: (socket: string | null) =>
    invoke<DockerContainers>("list_docker_containers", { socket }),
};

export function onDbHealthSample(
  handler: (e: HealthEvent) => void,
): Promise<UnlistenFn> {
  return listen<HealthEvent>("db_health_sample", (ev) => handler(ev.payload));
}

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

export function onPullRequestChanged(
  handler: (e: PullRequestChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<PullRequestChangedEvent>("pr_changed", (ev) =>
    handler(ev.payload),
  );
}

export function onAgentHostJob(
  handler: (job: HostJob) => void,
): Promise<UnlistenFn> {
  return listen<HostJob>("agent_host_job", (ev) => handler(ev.payload));
}

export function onAgentRunChanged(
  handler: (e: AgentRunChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<AgentRunChangedEvent>("agent_run_changed", (ev) =>
    handler(ev.payload),
  );
}

export function onExplanationChanged(
  handler: (e: ExplanationChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<ExplanationChangedEvent>("explanation_changed", (ev) =>
    handler(ev.payload),
  );
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
