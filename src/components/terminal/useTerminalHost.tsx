/**
 * The terminal in the window (SPEC.md, section 15): the dock beside any view,
 * the Terminal view, ⌘T, ⌘J, and ⌘Esc, New terminal from anywhere, the quit
 * dialog's count of busy tabs, and the tabs whose program ended. App renders
 * what this returns; the tabs themselves live in `store.ts`.
 */
import { ask } from "@tauri-apps/plugin-dialog";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { addCloseGuard } from "../../lib/closeGuard";
import {
  type AppSnapshot,
  ipc,
  onTerminalExited,
  subscribe,
} from "../../lib/ipc";
import { useKeys } from "../../lib/keys";
import { usePref } from "../../lib/prefs";
import {
  type FolderChoice,
  isChoice,
  quitQuestion,
  repositoryTarget,
  type TerminalTarget,
  workspaceTarget,
} from "../../lib/terminal";
import type { View } from "../Sidebar";
import {
  OPEN_TERMINAL_EVENT,
  openTab,
  programEnded,
  type TerminalRequest,
} from "./actions";
import ChooseFolderDialog from "./ChooseFolderDialog";
import { focus, hasFocus, setLeaveHandler } from "./sessions";
import { type DockSide, terminalStore, useDockSide } from "./store";
import { TerminalDock, TerminalView } from "./Terminal";

export function useTerminalHost({
  view,
  snapshot,
  showView,
}: {
  view: View;
  snapshot: AppSnapshot | null;
  showView: (view: View) => void;
}): {
  /** Beside the view, when the dock is open and the Terminal view is not shown. */
  dock: ReactNode;
  dockSide: DockSide;
  /** The Terminal view, for `view.kind === "terminal"`. */
  terminalView: ReactNode;
  /** The manual workspace's folder question. */
  dialog: ReactNode;
  /** A native menu item; true when it was the terminal's. */
  onMenu: (id: string) => boolean;
} {
  const [dockOpen, setDockOpen] = usePref("terminal.dockOpen", false);
  const dockSide = useDockSide();
  const [folders, setFolders] = usePref<Record<string, string>>(
    "terminal.workspaceFolders",
    {},
  );
  const [choice, setChoice] = useState<FolderChoice | null>(null);

  // The repository last selected, where ⌘T opens a tab, and the view to go
  // back to from the Terminal view's Dock.
  const lastRepository = useRef<{ id: string; workspaceId?: string } | null>(
    null,
  );
  const before = useRef<View>({ kind: "all" });
  useEffect(() => {
    if (view.kind === "repository")
      lastRepository.current = { id: view.id, workspaceId: view.workspaceId };
    if (view.kind !== "terminal" && view.kind !== "settings")
      before.current = view;
  }, [view]);

  const here = useCallback((): TerminalTarget => {
    const last = lastRepository.current;
    const repo = last
      ? snapshot?.repositories.find((r) => r.id === last.id)
      : undefined;
    return repo
      ? repositoryTarget(repo, last?.workspaceId)
      : { cwd: "", title: "Home" };
  }, [snapshot]);
  const where = (() => {
    const target = here();
    return target.cwd ? target.title : "your home folder";
  })();

  const showing = view.kind === "terminal";
  const open = useCallback(
    (target: TerminalTarget) => {
      openTab(target);
      if (!showing) setDockOpen(true);
    },
    [showing, setDockOpen],
  );
  const newHere = useCallback(() => open(here()), [open, here]);

  // ⌘T: the Terminal view; in it, or from a focused dock tab, a new tab.
  // A menu accelerator and the key listener can both see one keypress.
  const handledAt = useRef<Record<string, number>>({});
  const once = (name: string, f: () => void) => {
    const now = performance.now();
    if (now - (handledAt.current[name] ?? -1000) < 80) return;
    handledAt.current[name] = now;
    f();
  };
  const commandT = () =>
    once("t", () => {
      if (showing || (dockOpen && hasFocus())) {
        newHere();
        return;
      }
      showView({ kind: "terminal" });
      if (terminalStore.get().tabs.length === 0) openTab(here());
    });
  const toggleDock = () =>
    once("j", () => {
      if (showing) {
        // The view's tabs move to the dock, under the view before it.
        showView(before.current);
        setDockOpen(true);
        return;
      }
      setDockOpen(!dockOpen);
    });
  const focusIn = () => {
    const { active } = terminalStore.get();
    if (!active) return;
    if (!showing && !dockOpen) setDockOpen(true);
    focus(active);
  };
  useKeys({
    "mod+t": commandT,
    "mod+j": toggleDock,
    "mod+Escape": focusIn,
  });

  // ⌘Esc in a tab: back to the window, the tab still showing.
  useEffect(() => {
    setLeaveHandler(() => {
      const active = document.activeElement;
      if (active instanceof HTMLElement) active.blur();
    });
  }, []);

  // New terminal on a repository, a worktree, or a workspace.
  const request = useRef<(r: TerminalRequest) => void>(() => {});
  request.current = (r: TerminalRequest) => {
    if (r.kind === "folder") return open(r.target);
    if (!snapshot) return;
    if (r.kind === "repository") {
      const repo = snapshot.repositories.find((x) => x.id === r.id);
      if (repo) open(repositoryTarget(repo, r.workspaceId));
      return;
    }
    const ws = snapshot.workspaces.find((w) => w.id === r.id);
    if (!ws) return;
    const target = workspaceTarget(ws, snapshot, folders[ws.id] ?? null);
    if (isChoice(target)) setChoice(target);
    else open(target);
  };
  useEffect(() => {
    const listener = (e: Event) =>
      request.current((e as CustomEvent<TerminalRequest>).detail);
    window.addEventListener(OPEN_TERMINAL_EVENT, listener);
    return () => window.removeEventListener(OPEN_TERMINAL_EVENT, listener);
  }, []);

  // A program ended; Option as Meta for tabs opened from now on.
  useEffect(() => {
    void ipc
      .getTerminalSettings()
      .then((v) => terminalStore.setOptionAsMeta(v.settings.option_as_meta))
      .catch(() => {});
    return subscribe(onTerminalExited((e) => programEnded(e.id, e.code)));
  }, []);

  // Quitting ends every plain shell; ask first when one runs a program.
  useEffect(
    () =>
      addCloseGuard(async () => {
        if (terminalStore.get().tabs.length === 0) return true;
        const busy = await ipc.listBusyTerminals();
        if (busy.length === 0) return true;
        return ask(quitQuestion(busy.map((b) => b.program)), {
          title: "Quit Brainiac?",
          kind: "warning",
          okLabel: "Quit",
        });
      }),
    [],
  );

  const menu = useRef<(id: string) => boolean>(() => false);
  menu.current = (id: string) => {
    if (id === "new_terminal") commandT();
    else if (id === "toggle_dock") toggleDock();
    else if (id === "show_terminal") showView({ kind: "terminal" });
    else return false;
    return true;
  };
  const onMenu = useCallback((id: string) => menu.current(id), []);

  return {
    dock:
      dockOpen && !showing ? (
        <TerminalDock
          where={where}
          side={dockSide}
          onNew={newHere}
          onMaximize={() => showView({ kind: "terminal" })}
          onHide={() => setDockOpen(false)}
        />
      ) : null,
    dockSide,
    terminalView: (
      <TerminalView
        where={where}
        onNew={newHere}
        onDock={() => {
          showView(before.current);
          setDockOpen(true);
        }}
      />
    ),
    dialog: choice ? (
      <ChooseFolderDialog
        choice={choice}
        onClose={() => setChoice(null)}
        onOpen={(target) => {
          setChoice(null);
          setFolders({ ...folders, [choice.workspace.id]: target.cwd });
          open(target);
        }}
      />
    ) : null,
    onMenu,
  };
}
