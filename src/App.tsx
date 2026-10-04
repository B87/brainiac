import { ask, open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import AddDialog, { type AddMode } from "./components/AddDialog";
import {
  ExportProblems,
  RestoreDialog,
  runExport,
} from "./components/BackupDialogs";
import CommandPalette from "./components/CommandPalette";
import Dashboard, { type Discovered } from "./components/Dashboard";
import DatabasesView, {
  type DatabasesRequest,
} from "./components/DatabasesView";
import NotesView from "./components/NotesView";
import PullRequestView from "./components/PullRequestView";
import RepositoryView from "./components/RepositoryView";
import SettingsPage from "./components/SettingsPage";
import Sidebar, { type View } from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import TaskEditor from "./components/TaskEditor";
import TasksView from "./components/TasksView";
import TodayView from "./components/TodayView";
import UpdateBanner from "./components/UpdateBanner";
import { listenForClose } from "./lib/closeGuard";
import { shortPath } from "./lib/format";
import {
  type AppSnapshot,
  type DbConnection,
  type ExportResult,
  errorMessage,
  ipc,
  onIndexStatusChanged,
  onMenu,
  onPullRequestChanged,
  onRepositoryChanged,
  type PinEntityType,
  type SavedQuery,
  type SuggestedMove,
  subscribe,
  type Task,
  type TaskFields,
  type VaultState,
  type Workspace,
} from "./lib/ipc";
import { useKeys } from "./lib/keys";
import { folderOf } from "./lib/notes";
import { usePref } from "./lib/prefs";
import {
  parentFolder,
  plural,
  relocatedNotice,
  relocationQuestion,
} from "./lib/repo";
import type { SettingsSection } from "./lib/settings";
import { SIDE_PANEL_KEY, SidePanelState } from "./lib/sidePanel";
import { workspaceRepositories } from "./lib/workspace";

/** The section open when Brainiac quit, reopened at launch (SPEC.md, Main window v0.2). */
const VIEW_KEY = "brainiac.view";

function readView(): View {
  try {
    const v = JSON.parse(
      localStorage.getItem(VIEW_KEY) ?? "null",
    ) as View | null;
    if (v && typeof v.kind === "string") return v;
  } catch {
    // Fall back to the first launch's view.
  }
  return { kind: "all" };
}

/** A task being edited, or a new one with its first fields. */
type EditingTask = {
  task?: Task;
  initial?: Partial<TaskFields>;
  initialNote?: { id: string; title: string } | null;
};

export default function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [view, setView] = useState<View>(readView);
  const [vault, setVault] = useState<VaultState | null>(null);
  const [livePreview, setLivePreview] = usePref(
    "brainiac.notes.livePreview",
    true,
  );
  const [contextOpen, setContextOpen] = usePref(SIDE_PANEL_KEY, true);
  const [sidebarOpen, setSidebarOpen] = usePref("brainiac.sidebar.open", true);
  /** Bumped by ⌘S; the open note saves at once. */
  const [saveTick, setSaveTick] = useState(0);
  const [editingTask, setEditingTask] = useState<EditingTask | null>(null);
  const [dialog, setDialog] = useState<"restore" | null>(null);
  const [exportProblems, setExportProblems] = useState<ExportResult | null>(
    null,
  );
  const [banner, setBanner] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [addMode, setAddMode] = useState<AddMode | null>(null);
  const [discovered, setDiscovered] = useState<Discovered | null>(null);
  /** Bumped when the selected repository changes on disk; views re-fetch on it. */
  const [changeTick, setChangeTick] = useState(0);
  /** Bumped from the "Check for Updates…" menu item. */
  const [updateTick, setUpdateTick] = useState(0);
  /** Repositories with a fetch running. */
  const [fetching, setFetching] = useState<ReadonlySet<string>>(new Set());
  /** Short-lived outcome shown in the status bar, such as "2 refs updated". */
  const [notice, setNotice] = useState<string | null>(null);
  /** Pull requests waiting on your review, per workspace, for the sidebar. */
  const [reviewCounts, setReviewCounts] = useState<ReadonlyMap<string, number>>(
    new Map(),
  );
  /** Databases stays mounted once opened, so its tabs keep their results. */
  const [dbMounted, setDbMounted] = useState(view.kind === "databases");
  const [dbRequest, setDbRequest] = useState<DatabasesRequest | null>(null);
  const [dbLists, setDbLists] = useState<{
    connections: DbConnection[];
    queries: SavedQuery[];
  }>({ connections: [], queries: [] });
  const onDbLists = useCallback(
    (connections: DbConnection[], queries: SavedQuery[]) =>
      setDbLists({ connections, queries }),
    [],
  );
  useEffect(() => {
    if (view.kind === "databases") setDbMounted(true);
  }, [view.kind]);
  // Saved queries and connections for ⌘K, before Databases was opened.
  useEffect(() => {
    void Promise.all([ipc.listDbConnections(), ipc.listSavedQueries()])
      .then(([connections, queries]) => setDbLists({ connections, queries }))
      .catch(() => {});
  }, []);
  // The window's close button runs every close guard (unsaved notes, open transactions).
  useEffect(() => listenForClose(), []);
  const viewRef = useRef(view);
  viewRef.current = view;
  const livePreviewRef = useRef(livePreview);
  livePreviewRef.current = livePreview;
  const contextOpenRef = useRef(contextOpen);
  contextOpenRef.current = contextOpen;
  const sidebarOpenRef = useRef(sidebarOpen);
  sidebarOpenRef.current = sidebarOpen;
  /** Notes' own toggle while it is shown: a narrow window keeps its own state. */
  const toggleContextRef = useRef<(() => void) | null>(null);
  /** A menu accelerator and the key listener can both see one keypress. */
  const sidePanelToggledAt = useRef(0);
  const sidebarToggledAt = useRef(0);
  const flipSidePanel = useCallback(() => {
    if (toggleContextRef.current) toggleContextRef.current();
    else setContextOpen(!contextOpenRef.current);
  }, [setContextOpen]);
  const flipSidebar = useCallback(() => {
    setSidebarOpen(!sidebarOpenRef.current);
  }, [setSidebarOpen]);
  const toggleSidePanel = useCallback(() => {
    const now = performance.now();
    if (now - sidePanelToggledAt.current < 80) return;
    sidePanelToggledAt.current = now;
    flipSidePanel();
  }, [flipSidePanel]);
  const toggleSidebar = useCallback(() => {
    const now = performance.now();
    if (now - sidebarToggledAt.current < 80) return;
    sidebarToggledAt.current = now;
    flipSidebar();
  }, [flipSidebar]);
  const toggleSidePanelRef = useRef(toggleSidePanel);
  toggleSidePanelRef.current = toggleSidePanel;
  const toggleSidebarRef = useRef(toggleSidebar);
  toggleSidebarRef.current = toggleSidebar;
  useKeys({ "mod+b": () => toggleSidebar() });

  // ⌥⌘B hides the side panel. ⌥⌘0 still does, for the notes context panel.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (
        (e.code !== "KeyB" && e.code !== "Digit0") ||
        !e.metaKey ||
        !e.altKey ||
        e.ctrlKey ||
        e.shiftKey
      )
        return;
      if (document.querySelector('[aria-modal="true"]')) return;
      e.preventDefault();
      toggleSidePanel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleSidePanel]);

  // Counted from the cache, so it is cheap; a failure only leaves the
  // sidebar's count as it was.
  const reloadReviewCounts = useCallback(async () => {
    try {
      const counts = await ipc.getReviewCounts();
      setReviewCounts(new Map(counts.map((c) => [c.workspace_id, c.awaiting])));
    } catch {
      // The Pull requests tab reports the provider's problem itself.
    }
  }, []);

  const reloadSnapshot = useCallback(async () => {
    try {
      const next = await ipc.getAppSnapshot();
      // Ignore snapshots older than what we already show.
      setSnapshot((prev) =>
        prev && prev.snapshot_version > next.snapshot_version ? prev : next,
      );
    } catch (e) {
      setBanner(errorMessage(e));
    }
    // Workspaces turn pull requests on and off with the snapshot.
    void reloadReviewCounts();
  }, [reloadReviewCounts]);

  /** The note open in Notes, so coming back to Notes shows it again. */
  const lastNote = useRef<string | undefined>(
    view.kind === "notes" ? view.noteId : undefined,
  );
  const showView = useCallback((next: View) => {
    if (next.kind === "notes") {
      if (next.noteId) lastNote.current = next.noteId;
      else if (lastNote.current)
        next = { kind: "notes", noteId: lastNote.current };
    }
    setView(next);
    setPaletteOpen(false);
    if (next.kind === "repository")
      void ipc.openRepository(next.id).catch(() => {});
  }, []);

  // Remember the section, so Brainiac reopens it.
  useEffect(() => {
    try {
      // Settings is not a section to reopen: the view under it is.
      const shown = view.kind === "settings" ? view.back : view;
      const kept =
        shown.kind === "repository"
          ? { kind: "repository", id: shown.id, workspaceId: shown.workspaceId }
          : shown;
      localStorage.setItem(VIEW_KEY, JSON.stringify(kept));
    } catch {
      // Preference only.
    }
  }, [view]);

  // The vault and how complete search is.
  useEffect(() => {
    void ipc
      .getVaultState()
      .then(setVault)
      .catch((e) => setBanner(errorMessage(e)));
    return subscribe(
      onIndexStatusChanged((index) => {
        setVault((v) => (v ? { ...v, index } : v));
        // The vault becomes available or unavailable with a scan.
        void ipc
          .getVaultState()
          .then(setVault)
          .catch(() => {});
      }),
    );
  }, []);

  const openNote = useCallback(
    (noteId: string) => showView({ kind: "notes", noteId }),
    [showView],
  );
  const openRepo = useCallback(
    (id: string) => showView({ kind: "repository", id }),
    [showView],
  );
  const editTask = useCallback((task: Task) => setEditingTask({ task }), []);

  /** A new note: linked to the open repository, next to the open note, or in Inbox/. */
  const newNote = useCallback(
    async (folder?: string) => {
      if (!vault?.vault) {
        showView({ kind: "notes" });
        return;
      }
      try {
        let target = folder;
        let repositoryId: string | null = null;
        if (target === undefined) {
          if (view.kind === "repository") {
            repositoryId = view.id;
            target = "";
          } else if (view.kind === "notes" && view.noteId) {
            target = folderOf(
              (await ipc.readNote(view.noteId)).note.relative_path,
            );
          } else target = "Inbox";
        }
        const note = await ipc.createNote({
          folder: target || null,
          repository_id: repositoryId,
        });
        showView({ kind: "notes", noteId: note.id });
      } catch (e) {
        setBanner(errorMessage(e));
      }
    },
    [vault, view, showView],
  );

  /** A new task, linked to the open repository or note. */
  const newTask = useCallback(async () => {
    if (view.kind === "repository") {
      setEditingTask({ initial: { repository_id: view.id } });
      return;
    }
    if (view.kind === "notes" && view.noteId) {
      try {
        const note = await ipc.readNote(view.noteId);
        if (!note.note.missing) {
          setEditingTask({
            initialNote: { id: note.note.id, title: note.note.title },
          });
          return;
        }
      } catch {
        // A new task without a note.
      }
    }
    setEditingTask({});
  }, [view]);

  const exportNow = useCallback(async () => {
    const result = await runExport(setNotice, setBanner);
    if (result && !result.complete) setExportProblems(result);
  }, []);

  /** Settings in place of the sidebar and the view; Back returns to the view. */
  const openSettings = useCallback((section: SettingsSection = "general") => {
    setPaletteOpen(false);
    setView((v) => ({
      kind: "settings",
      section,
      back: v.kind === "settings" ? v.back : v,
    }));
  }, []);
  const openAccounts = useCallback(
    () => openSettings("accounts"),
    [openSettings],
  );

  const openRepositoryPicker = useCallback(async () => {
    try {
      const dir = await open({
        directory: true,
        multiple: false,
        title: "Open Repository",
      });
      if (typeof dir !== "string") return;
      const summary = await ipc.registerRepository(dir);
      await reloadSnapshot();
      showView({ kind: "repository", id: summary.id });
      setBanner(null);
    } catch (e) {
      setBanner(errorMessage(e));
    }
  }, [reloadSnapshot, showView]);

  const byId = useMemo(
    () => new Map((snapshot?.repositories ?? []).map((r) => [r.id, r])),
    [snapshot],
  );
  const workspace =
    view.kind === "workspace"
      ? (snapshot?.workspaces.find((w) => w.id === view.id) ?? null)
      : null;
  const selected =
    view.kind === "repository" ? (byId.get(view.id) ?? null) : null;

  // Fall back to the overview when the shown repository or workspace is removed.
  // Today, Tasks, and Notes are always there.
  useEffect(() => {
    if (!snapshot) return;
    if (
      (view.kind === "workspace" && !workspace) ||
      (view.kind === "repository" && !selected)
    )
      setView({ kind: "all" });
  }, [snapshot, view, workspace, selected]);

  /** Repositories the main area covers, for refresh and the status bar. */
  const scope = useMemo(() => {
    if (!snapshot) return [];
    if (selected) return [selected];
    if (workspace) return workspaceRepositories(workspace, byId);
    return snapshot.repositories;
  }, [snapshot, selected, workspace, byId]);

  const refresh = useCallback(async () => {
    try {
      await Promise.allSettled(scope.map((r) => ipc.refreshRepository(r.id)));
      await reloadSnapshot();
    } catch (e) {
      setBanner(errorMessage(e));
    }
  }, [scope, reloadSnapshot]);

  const fetchRepositories = useCallback(
    async (ids: string[]) => {
      const todo = ids.filter((id) => !fetching.has(id));
      if (!todo.length) return;
      setFetching((prev) => new Set([...prev, ...todo]));
      const results = await Promise.allSettled(
        todo.map((id) =>
          ipc.fetchRepository(id).finally(() =>
            setFetching((prev) => {
              const next = new Set(prev);
              next.delete(id);
              return next;
            }),
          ),
        ),
      );
      const moved = results.flatMap((r) =>
        r.status === "fulfilled" ? r.value.moved : [],
      );
      const failed = results.flatMap((r) =>
        r.status === "rejected" ? [r.reason] : [],
      );
      const what =
        todo.length === 1 ? "Fetched" : `Fetched ${todo.length} repositories`;
      setNotice(
        failed.length === todo.length
          ? null
          : moved.length
            ? `${what}: ${plural(moved.length, "ref")} updated`
            : `${what}: already up to date`,
      );
      if (failed.length)
        setBanner(
          failed.length === 1
            ? errorMessage(failed[0])
            : `${failed.length} fetches failed. First: ${errorMessage(failed[0])}`,
        );
      await reloadSnapshot();
    },
    [fetching, reloadSnapshot],
  );

  // A fetch notice fades after a few seconds.
  useEffect(() => {
    if (!notice) return;
    const t = setTimeout(() => setNotice(null), 5000);
    return () => clearTimeout(t);
  }, [notice]);

  const guard = useCallback(
    async (task: () => Promise<unknown>) => {
      try {
        await task();
        await reloadSnapshot();
      } catch (e) {
        setBanner(errorMessage(e));
      }
    },
    [reloadSnapshot],
  );

  const removeRepository = useCallback(
    async (id: string) => {
      const repo = byId.get(id);
      const confirmed = await ask(
        `Remove “${repo?.name ?? "this repository"}” from Brainiac? It also leaves every workspace. Files on disk are not touched.`,
        { title: "Remove Repository", kind: "warning", okLabel: "Remove" },
      );
      if (confirmed) await guard(() => ipc.removeRepository(id));
    },
    [byId, guard],
  );

  /** Points a registration at `path`, asking first when it may not be the same repository. */
  const relocateTo = useCallback(
    async (id: string, path: string) => {
      const name = byId.get(id)?.name ?? "the repository";
      try {
        const request = { repository_id: id, path, confirmed_root: null };
        let outcome = await ipc.relocateRepository(request);
        if (outcome.outcome === "needs_confirmation") {
          const confirmed = await ask(
            relocationQuestion(name, shortPath(outcome.root), outcome.concerns),
            {
              title: "Locate Repository",
              kind: "warning",
              okLabel: "Use This Folder",
            },
          );
          if (!confirmed) return;
          outcome = await ipc.relocateRepository({
            ...request,
            confirmed_root: outcome.root,
          });
        }
        if (outcome.outcome === "relocated") {
          setNotice(
            relocatedNotice(
              name,
              shortPath(outcome.repository.display_path),
              outcome.carried,
            ),
          );
          setBanner(null);
        } else {
          setBanner(
            "The folder changed while you were deciding. Locate it again.",
          );
        }
      } catch (e) {
        setBanner(errorMessage(e));
      }
      await reloadSnapshot();
    },
    [byId, reloadSnapshot],
  );

  /** Asks for the folder a repository moved to. */
  const locate = useCallback(
    async (id: string) => {
      const repo = byId.get(id);
      try {
        const dir = await open({
          directory: true,
          multiple: false,
          title: `Locate “${repo?.name ?? "repository"}”`,
          defaultPath: repo ? parentFolder(repo.display_path) : undefined,
        });
        if (typeof dir === "string") await relocateTo(id, dir);
      } catch (e) {
        setBanner(errorMessage(e));
      }
    },
    [byId, relocateTo],
  );

  const togglePin = useCallback(
    (entityType: PinEntityType, id: string) => {
      const pinned = !!snapshot?.pins.some(
        (p) => p.entity_type === entityType && p.entity_id === id,
      );
      void guard(() => ipc.setPinned(entityType, id, !pinned));
    },
    [snapshot, guard],
  );

  /**
   * Lists repositories in a discovered workspace's folder that it does not
   * track yet, and members that seem to have moved there.
   */
  const rescan = useCallback(async (ws: Workspace, explicit: boolean) => {
    if (ws.discovery_mode !== "discovered" || !ws.discovery_root) return;
    try {
      const found = await ipc.rescanWorkspace(ws.id);
      const result: Discovered = {
        workspaceId: ws.id,
        repositories: found.repositories.map((e) => ({
          name: e.display_name,
          path: e.resolved_path ?? e.configured_path,
        })),
        skipped: found.skipped,
        moves: found.moves,
      };
      // A background scan only speaks up when there is something to act on.
      if (explicit || result.repositories.length || result.moves.length)
        setDiscovered(result);
    } catch (e) {
      if (explicit) setBanner(errorMessage(e));
    }
  }, []);

  const applyMoves = useCallback(
    async (ws: Workspace, moves: SuggestedMove[]) => {
      setDiscovered(null);
      for (const m of moves) await relocateTo(m.repository_id, m.to);
      const fresh = (await ipc.getAppSnapshot()).workspaces.find(
        (w) => w.id === ws.id,
      );
      if (fresh) await rescan(fresh, false);
    },
    [relocateTo, rescan],
  );

  // Check for new repositories whenever a discovered workspace is opened.
  const workspaceId = workspace?.id;
  // biome-ignore lint/correctness/useExhaustiveDependencies: only opening a different workspace triggers the scan.
  useEffect(() => {
    setDiscovered(null);
    if (workspace) void rescan(workspace, false);
  }, [workspaceId]);

  const deleteWorkspace = useCallback(
    async (ws: Workspace) => {
      const confirmed = await ask(
        `Delete the workspace “${ws.name}”? Its repositories stay registered, and files on disk are not touched.`,
        { title: "Delete Workspace", kind: "warning", okLabel: "Delete" },
      );
      if (confirmed) await guard(() => ipc.removeWorkspace(ws.id));
    },
    [guard],
  );

  // Keep the latest callbacks reachable from long-lived event listeners.
  const fetchScope = () =>
    void fetchRepositories(
      scope.filter((r) => r.state !== "missing").map((r) => r.id),
    );
  const actions = useRef({
    refresh,
    fetchScope,
    newNote,
    newTask,
    exportNow,
    openSettings,
    showView,
  });
  actions.current = {
    refresh,
    fetchScope,
    newNote,
    newTask,
    exportNow,
    openSettings,
    showView,
  };

  useEffect(() => {
    void reloadSnapshot();
  }, [reloadSnapshot]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    let pending = false;
    void onRepositoryChanged((e) => {
      // Only real changes reload the open view; unchanged polls just refresh the snapshot.
      const v = viewRef.current;
      if (e.changed && v.kind === "repository" && e.repository_id === v.id)
        setChangeTick((t) => t + 1);
      if (!pending) {
        pending = true;
        setTimeout(() => {
          pending = false;
          void reloadSnapshot();
        }, 150);
      }
    }).then((u) => (disposed ? u() : unlisteners.push(u)));
    let counting = false;
    void onPullRequestChanged(() => {
      // A review given, asked for, or a pull request merged: count again, once per burst.
      if (!counting) {
        counting = true;
        setTimeout(() => {
          counting = false;
          void reloadReviewCounts();
        }, 150);
      }
    }).then((u) => (disposed ? u() : unlisteners.push(u)));
    void onMenu((e) => {
      if (e.id === "open_repository") setAddMode({ kind: "choose" });
      if (e.id === "refresh") void actions.current.refresh();
      if (e.id === "fetch") actions.current.fetchScope();
      if (e.id === "palette") setPaletteOpen(true);
      if (e.id === "check_updates") setUpdateTick((t) => t + 1);
      if (e.id === "new_note") void actions.current.newNote();
      if (e.id === "new_task") void actions.current.newTask();
      if (e.id === "save") setSaveTick((t) => t + 1);
      if (e.id === "toggle_source") setLivePreview(!livePreviewRef.current);
      if (e.id === "toggle_sidebar") toggleSidebarRef.current();
      if (e.id === "toggle_context") toggleSidePanelRef.current();
      if (e.id === "export") void actions.current.exportNow();
      if (e.id === "restore") setDialog("restore");
      if (e.id === "settings") actions.current.openSettings();
      if (e.id === "show_today") actions.current.showView({ kind: "today" });
      if (e.id === "show_tasks") actions.current.showView({ kind: "tasks" });
      if (e.id === "show_notes") actions.current.showView({ kind: "notes" });
      if (e.id === "show_databases")
        actions.current.showView({ kind: "databases" });
    }).then((u) => (disposed ? u() : unlisteners.push(u)));
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [reloadSnapshot, reloadReviewCounts, setLivePreview]);

  const isPinned = (type: PinEntityType, id: string) =>
    !!snapshot?.pins.some((p) => p.entity_type === type && p.entity_id === id);

  /** The update, Git, and error banners at the top of the view. */
  const banners = (
    <>
      <UpdateBanner manualTick={updateTick} />
      {snapshot && !snapshot.git.available && (
        <div className="border-b border-amber-300 bg-amber-50 py-2 px-3 pl-lead text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
          {snapshot.git.message ?? "Git is not available."}
        </div>
      )}
      {banner && (
        <div className="flex items-center gap-3 border-b border-red-300 bg-red-50 py-2 px-3 pl-lead text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100">
          <span className="selectable flex-1">{banner}</span>
          <button
            type="button"
            className="rounded border px-2 py-0.5"
            onClick={() => setBanner(null)}
          >
            Dismiss
          </button>
        </div>
      )}
    </>
  );

  return (
    <SidePanelState open={contextOpen} toggle={flipSidePanel}>
      <div className="flex h-full flex-col">
        {view.kind === "settings" && snapshot ? (
          <SettingsPage
            settings={snapshot.settings}
            section={view.section}
            vault={vault}
            top={banners}
            onSection={(section) => setView({ ...view, section })}
            onBack={() => setView(view.back)}
            onVault={(state) => {
              setVault(state);
              setBanner(null);
            }}
            onChanged={() => void reloadSnapshot()}
            onExport={() => void exportNow()}
            onRestore={() => setDialog("restore")}
          />
        ) : (
          <div className="flex min-h-0 flex-1">
            <Sidebar
              snapshot={snapshot}
              reviewCounts={reviewCounts}
              vault={vault}
              view={view}
              open={sidebarOpen}
              onToggle={flipSidebar}
              onView={showView}
              onAdd={() => setAddMode({ kind: "choose" })}
            />
            <main
              className="flex min-w-0 flex-1 flex-col bg-app"
              data-sidebar={sidebarOpen ? undefined : "hidden"}
            >
              {banners}
              {!snapshot ? (
                <div className="p-6 text-muted">Loading…</div>
              ) : view.kind === "today" ? (
                <TodayView
                  snapshot={snapshot}
                  fetching={fetching}
                  onEdit={editTask}
                  onNew={() => void newTask()}
                  onOpenNote={openNote}
                  onOpenRepo={openRepo}
                  onFetch={(ids) => void fetchRepositories(ids)}
                  onError={setBanner}
                />
              ) : view.kind === "tasks" ? (
                <TasksView
                  snapshot={snapshot}
                  scope={view.scope ?? "open"}
                  onScope={(scope) => setView({ kind: "tasks", scope })}
                  onEdit={editTask}
                  onNew={() => void newTask()}
                  onOpenNote={openNote}
                  onOpenRepo={openRepo}
                  onError={setBanner}
                />
              ) : view.kind === "notes" ? (
                <NotesView
                  snapshot={snapshot}
                  vault={vault}
                  noteId={view.noteId ?? null}
                  livePreview={livePreview}
                  onLivePreview={setLivePreview}
                  contextOpen={contextOpen}
                  onToggleContext={() => setContextOpen(!contextOpen)}
                  toggleContextRef={toggleContextRef}
                  saveTick={saveTick}
                  onOpenNote={openNote}
                  onOpenRepo={openRepo}
                  onNewNote={(folder) => void newNote(folder)}
                  onEditTask={editTask}
                  onNewTask={(note) => setEditingTask({ initialNote: note })}
                  onVault={(state) => {
                    setVault(state);
                    setBanner(null);
                  }}
                  onNotice={setNotice}
                  onError={setBanner}
                  onPinsChanged={() => void reloadSnapshot()}
                  onAddRepository={() => void openRepositoryPicker()}
                  onLocate={(id) => void locate(id)}
                />
              ) : view.kind === "databases" ? null : view.kind ===
                "pullRequest" ? (
                <PullRequestView
                  key={view.reference}
                  reference={view.reference}
                  onBack={() => setView(view.back)}
                  onOpenNote={openNote}
                  onError={setBanner}
                />
              ) : selected ? (
                <RepositoryView
                  key={`${selected.id}:${JSON.stringify(view.kind === "repository" ? (view.focus ?? null) : null)}`}
                  repository={selected}
                  changeTick={changeTick}
                  pinned={isPinned("repository", selected.id)}
                  focus={view.kind === "repository" ? view.focus : undefined}
                  fetching={fetching.has(selected.id)}
                  onFetch={() => void fetchRepositories([selected.id])}
                  onTogglePin={() => togglePin("repository", selected.id)}
                  onPalette={() => setPaletteOpen(true)}
                  onRefresh={() => void refresh()}
                  onRemove={() => void removeRepository(selected.id)}
                  onLocate={() => void locate(selected.id)}
                  onError={setBanner}
                  snapshot={snapshot}
                  hasVault={!!vault?.vault}
                  onOpenNote={openNote}
                  onNewNote={() => void newNote()}
                  onEditTask={editTask}
                  onOpenPullRequest={(reference) =>
                    setView({ kind: "pullRequest", reference, back: view })
                  }
                  onOpenSettings={openAccounts}
                  onChanged={() => void reloadSnapshot()}
                  onNewQuery={(connectionId) => {
                    setDbRequest({
                      kind: "new_query",
                      connectionId,
                      tick: Date.now(),
                    });
                    showView({ kind: "databases" });
                  }}
                />
              ) : (
                <Dashboard
                  key={workspace?.id ?? "all"}
                  snapshot={snapshot}
                  workspace={workspace}
                  pinned={!!workspace && isPinned("workspace", workspace.id)}
                  discovered={discovered}
                  onOpen={(id) =>
                    showView({
                      kind: "repository",
                      id,
                      workspaceId: workspace?.id,
                    })
                  }
                  onPalette={() => setPaletteOpen(true)}
                  onRefreshAll={() => void refresh()}
                  onAdd={() => setAddMode({ kind: "choose" })}
                  onAddToWorkspace={() =>
                    workspace && setAddMode({ kind: "members", workspace })
                  }
                  onRescan={() => workspace && void rescan(workspace, true)}
                  onTrack={(paths) => {
                    if (!workspace) return;
                    setDiscovered(null);
                    void guard(() =>
                      ipc.updateWorkspaceMembership({
                        workspace_id: workspace.id,
                        add: paths,
                        remove: [],
                      }),
                    );
                  }}
                  onDismissDiscovered={() => setDiscovered(null)}
                  onDismissMoves={() =>
                    setDiscovered((d) => (d ? { ...d, moves: [] } : d))
                  }
                  onApplyMoves={(moves) =>
                    workspace && void applyMoves(workspace, moves)
                  }
                  onLocate={(id) => void locate(id)}
                  onRemoveMember={(path) =>
                    workspace &&
                    void guard(() =>
                      ipc.updateWorkspaceMembership({
                        workspace_id: workspace.id,
                        add: [],
                        remove: [path],
                      }),
                    )
                  }
                  onRename={(name) =>
                    workspace &&
                    void guard(() => ipc.renameWorkspace(workspace.id, name))
                  }
                  onDeleteWorkspace={() =>
                    workspace && void deleteWorkspace(workspace)
                  }
                  onTogglePin={() =>
                    workspace && togglePin("workspace", workspace.id)
                  }
                  tab={
                    view.kind === "workspace"
                      ? (view.tab ?? "overview")
                      : "overview"
                  }
                  onTab={(tab) =>
                    workspace &&
                    setView({ kind: "workspace", id: workspace.id, tab })
                  }
                  fetching={fetching}
                  onFetch={(ids) => void fetchRepositories(ids)}
                  onOpenFocused={(id, focus) =>
                    showView({
                      kind: "repository",
                      id,
                      workspaceId: workspace?.id,
                      focus,
                    })
                  }
                  onChanged={() => void reloadSnapshot()}
                  onError={setBanner}
                  onOpenPullRequest={(reference) =>
                    setView({ kind: "pullRequest", reference, back: view })
                  }
                  onOpenSettings={openAccounts}
                />
              )}
              {snapshot && dbMounted && (
                <DatabasesView
                  visible={view.kind === "databases"}
                  saveTick={saveTick}
                  request={dbRequest}
                  onNotice={setNotice}
                  onError={setBanner}
                  onLists={onDbLists}
                />
              )}
            </main>
          </div>
        )}
        <StatusBar
          snapshot={snapshot}
          scope={scope}
          selected={selected}
          view={view}
          notice={notice}
          onRetry={() => void refresh()}
        />
        {paletteOpen && snapshot && (
          <CommandPalette
            snapshot={snapshot}
            onClose={() => setPaletteOpen(false)}
            onView={showView}
            onOpenRepository={() => {
              setPaletteOpen(false);
              void openRepositoryPicker();
            }}
            onNewWorkspace={() => {
              setPaletteOpen(false);
              setAddMode({ kind: "choose" });
            }}
            onFetch={() => {
              setPaletteOpen(false);
              fetchScope();
            }}
            current={selected}
            onLocate={(id) => {
              setPaletteOpen(false);
              void locate(id);
            }}
            onOpenNote={openNote}
            onOpenTask={(id) => {
              setPaletteOpen(false);
              void ipc
                .getTask(id)
                .then((task) => setEditingTask({ task }))
                .catch((e) => setBanner(errorMessage(e)));
            }}
            onNewNote={() => {
              setPaletteOpen(false);
              void newNote();
            }}
            onNewTask={() => {
              setPaletteOpen(false);
              void newTask();
            }}
            dbConnections={dbLists.connections}
            dbQueries={dbLists.queries}
            onDatabase={(request) => {
              setPaletteOpen(false);
              setDbRequest({
                ...request,
                tick: Date.now(),
              } as DatabasesRequest);
              showView({ kind: "databases" });
            }}
          />
        )}
        {editingTask && snapshot && (
          <TaskEditor
            snapshot={snapshot}
            task={editingTask.task}
            initial={editingTask.initial}
            initialNote={editingTask.initialNote}
            onClose={() => setEditingTask(null)}
            onSaved={() => setEditingTask(null)}
          />
        )}
        {dialog === "restore" && (
          <RestoreDialog onClose={() => setDialog(null)} />
        )}
        {exportProblems && (
          <ExportProblems
            result={exportProblems}
            onClose={() => setExportProblems(null)}
          />
        )}
        {addMode && snapshot && (
          <AddDialog
            snapshot={snapshot}
            mode={addMode}
            onClose={() => setAddMode(null)}
            onOpenRepository={() => void openRepositoryPicker()}
            onDone={(ws) => {
              setAddMode(null);
              setDiscovered(null);
              void reloadSnapshot().then(() =>
                showView({ kind: "workspace", id: ws.id }),
              );
            }}
          />
        )}
      </div>
    </SidePanelState>
  );
}
