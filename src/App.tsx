import { ask, open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import AddDialog, { type AddMode } from "./components/AddDialog";
import CommandPalette from "./components/CommandPalette";
import Dashboard, { type Discovered } from "./components/Dashboard";
import RepositoryView from "./components/RepositoryView";
import Sidebar, { type View } from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import UpdateBanner from "./components/UpdateBanner";
import { shortPath } from "./lib/format";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
  onMenu,
  onRepositoryChanged,
  type PinEntityType,
  type SuggestedMove,
  type Workspace,
} from "./lib/ipc";
import {
  parentFolder,
  plural,
  relocatedNotice,
  relocationQuestion,
} from "./lib/repo";
import { workspaceRepositories } from "./lib/workspace";

export default function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [view, setView] = useState<View>({ kind: "all" });
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
  const viewRef = useRef(view);
  viewRef.current = view;

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
  }, []);

  const showView = useCallback((next: View) => {
    setView(next);
    setPaletteOpen(false);
    if (next.kind === "repository")
      void ipc.openRepository(next.id).catch(() => {});
  }, []);

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
  const actions = useRef({ refresh, fetchScope });
  actions.current = { refresh, fetchScope };

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
    void onMenu((e) => {
      if (e.id === "open_repository") setAddMode({ kind: "choose" });
      if (e.id === "refresh") void actions.current.refresh();
      if (e.id === "fetch") actions.current.fetchScope();
      if (e.id === "palette") setPaletteOpen(true);
      if (e.id === "check_updates") setUpdateTick((t) => t + 1);
    }).then((u) => (disposed ? u() : unlisteners.push(u)));
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [reloadSnapshot]);

  const isPinned = (type: PinEntityType, id: string) =>
    !!snapshot?.pins.some((p) => p.entity_type === type && p.entity_id === id);

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1">
        <Sidebar
          snapshot={snapshot}
          view={view}
          onView={showView}
          onAdd={() => setAddMode({ kind: "choose" })}
        />
        <main className="flex min-w-0 flex-1 flex-col bg-app">
          <UpdateBanner manualTick={updateTick} />
          {snapshot && !snapshot.git.available && (
            <div className="border-b border-amber-300 bg-amber-50 px-3 py-2 text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
              {snapshot.git.message ?? "Git is not available."}
            </div>
          )}
          {banner && (
            <div className="flex items-center gap-3 border-b border-red-300 bg-red-50 px-3 py-2 text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100">
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
          {!snapshot ? (
            <div className="p-6 text-muted">Loading…</div>
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
            />
          )}
        </main>
      </div>
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
  );
}
