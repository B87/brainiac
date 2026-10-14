/**
 * The terminal's rules that need no window (SPEC.md, section 15): where a
 * new tab opens, which keys Brainiac takes from a focused tab, and the tabs'
 * list. The panes and xterm.js are in `components/terminal/`.
 */
import type { AppSnapshot, RepositorySummary, Workspace } from "./ipc";

/** A tab as the window knows it. Its program is the backend's (`TerminalTab`). */
export type TerminalTabState = {
  /** Brainiac's own key, set before the program starts. */
  key: string;
  /** The backend's ID once the program started. */
  id: string | null;
  /** The repository's or workspace's name, or the folder's. */
  title: string;
  /** "zsh", "herdr": the program, once started. */
  program: string | null;
  /** Empty for the home folder. */
  cwd: string;
  workspaceId?: string;
  /** Set when the program ended with a code other than 0, or failed to start. */
  ended: { code: number | null } | null;
  error: string | null;
};

/** Where a new tab opens, and what it is called. */
export type TerminalTarget = {
  cwd: string;
  title: string;
  workspaceId?: string;
};

/** A workspace's members a tab could open in, when it has no one folder. */
export type FolderChoice = {
  workspace: Workspace;
  members: { title: string; cwd: string }[];
};

/** The last part of a path: its folder's name. */
export function folderName(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export function repositoryTarget(
  repo: RepositorySummary,
  workspaceId?: string,
): TerminalTarget {
  return { cwd: repo.canonical_root, title: repo.name, workspaceId };
}

/**
 * Where a terminal opened on a workspace goes: its root repository, the
 * folder chosen for discovery, or for a manual workspace the member chosen
 * before (`remembered`); otherwise the members to choose from.
 */
export function workspaceTarget(
  workspace: Workspace,
  snapshot: Pick<AppSnapshot, "repositories">,
  remembered: string | null,
): TerminalTarget | FolderChoice {
  const root = workspace.root_repository_id
    ? snapshot.repositories.find((r) => r.id === workspace.root_repository_id)
    : undefined;
  if (root)
    return {
      cwd: root.canonical_root,
      title: workspace.name,
      workspaceId: workspace.id,
    };
  if (workspace.discovery_mode === "discovered" && workspace.discovery_root) {
    const cwd = workspace.discovery_path
      ? `${workspace.discovery_root.replace(/\/$/, "")}/${workspace.discovery_path}`
      : workspace.discovery_root;
    return { cwd, title: workspace.name, workspaceId: workspace.id };
  }
  const members = workspace.members
    .filter((m) => m.status !== "missing")
    .map((m) => ({ title: m.display_name, cwd: m.canonical_path }));
  const chosen = members.find((m) => m.cwd === remembered);
  if (chosen) return { ...chosen, workspaceId: workspace.id };
  if (members.length === 1) return { ...members[0], workspaceId: workspace.id };
  return { workspace, members };
}

export function isChoice(
  target: TerminalTarget | FolderChoice,
): target is FolderChoice {
  return "members" in target;
}

/** Whether this is a Mac, where Brainiac's shortcuts use ⌘ and leave Ctrl to the tab. */
export const IS_MAC =
  typeof navigator !== "undefined" &&
  /Mac/i.test(navigator.platform || navigator.userAgent);

/**
 * What a key pressed in a focused tab is for: `leave` moves focus out of the
 * terminal, `app` is one of Brainiac's shortcuts (⌘ keys on a Mac), and
 * `null` goes to the program. On Linux, every Ctrl key goes to the tab
 * (Ctrl+J and Ctrl+T too, which shells use) except Ctrl+Esc.
 */
export function tabKey(
  e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">,
  mac: boolean,
): "leave" | "app" | null {
  if (e.key === "Escape" && (mac ? e.metaKey : e.ctrlKey) && !e.altKey)
    return "leave";
  if (mac && e.metaKey) return "app";
  return null;
}

/** `Cmd+1` … `Cmd+9` in a focused tab on macOS: the tab to show, from 1. */
export function tabNumber(
  e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">,
  mac: boolean,
): number | null {
  if (!mac || !e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return null;
  return /^[1-9]$/.test(e.key) ? Number(e.key) : null;
}

/** The tab after `key` is closed: the next one, else the one before. */
export function nextActive(
  tabs: readonly TerminalTabState[],
  closing: string,
  active: string | null,
): string | null {
  if (active !== closing) return active;
  const i = tabs.findIndex((t) => t.key === closing);
  const rest = tabs.filter((t) => t.key !== closing);
  return rest[Math.min(Math.max(i, 0), rest.length - 1)]?.key ?? null;
}

/** "zsh is still running" or "pnpm is still running in it", for the close dialog. */
export function closeQuestion(title: string, program: string): string {
  return `${program} is still running in “${title}”. Closing the tab ends it.`;
}

/** The quit dialog's question, naming the busy programs. */
export function quitQuestion(programs: readonly string[]): string {
  const n = programs.length;
  const names = [...new Set(programs)].join(", ");
  return n === 1
    ? `A terminal tab is still running ${names}. Quitting Brainiac ends it.`
    : `${n} terminal tabs are still running programs (${names}). Quitting Brainiac ends them.`;
}
