import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import CommandPalette from "./components/CommandPalette";
import Inspector from "./components/Inspector";
import OverviewTable from "./components/OverviewTable";
import RepositoryView from "./components/RepositoryView";
import Sidebar from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import UpdateBanner from "./components/UpdateBanner";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
  onMenu,
  onRepositoryChanged,
} from "./lib/ipc";

export default function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  /** Bumped when the selected repository changes on disk; views re-fetch on it. */
  const [changeTick, setChangeTick] = useState(0);
  /** Bumped from the "Check for Updates…" menu item. */
  const [updateTick, setUpdateTick] = useState(0);
  const selectedRef = useRef<string | null>(null);
  selectedRef.current = selectedId;

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

  const selectRepository = useCallback((id: string | null) => {
    setSelectedId(id);
    if (id) void ipc.openRepository(id).catch(() => {});
  }, []);

  const addRepository = useCallback(async () => {
    try {
      const dir = await open({
        directory: true,
        multiple: false,
        title: "Open Repository",
      });
      if (!dir) return;
      const summary = await ipc.registerRepository(dir);
      await reloadSnapshot();
      selectRepository(summary.id);
      setBanner(null);
    } catch (e) {
      setBanner(errorMessage(e));
    }
  }, [reloadSnapshot, selectRepository]);

  const refresh = useCallback(async () => {
    try {
      const id = selectedRef.current;
      if (id) {
        await ipc.refreshRepository(id);
      } else if (snapshot) {
        await Promise.allSettled(
          snapshot.repositories.map((r) => ipc.refreshRepository(r.id)),
        );
      }
      await reloadSnapshot();
    } catch (e) {
      setBanner(errorMessage(e));
    }
  }, [snapshot, reloadSnapshot]);

  const removeRepository = useCallback(
    async (id: string) => {
      try {
        await ipc.removeRepository(id);
        if (selectedRef.current === id) setSelectedId(null);
        await reloadSnapshot();
      } catch (e) {
        setBanner(errorMessage(e));
      }
    },
    [reloadSnapshot],
  );

  // Keep the latest callbacks reachable from long-lived event listeners.
  const actions = useRef({ addRepository, refresh });
  actions.current = { addRepository, refresh };

  useEffect(() => {
    void reloadSnapshot();
  }, [reloadSnapshot]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    let pending = false;
    void onRepositoryChanged((e) => {
      if (e.repository_id === selectedRef.current) setChangeTick((t) => t + 1);
      if (!pending) {
        pending = true;
        setTimeout(() => {
          pending = false;
          void reloadSnapshot();
        }, 150);
      }
    }).then((u) => (disposed ? u() : unlisteners.push(u)));
    void onMenu((e) => {
      if (e.id === "open_repository") void actions.current.addRepository();
      if (e.id === "refresh") void actions.current.refresh();
      if (e.id === "palette") setPaletteOpen(true);
      if (e.id === "check_updates") setUpdateTick((t) => t + 1);
    }).then((u) => (disposed ? u() : unlisteners.push(u)));
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [reloadSnapshot]);

  const selected =
    snapshot?.repositories.find((r) => r.id === selectedId) ?? null;

  return (
    <div className="flex h-full flex-col">
      <UpdateBanner manualTick={updateTick} />
      {snapshot && !snapshot.git.available && (
        <div className="border-b border-amber-300 bg-amber-50 px-3 py-2 text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
          {snapshot.git.message ?? "Git is not available."}
        </div>
      )}
      {banner && (
        <div className="flex items-center gap-3 border-b border-red-300 bg-red-50 px-3 py-2 text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100">
          <span className="flex-1 selectable">{banner}</span>
          <button
            type="button"
            className="rounded border px-2 py-0.5"
            onClick={() => setBanner(null)}
          >
            Dismiss
          </button>
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <Sidebar
          snapshot={snapshot}
          selectedId={selectedId}
          onSelect={selectRepository}
          onAdd={addRepository}
          onPalette={() => setPaletteOpen(true)}
        />
        <main className="pane flex min-w-0 flex-1 flex-col border-l">
          {selected ? (
            <RepositoryView
              key={selected.id}
              repository={selected}
              changeTick={changeTick}
              onError={setBanner}
            />
          ) : (
            <OverviewTable
              snapshot={snapshot}
              onSelect={selectRepository}
              onAdd={addRepository}
            />
          )}
        </main>
        {selected && (
          <Inspector
            repository={selected}
            onRefresh={refresh}
            onRemove={() => removeRepository(selected.id)}
            onError={setBanner}
          />
        )}
      </div>
      <StatusBar snapshot={snapshot} selected={selected} />
      {paletteOpen && snapshot && (
        <CommandPalette
          snapshot={snapshot}
          onClose={() => setPaletteOpen(false)}
          onSelect={(id) => {
            setPaletteOpen(false);
            selectRepository(id);
          }}
          onAdd={() => {
            setPaletteOpen(false);
            void addRepository();
          }}
        />
      )}
    </div>
  );
}
