import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
  onNoteChanged,
  onTaskChanged,
  subscribe,
  type Task,
  type VaultState,
} from "../lib/ipc";
import ContextPanel from "./ContextPanel";
import { NoteIcon } from "./icons";
import NotePane from "./NotePane";
import NoteTree from "./NoteTree";
import VaultSetup from "./VaultSetup";

/** Below this width the context panel hides by itself (SPEC.md, Notes view). */
const NARROW = 1080;

/**
 * Notes (SPEC.md, section 5): the vault tree, the editor, and the context
 * panel. Before a vault is chosen it offers the setup instead.
 */
export default function NotesView({
  snapshot,
  vault,
  noteId,
  livePreview,
  onLivePreview,
  contextOpen,
  onToggleContext,
  toggleContextRef,
  saveTick,
  onOpenNote,
  onOpenRepo,
  onNewNote,
  onEditTask,
  onNewTask,
  onVault,
  onNotice,
  onError,
  onPinsChanged,
  onAddRepository,
  onLocate,
}: {
  snapshot: AppSnapshot;
  vault: VaultState | null;
  noteId: string | null;
  livePreview: boolean;
  onLivePreview: (on: boolean) => void;
  contextOpen: boolean;
  onToggleContext: () => void;
  /** Filled with this view's toggle, which ⌥⌘B and ⌥⌘0 call. */
  toggleContextRef: React.RefObject<(() => void) | null>;
  saveTick: number;
  onOpenNote: (noteId: string) => void;
  onOpenRepo: (repositoryId: string) => void;
  onNewNote: (folder: string) => void;
  onEditTask: (task: Task) => void;
  onNewTask: (note: { id: string; title: string }) => void;
  onVault: (state: VaultState) => void;
  onNotice: (message: string) => void;
  onError: (message: string | null) => void;
  /** Pins changed: the snapshot holds them. */
  onPinsChanged: () => void;
  onAddRepository: () => void;
  onLocate: (repositoryId: string) => void;
}) {
  const flush = useRef<(() => Promise<void>) | null>(null);
  // Kept with the note they count, so another note never shows them.
  const [counts, setCounts] = useState<{
    noteId: string;
    repositories: number;
    tasks: number;
  } | null>(null);
  const [narrow, setNarrow] = useState(false);
  // A narrow window shows the panel only when asked, without changing the
  // saved choice for wide windows.
  const [narrowOpen, setNarrowOpen] = useState(false);
  const host = useRef<HTMLDivElement>(null);
  const showContext =
    !!vault?.vault && !!noteId && (narrow ? narrowOpen : contextOpen);

  useEffect(() => {
    toggleContextRef.current = narrow
      ? () => setNarrowOpen((open) => !open)
      : onToggleContext;
    return () => {
      toggleContextRef.current = null;
    };
  }, [toggleContextRef, narrow, onToggleContext]);

  // With the panel hidden, the header still counts the note's repositories
  // and tasks; the panel reports them itself while it is shown.
  useEffect(() => {
    if (!noteId || showContext) return;
    let alive = true;
    let t: ReturnType<typeof setTimeout> | undefined;
    const load = () =>
      void ipc
        .getNoteContext(noteId)
        .then(
          (c) =>
            alive &&
            setCounts({
              noteId,
              repositories: c.repositories.length,
              tasks: c.tasks.length,
            }),
        )
        .catch(() => {});
    load();
    const off = subscribe(
      onNoteChanged(() => {
        clearTimeout(t);
        t = setTimeout(load, 200);
      }),
      onTaskChanged(() => {
        clearTimeout(t);
        t = setTimeout(load, 200);
      }),
    );
    return () => {
      alive = false;
      clearTimeout(t);
      off();
    };
  }, [noteId, showContext]);
  const reportCounts = useCallback(
    (c: { repositories: number; tasks: number }) => {
      if (noteId) setCounts({ noteId, ...c });
    },
    [noteId],
  );

  useEffect(() => {
    if (!host.current) return;
    const observer = new ResizeObserver(([entry]) =>
      setNarrow(entry.contentRect.width < NARROW),
    );
    observer.observe(host.current);
    return () => observer.disconnect();
  }, []);

  const pinKey = snapshot.pins
    .filter((p) => p.entity_type === "note")
    .map((p) => p.entity_id)
    .join(",");
  // Stable while the pins are the same, so the tree reloads only when they change.
  const pinnedIds = useMemo(
    () => new Set(pinKey ? pinKey.split(",") : []),
    [pinKey],
  );
  const togglePin = useCallback(
    (id: string) => {
      const pinned = snapshot.pins.some(
        (p) => p.entity_type === "note" && p.entity_id === id,
      );
      void ipc
        .setPinned("note", id, !pinned)
        .then(onPinsChanged)
        .catch((e) => onError(errorMessage(e)));
    },
    [snapshot, onError, onPinsChanged],
  );
  const newTaskFor = useCallback(async () => {
    if (!noteId) return;
    try {
      const note = await ipc.readNote(noteId);
      onNewTask({ id: noteId, title: note.note.title });
    } catch (e) {
      onError(errorMessage(e));
    }
  }, [noteId, onNewTask, onError]);

  if (!vault?.vault)
    return (
      <div ref={host} className="flex min-h-0 flex-1 flex-col">
        <div
          data-tauri-drag-region
          className="h-12 shrink-0 border-b bg-header"
        />
        <VaultSetup onDone={onVault} />
      </div>
    );
  return (
    <div ref={host} className="flex min-h-0 min-w-0 flex-1 flex-col">
      {!vault.vault.available && (
        <div className="banner pl-lead" data-tone="warn" role="alert">
          <span className="flex-1">
            The vault folder {vault.vault.root_path} can't be read. Connect its
            disk, or choose the vault again. Your notes are not treated as
            deleted.
          </span>
        </div>
      )}
      {vault.index.state === "unavailable" && vault.index.message && (
        <div className="banner pl-lead" data-tone="warn" role="alert">
          <span className="flex-1">{vault.index.message}</span>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() =>
              void ipc.rebuildSearch().catch((e) => onError(errorMessage(e)))
            }
          >
            Rebuild Index
          </button>
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <NoteTree
          vault={vault.vault}
          index={vault.index}
          selectedId={noteId}
          pinnedIds={pinnedIds}
          onOpen={onOpenNote}
          onNew={onNewNote}
          onError={onError}
        />
        {noteId ? (
          <NotePane
            key={`pane:${noteId}`}
            noteId={noteId}
            vault={vault.vault}
            livePreview={livePreview}
            onLivePreview={onLivePreview}
            saveTick={saveTick}
            contextOpen={showContext}
            onToggleContext={() => toggleContextRef.current?.()}
            contextCounts={counts?.noteId === noteId ? counts : null}
            pinned={pinnedIds.has(noteId)}
            onTogglePin={() => togglePin(noteId)}
            onOpenNote={onOpenNote}
            onNotice={onNotice}
            onError={onError}
            flushRef={flush}
          />
        ) : (
          <div className="flex min-w-0 flex-1 flex-col">
            <div
              data-tauri-drag-region
              className="h-12 shrink-0 border-b bg-header"
            />
            <div className="flex flex-1 flex-col items-center justify-center gap-3 text-muted">
              <NoteIcon size={28} />
              <p className="m-0">
                Open a note from the list, or create one with ⌘N.
              </p>
              <button
                type="button"
                className="btn"
                onClick={() => onNewNote("")}
              >
                New Note
              </button>
            </div>
          </div>
        )}
        {showContext && noteId && (
          <ContextPanel
            key={`context:${noteId}`}
            noteId={noteId}
            snapshot={snapshot}
            onCounts={reportCounts}
            onOpenNote={onOpenNote}
            onOpenRepo={onOpenRepo}
            onEditTask={onEditTask}
            onNewTask={() => void newTaskFor()}
            onError={onError}
            beforeLink={() => flush.current?.() ?? Promise.resolve()}
            onAddRepository={onAddRepository}
            onLocate={onLocate}
          />
        )}
      </div>
    </div>
  );
}
