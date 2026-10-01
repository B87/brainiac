/**
 * Typed wrappers around Tauri `invoke`. Types come from `generated/`, which
 * `cargo test` exports from the Rust DTOs; do not edit those files by hand.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppError } from "./generated/AppError";
import type { AppSnapshot } from "./generated/AppSnapshot";
import type { ChangesResult } from "./generated/ChangesResult";
import type { CommitPage } from "./generated/CommitPage";
import type { DiffResult } from "./generated/DiffResult";
import type { DiffSelector } from "./generated/DiffSelector";
import type { ListCommitsRequest } from "./generated/ListCommitsRequest";
import type { MenuEvent } from "./generated/MenuEvent";
import type { RepositoryChangedEvent } from "./generated/RepositoryChangedEvent";
import type { RepositorySummary } from "./generated/RepositorySummary";
import type { RepositoryTab } from "./generated/RepositoryTab";

export type { ChangeEntry } from "./generated/ChangeEntry";
export type { CommitSummary } from "./generated/CommitSummary";
export type { DiffContent } from "./generated/DiffContent";
export type { Hunk } from "./generated/Hunk";
export type {
  AppError,
  AppSnapshot,
  ChangesResult,
  CommitPage,
  DiffResult,
  DiffSelector,
  RepositorySummary,
  RepositoryTab,
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
  refreshRepository: (repositoryId: string) =>
    invoke<RepositorySummary>("refresh_repository", { repositoryId }),
  openRepository: (repositoryId: string) =>
    invoke<void>("open_repository", { repositoryId }),
  setRepositoryTab: (repositoryId: string, tab: RepositoryTab) =>
    invoke<void>("set_repository_tab", { repositoryId, tab }),
  listChanges: (repositoryId: string) =>
    invoke<ChangesResult>("list_changes", { repositoryId }),
  getDiff: (repositoryId: string, selector: DiffSelector) =>
    invoke<DiffResult>("get_diff", { repositoryId, selector }),
  listCommits: (request: ListCommitsRequest) =>
    invoke<CommitPage>("list_commits", { request }),
  openInEditor: (repositoryId: string, path?: string, line?: number) =>
    invoke<void>("open_in_editor", {
      repositoryId,
      path: path ?? null,
      line: line ?? null,
    }),
  revealInFinder: (repositoryId: string, path?: string) =>
    invoke<void>("reveal_in_finder", { repositoryId, path: path ?? null }),
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
