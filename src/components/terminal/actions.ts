/**
 * What the terminal's controls do, shared by the view, the dock, the menu,
 * and the shortcuts (SPEC.md, section 15).
 */
import { ask } from "@tauri-apps/plugin-dialog";
import { ipc } from "../../lib/ipc";
import {
  closeQuestion,
  nextActive,
  type TerminalTarget,
} from "../../lib/terminal";
import { dispose, ended } from "./sessions";
import { terminalStore } from "./store";

/** The window event that asks App to open a tab from anywhere. */
export const OPEN_TERMINAL_EVENT = "brainiac:open-terminal";

/** What a New terminal control opens a tab on. */
export type TerminalRequest =
  | { kind: "repository"; id: string; workspaceId?: string }
  | { kind: "workspace"; id: string }
  | { kind: "folder"; target: TerminalTarget };

/** Open a tab from a view that has no callback for it (a menu, a row). */
export function requestTerminal(request: TerminalRequest) {
  window.dispatchEvent(
    new CustomEvent<TerminalRequest>(OPEN_TERMINAL_EVENT, { detail: request }),
  );
}

/** Add a tab and show it; its program starts when its pane first shows. */
export function openTab(target: TerminalTarget): string {
  return terminalStore.add(target);
}

/**
 * Close a tab, ending its program. When a program runs in front of the
 * tab's own (`pnpm dev` in a shell), ask first.
 */
export async function closeTab(key: string): Promise<boolean> {
  const tab = terminalStore.tab(key);
  if (!tab) return true;
  if (tab.id) {
    const front = await ipc.terminalForeground(tab.id).catch(() => null);
    if (front) {
      const sure = await ask(closeQuestion(tab.title, front), {
        title: `Close “${tab.title}”?`,
        kind: "warning",
        okLabel: "Close and end it",
      });
      if (!sure) return false;
    }
  }
  forget(key);
  return true;
}

function forget(key: string) {
  const { tabs, active } = terminalStore.get();
  dispose(key);
  terminalStore.remove(key, nextActive(tabs, key, active));
}

/**
 * A tab's program ended. A shell left with `exit` (code 0) closes its tab,
 * as in a terminal app; any other end keeps the tab, with what it printed.
 */
export function programEnded(id: string, code: number | null) {
  const tab = terminalStore.byId(id);
  if (!tab) return;
  if (code === 0) {
    forget(tab.key);
    return;
  }
  ended(tab.key, code);
  terminalStore.update(tab.key, { id: null, ended: { code } });
}
