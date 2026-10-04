import { useCallback, useEffect, useMemo, useState } from "react";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
  type NoteSummary,
  onNoteChanged,
  onTaskChanged,
  type RepositoryNotes,
  type RepositorySummary,
  subscribe,
  type Task,
} from "../lib/ipc";
import { step, useKeys } from "../lib/keys";
import { editedLabel, vaultImageUrl } from "../lib/notes";
import { createLatest } from "../lib/stale";
import { localDate } from "../lib/tasks";
import { LinkIcon, NoteIcon, PlusIcon } from "./icons";
import { NoteEditor } from "./NoteEditor";
import RepositoryDatabases from "./RepositoryDatabases";
import TaskRow from "./TaskRow";
import { toggleTask } from "./TasksView";

/**
 * A repository's Notes tab (SPEC.md, Notes and repositories): the notes
 * linked to it, its open tasks, and notes that mention it, with a read-only
 * preview.
 */
export default function RepositoryNotesTab({
  repository,
  snapshot,
  hasVault,
  onOpenNote,
  onNewNote,
  onEditTask,
  onError,
  onNewQuery,
}: {
  repository: RepositorySummary;
  snapshot: AppSnapshot;
  hasVault: boolean;
  onOpenNote: (noteId: string) => void;
  onNewNote: () => void;
  onEditTask: (task: Task) => void;
  onError: (message: string | null) => void;
  /** A new query tab on a connection linked to the repository (v0.4). */
  onNewQuery: (connectionId: string) => void;
}) {
  const [data, setData] = useState<RepositoryNotes | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [preview, setPreview] = useState<{
    id: string;
    text: string | null;
  } | null>(null);
  const latest = useMemo(() => createLatest(), []);
  const previewLatest = useMemo(() => createLatest(), []);
  const repos = useMemo(
    () => new Map(snapshot.repositories.map((r) => [r.id, r])),
    [snapshot],
  );

  const reload = useCallback(() => {
    void latest.run(
      () => ipc.getRepositoryNotes(repository.id),
      (d) => {
        setData(d);
        setSelected((s) => s ?? d.notes[0]?.id ?? d.suggested[0]?.id ?? null);
      },
      (e) => onError(errorMessage(e)),
    );
  }, [latest, repository.id, onError]);
  useEffect(reload, [reload]);
  useEffect(() => {
    let t: ReturnType<typeof setTimeout> | undefined;
    const soon = () => {
      clearTimeout(t);
      t = setTimeout(reload, 200);
    };
    const off = subscribe(onNoteChanged(soon), onTaskChanged(soon));
    return () => {
      clearTimeout(t);
      off();
    };
  }, [reload]);

  useEffect(() => {
    if (!selected) {
      setPreview(null);
      return;
    }
    void previewLatest.run(
      () => ipc.readNote(selected),
      (c) => setPreview({ id: selected, text: c.text }),
      () => setPreview({ id: selected, text: null }),
    );
  }, [selected, previewLatest]);

  const all: NoteSummary[] = [
    ...(data?.notes ?? []),
    ...(data?.suggested ?? []),
  ];
  const current = all.find((n) => n.id === selected) ?? null;
  const linked = !!data?.notes.some((n) => n.id === selected);
  useKeys({
    j: () => setSelected(step(all, current, 1)?.id ?? selected),
    k: () => setSelected(step(all, current, -1)?.id ?? selected),
    ArrowDown: () => setSelected(step(all, current, 1)?.id ?? selected),
    ArrowUp: () => setSelected(step(all, current, -1)?.id ?? selected),
    Enter: () => selected && onOpenNote(selected),
  });

  const run = (p: Promise<unknown>) =>
    void p.catch((e) => onError(errorMessage(e))).finally(reload);

  if (!hasVault)
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-8 text-center text-muted">
        <NoteIcon size={28} />
        <p className="m-0 max-w-[420px]">
          Choose a vault in Notes to link notes to {repository.name}.
        </p>
      </div>
    );

  const noteRow = (n: NoteSummary) => (
    <button
      key={n.id}
      type="button"
      className="list-row flex-col px-3 py-1.5"
      aria-current={n.id === selected}
      onClick={() => setSelected(n.id)}
      onDoubleClick={() => onOpenNote(n.id)}
    >
      <span
        className={`truncate ${n.missing ? "text-muted line-through" : ""}`}
      >
        {n.title}
      </span>
      <span className="meta mono truncate text-[11px] text-muted">
        {n.relative_path}
      </span>
    </button>
  );

  const empty =
    data && !data.notes.length && !data.tasks.length && !data.suggested.length;
  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex w-[320px] shrink-0 flex-col border-r">
        <div className="flex items-center gap-2 border-b px-3 py-2">
          <span className="section-label flex-1">Linked notes</span>
          <button type="button" className="btn btn-sm" onClick={onNewNote}>
            <PlusIcon size={11} />
            New Note for {repository.name}
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {empty && (
            <p className="m-0 px-3 py-3 text-[12px] text-muted">
              No notes are linked to {repository.name}. Create one here, or open
              a note and use Link… in its context panel. Notes that mention{" "}
              {repository.name} appear below as suggestions.
            </p>
          )}
          {data?.notes.map(noteRow)}
          {data && data.tasks.length > 0 && (
            <>
              <div className="section-label px-3 pt-3 pb-1">Open tasks</div>
              {data.tasks.map((t) => (
                <TaskRow
                  key={t.id}
                  task={t}
                  repos={repos}
                  today={localDate()}
                  showRepo={false}
                  onToggle={() => run(toggleTask(t))}
                  onSelect={() => onEditTask(t)}
                  onOpenNote={onOpenNote}
                />
              ))}
            </>
          )}
          {data && data.suggested.length > 0 && (
            <>
              <div className="section-label px-3 pt-3 pb-1">
                Mention {repository.name}
              </div>
              {data.suggested.map(noteRow)}
            </>
          )}
          <RepositoryDatabases
            repository={repository}
            onNewQuery={onNewQuery}
            onError={onError}
          />
        </div>
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        {current ? (
          <>
            <div className="flex items-center gap-2 border-b px-4 py-2">
              <span className="min-w-0 flex-1 truncate font-medium">
                {current.title}
              </span>
              <span className="text-[12px] text-muted">
                {editedLabel(current.modified_at)}
              </span>
              {linked ? (
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() =>
                    run(ipc.unlinkRepository(current.id, repository.id))
                  }
                >
                  Unlink
                </button>
              ) : (
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() =>
                    run(ipc.linkRepository(current.id, repository.id))
                  }
                >
                  <LinkIcon size={12} />
                  Link
                </button>
              )}
              <button
                type="button"
                className="btn btn-sm btn-primary"
                onClick={() => onOpenNote(current.id)}
              >
                Open Note
              </button>
            </div>
            <div className="relative min-h-0 flex-1">
              {preview?.id === current.id && preview.text !== null ? (
                <NoteEditor
                  key={current.id}
                  text={preview.text}
                  livePreview
                  readOnly
                  onChange={() => {}}
                  resolveImage={(src) =>
                    vaultImageUrl(current.relative_path, src)
                  }
                  className="note-editor"
                />
              ) : (
                <p className="m-0 p-4 text-muted">
                  {preview?.id === current.id
                    ? "This note has no text to preview."
                    : ""}
                </p>
              )}
            </div>
          </>
        ) : (
          <div className="flex flex-1 items-center justify-center text-muted">
            Select a note to preview it.
          </div>
        )}
      </div>
    </div>
  );
}
