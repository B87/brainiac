import { useCallback, useEffect, useMemo, useState } from "react";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
  type NoteContext,
  onNoteChanged,
  onTaskChanged,
  subscribe,
  type Task,
} from "../lib/ipc";
import { folderOf, stem } from "../lib/notes";
import { createLatest } from "../lib/stale";
import { localDate } from "../lib/tasks";
import { CloseIcon, ExternalIcon, LinkIcon, NoteIcon, PlusIcon } from "./icons";
import Popover from "./Popover";
import { RepoLine } from "./RepoChip";
import TaskRow from "./TaskRow";
import { toggleTask } from "./TasksView";

/**
 * The note's context (SPEC.md, Notes view): linked repositories with their
 * live state, its tasks, backlinks, unresolved links, and repositories the
 * note mentions. Brainiac never links anything by itself.
 */
export default function ContextPanel({
  noteId,
  snapshot,
  onCounts,
  onOpenNote,
  onOpenRepo,
  onEditTask,
  onNewTask,
  onError,
  beforeLink,
  onAddRepository,
  onLocate,
}: {
  noteId: string;
  snapshot: AppSnapshot;
  onCounts: (counts: { repositories: number; tasks: number }) => void;
  onOpenNote: (noteId: string) => void;
  onOpenRepo: (repositoryId: string) => void;
  onEditTask: (task: Task) => void;
  onNewTask: () => void;
  onError: (message: string | null) => void;
  /** Save the note's unsaved edits before linking writes its ID into it. */
  beforeLink: () => Promise<void>;
  /** Register a repository again (a removed one's Add Again). */
  onAddRepository: () => void;
  /** Point a missing repository at the folder it moved to. */
  onLocate: (repositoryId: string) => void;
}) {
  const [context, setContext] = useState<NoteContext | null>(null);
  const [linkOpen, setLinkOpen] = useState(false);
  const latest = useMemo(() => createLatest(), []);
  const repos = useMemo(
    () => new Map(snapshot.repositories.map((r) => [r.id, r])),
    [snapshot],
  );

  const reload = useCallback(() => {
    void latest.run(
      () => ipc.getNoteContext(noteId),
      (c) => {
        setContext(c);
        onCounts({
          repositories: c.repositories.length,
          tasks: c.tasks.length,
        });
      },
      (e) => onError(errorMessage(e)),
    );
  }, [latest, noteId, onCounts, onError]);
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

  const run = (p: Promise<unknown>) =>
    void p.catch((e) => onError(errorMessage(e))).finally(reload);

  const linkedIds = new Set(context?.repositories.map((r) => r.repository_id));
  const linkable = snapshot.repositories
    .filter((r) => !linkedIds.has(r.id))
    .sort((a, b) => a.name.localeCompare(b.name));
  const today = localDate();

  const create = async (path: string) => {
    try {
      const note = await ipc.createNote({
        folder: folderOf(path) || null,
        title: stem(path),
      });
      onOpenNote(note.id);
    } catch (e) {
      onError(errorMessage(e));
    }
  };

  return (
    <aside
      aria-label="Note context"
      className="flex w-[300px] shrink-0 flex-col overflow-y-auto border-l bg-panel pb-4"
    >
      <Section
        title="Repositories"
        action={
          <div className="relative">
            <button
              type="button"
              className="btn btn-sm"
              aria-haspopup="menu"
              aria-expanded={linkOpen}
              disabled={!linkable.length}
              onClick={() => setLinkOpen(!linkOpen)}
            >
              <LinkIcon size={12} />
              Link…
            </button>
            {linkOpen && (
              <Popover align="right" onClose={() => setLinkOpen(false)}>
                {linkable.map((r) => (
                  <button
                    key={r.id}
                    type="button"
                    role="menuitem"
                    className="menu-item"
                    onClick={() => {
                      setLinkOpen(false);
                      run(
                        beforeLink().then(() =>
                          ipc.linkRepository(noteId, r.id),
                        ),
                      );
                    }}
                  >
                    <span className="dot" data-state="unknown" />
                    {r.name}
                  </button>
                ))}
              </Popover>
            )}
          </div>
        }
      >
        {context?.repositories.length === 0 && (
          <Empty>Link the repositories this note is about.</Empty>
        )}
        {context?.repositories.map((link) => {
          const repo = repos.get(link.repository_id);
          return (
            <div
              key={link.repository_id}
              className="flex flex-col gap-1.5 rounded-md border bg-app px-2.5 py-2"
            >
              {repo ? (
                <RepoLine repo={repo} />
              ) : (
                <span className="flex items-center gap-1.5">
                  <span className="dot" data-state="missing" />
                  <span className="truncate font-medium">{link.name}</span>
                  <span className="text-[11.5px] text-muted">removed</span>
                </span>
              )}
              <div className="flex flex-wrap gap-1.5">
                {repo?.state === "missing" ? (
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => onLocate(repo.id)}
                  >
                    Locate…
                  </button>
                ) : repo ? (
                  <>
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => onOpenRepo(repo.id)}
                    >
                      Open
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => run(ipc.openInEditor(repo.id))}
                    >
                      <ExternalIcon size={12} />
                      Open in Editor
                    </button>
                  </>
                ) : link.reconnect_to ? (
                  <button
                    type="button"
                    className="btn btn-sm"
                    title="A registered repository has the same remote"
                    onClick={() =>
                      link.reconnect_to &&
                      run(
                        ipc.reconnectRepository(
                          link.repository_id,
                          link.reconnect_to,
                        ),
                      )
                    }
                  >
                    Reconnect to {repos.get(link.reconnect_to)?.name ?? "it"}
                  </button>
                ) : (
                  <button
                    type="button"
                    className="btn btn-sm"
                    title="Register the repository again; one with the same remote is offered as a reconnection"
                    onClick={onAddRepository}
                  >
                    Add Again…
                  </button>
                )}
                <button
                  type="button"
                  className="btn btn-sm btn-ghost ml-auto"
                  onClick={() =>
                    run(ipc.unlinkRepository(noteId, link.repository_id))
                  }
                >
                  Unlink
                </button>
              </div>
            </div>
          );
        })}
        {context && context.suggestions.length > 0 && (
          <div className="mt-1 flex flex-col gap-1">
            <span className="text-[11.5px] text-muted">
              Mentioned in this note
            </span>
            {context.suggestions.map((s) => (
              <div key={s.repository_id} className="flex items-center gap-1.5">
                <span className="dot" data-state="unknown" />
                <span className="flex-1 truncate">{s.name}</span>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() =>
                    run(
                      beforeLink().then(() =>
                        ipc.linkRepository(noteId, s.repository_id),
                      ),
                    )
                  }
                >
                  Link
                </button>
                <button
                  type="button"
                  className="btn btn-sm btn-ghost px-1.5"
                  aria-label={`Dismiss the suggestion to link ${s.name}`}
                  onClick={() =>
                    run(ipc.dismissSuggestion(noteId, s.repository_id))
                  }
                >
                  <CloseIcon />
                </button>
              </div>
            ))}
          </div>
        )}
      </Section>

      <Section
        title="Tasks"
        action={
          <button type="button" className="btn btn-sm" onClick={onNewTask}>
            <PlusIcon size={11} />
            New
          </button>
        }
      >
        {context?.tasks.length === 0 && (
          <Empty>
            No tasks for this note. Tasks are round; checkboxes in the note stay
            note text.
          </Empty>
        )}
        {context && context.tasks.length > 0 && (
          <div className="-mx-2 overflow-hidden rounded-md border bg-app">
            {context.tasks.map((t) => (
              <TaskRow
                key={t.id}
                task={t}
                repos={repos}
                today={today}
                showNote={false}
                onToggle={() => run(toggleTask(t))}
                onSelect={() => onEditTask(t)}
                onOpenRepo={onOpenRepo}
              />
            ))}
          </div>
        )}
      </Section>

      <Section
        title={`Backlinks${context?.backlinks.length ? ` · ${context.backlinks.length}` : ""}`}
      >
        {context?.backlinks.length === 0 && (
          <Empty>No other note links here.</Empty>
        )}
        {context?.backlinks.map((b) => (
          <button
            key={b.note.id}
            type="button"
            className="flex w-full flex-col items-start gap-0.5 rounded-md border-0 bg-transparent px-2 py-1.5 text-left font-[inherit] text-[inherit] hover:bg-control"
            onClick={() => onOpenNote(b.note.id)}
          >
            <span className="flex max-w-full items-center gap-1.5">
              <NoteIcon size={12} className="shrink-0 text-muted" />
              <span className="truncate font-medium">{b.note.title}</span>
            </span>
            {b.excerpt && (
              <span className="line-clamp-2 text-[12px] text-muted">
                {b.excerpt}
              </span>
            )}
          </button>
        ))}
      </Section>

      {context && context.unresolved.length > 0 && (
        <Section title="Unresolved links">
          {context.unresolved.map((u) => (
            <div key={u.raw_target} className="flex items-center gap-2">
              <span
                className="mono flex-1 truncate text-[12px]"
                title={`Line ${u.line}`}
              >
                {u.raw_target}
              </span>
              <button
                type="button"
                className="btn btn-sm"
                title={`Create ${u.suggested_path}`}
                onClick={() => void create(u.suggested_path)}
              >
                Create
              </button>
            </div>
          ))}
        </Section>
      )}
    </aside>
  );
}

function Section({
  title,
  action,
  children,
}: {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col">
      <div className="panel-title">
        <h2 className="section-label m-0">{title}</h2>
        {action}
      </div>
      <div className="flex flex-col gap-1.5 px-3.5 pt-1">{children}</div>
    </section>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="m-0 text-[12px] text-muted">{children}</p>;
}
