import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
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
  const [counts, setCounts] = useState<{
    repositories: number;
    tasks: number;
  } | null>(null);
  const [narrow, setNarrow] = useState(false);
  const host = useRef<HTMLDivElement>(null);

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
  const showContext = contextOpen && !narrow && !!noteId;

  return (
    <div ref={host} className="flex min-h-0 min-w-0 flex-1 flex-col">
      {!vault.vault.available && (
        <div className="banner" data-tone="warn" role="alert">
          <span className="flex-1">
            The vault folder {vault.vault.root_path} can't be read. Connect its
            disk, or choose the vault again. Your notes are not treated as
            deleted.
          </span>
        </div>
      )}
      {vault.index.state === "unavailable" && vault.index.message && (
        <div className="banner" data-tone="warn" role="alert">
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
            onToggleContext={onToggleContext}
            contextCounts={counts}
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
            onCounts={setCounts}
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
