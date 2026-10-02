import type { RepositorySummary, Task } from "../lib/ipc";
import { dateNotes, isOpen, localDate, STATUS_LABEL } from "../lib/tasks";
import { AlertIcon, NoteIcon } from "./icons";
import { RepoChip } from "./RepoChip";

/**
 * One task: a round check (never a square checkbox), its title, dates, and
 * its linked note and repository, the repository with its live state.
 */
export default function TaskRow({
  task,
  repos,
  today = localDate(),
  selected = false,
  showNote = true,
  showRepo = true,
  onToggle,
  onSelect,
  onOpenNote,
  onOpenRepo,
}: {
  task: Task;
  repos: Map<string, RepositorySummary>;
  today?: string;
  selected?: boolean;
  showNote?: boolean;
  showRepo?: boolean;
  onToggle: () => void;
  onSelect: () => void;
  onOpenNote?: (noteId: string) => void;
  onOpenRepo?: (repositoryId: string) => void;
}) {
  const open = isOpen(task);
  const notes = dateNotes(task, today);
  return (
    <div
      className="list-row min-h-9 items-start gap-2.5 px-3 py-2"
      aria-current={selected}
      data-task-id={task.id}
    >
      <button
        type="button"
        aria-pressed={task.status === "done"}
        aria-label={
          open ? `Mark “${task.title}” done` : `Reopen “${task.title}”`
        }
        title={`${STATUS_LABEL[task.status]} · Space to toggle`}
        className="task-check mt-0.5"
        data-status={task.status}
        onClick={onToggle}
      >
        {task.status === "done" && (
          <svg width="10" height="10" viewBox="0 0 16 16" aria-hidden="true">
            <path
              d="M3.5 8.5l3 3 6-7"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.2"
            />
          </svg>
        )}
      </button>
      <button
        type="button"
        className="flex min-w-0 flex-1 flex-col items-start gap-1 border-0 bg-transparent p-0 text-left font-[inherit] text-[inherit]"
        onClick={onSelect}
      >
        <span
          className={`max-w-full truncate ${open ? "" : "task-done-title"}`}
        >
          {task.title}
        </span>
        {(notes.length > 0 ||
          task.status === "in_progress" ||
          task.status === "cancelled" ||
          task.to_sort) && (
          <span className="meta flex flex-wrap items-center gap-x-2.5 text-[11.5px] text-muted">
            {task.status === "in_progress" && <span>In progress</span>}
            {task.status === "cancelled" && <span>Cancelled</span>}
            {task.to_sort && <span>To sort</span>}
            {notes.map((n) =>
              n.overdue ? (
                <span key={n.text} className="overdue flex items-center gap-1">
                  <AlertIcon size={11} />
                  {n.text}
                </span>
              ) : (
                <span key={n.text}>{n.text}</span>
              ),
            )}
          </span>
        )}
      </button>
      <span className="flex shrink-0 items-center gap-1.5">
        {showNote && task.note && (
          <button
            type="button"
            className="ref-chip"
            title={
              task.note.missing
                ? "This note's file is gone"
                : task.note.relative_path
            }
            onClick={() => task.note && onOpenNote?.(task.note.id)}
          >
            <NoteIcon size={11} />
            <span className={task.note.missing ? "line-through" : ""}>
              {task.note.title}
            </span>
          </button>
        )}
        {showRepo && task.repository_id && (
          <RepoChip
            repo={repos.get(task.repository_id)}
            onClick={
              repos.has(task.repository_id) && onOpenRepo
                ? () => task.repository_id && onOpenRepo(task.repository_id)
                : undefined
            }
          />
        )}
      </span>
    </div>
  );
}
