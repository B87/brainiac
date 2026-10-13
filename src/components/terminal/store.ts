/**
 * The terminal's tabs, shared by the Terminal view, the dock, and the
 * sidebar: one list, whichever shows it (SPEC.md, section 15). A small store
 * outside React, read with `useTerminals`, because a tab outlives the
 * component that shows it.
 */
import { useSyncExternalStore } from "react";
import type { TerminalTabState, TerminalTarget } from "../../lib/terminal";

export type TerminalState = {
  tabs: TerminalTabState[];
  /** The tab shown, by key. */
  active: string | null;
};

export type DockSide = "bottom" | "right";
const SIDE_KEY = "terminal.dockSide";

function readSide(): DockSide {
  try {
    return localStorage.getItem(SIDE_KEY) === '"right"' ? "right" : "bottom";
  } catch {
    return "bottom";
  }
}

let state: TerminalState = { tabs: [], active: null };
let dockSide: DockSide = readSide();
let optionAsMeta = false;
let counter = 0;
const listeners = new Set<() => void>();

function set(next: TerminalState) {
  state = next;
  for (const listener of listeners) listener();
}

export const terminalStore = {
  get: (): TerminalState => state,
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },
  tab: (key: string) => state.tabs.find((t) => t.key === key),
  byId: (id: string) => state.tabs.find((t) => t.id === id),
  /** Add a tab for `target` and show it; its program starts when a pane shows it. */
  add(target: TerminalTarget): string {
    counter += 1;
    const key = `tab-${counter}`;
    const tab: TerminalTabState = {
      key,
      id: null,
      title: target.title,
      program: null,
      cwd: target.cwd,
      workspaceId: target.workspaceId,
      ended: null,
      error: null,
    };
    // Grouped by workspace: a new tab goes after the last of its workspace.
    const tabs = [...state.tabs];
    const last = target.workspaceId
      ? tabs.map((t) => t.workspaceId).lastIndexOf(target.workspaceId)
      : -1;
    tabs.splice(last === -1 ? tabs.length : last + 1, 0, tab);
    set({ tabs, active: key });
    return key;
  },
  update(key: string, patch: Partial<TerminalTabState>) {
    set({
      ...state,
      tabs: state.tabs.map((t) => (t.key === key ? { ...t, ...patch } : t)),
    });
  },
  remove(key: string, active: string | null) {
    set({ tabs: state.tabs.filter((t) => t.key !== key), active });
  },
  setActive(key: string) {
    if (state.active !== key) set({ ...state, active: key });
  },
  dockSide: () => dockSide,
  /** Where the dock sits, remembered on this Mac (Settings → Terminal). */
  setDockSide(side: DockSide) {
    dockSide = side;
    try {
      localStorage.setItem(SIDE_KEY, JSON.stringify(side));
    } catch {
      // Preference only.
    }
    for (const listener of listeners) listener();
  },
  optionAsMeta: () => optionAsMeta,
  /** Applies to tabs opened from now on. */
  setOptionAsMeta(value: boolean) {
    optionAsMeta = value;
  },
};

export function useDockSide(): DockSide {
  return useSyncExternalStore(terminalStore.subscribe, terminalStore.dockSide);
}

export function useTerminals(): TerminalState {
  return useSyncExternalStore(terminalStore.subscribe, terminalStore.get);
}
