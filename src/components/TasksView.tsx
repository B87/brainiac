import { useCallback, useEffect, useMemo, useState } from "react";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
  onNoteChanged,
  onTaskChanged,
  subscribe,
  type Task,
  type TaskFilter,
} from "../lib/ipc";
import { step, useKeys } from "../lib/keys";
import { plural } from "../lib/repo";
import { createLatest } from "../lib/stale";
import { localDate, toggled } from "../lib/tasks";
import { PlusIcon, TaskIcon } from "./icons";
import TaskRow from "./TaskRow";

/** The Tasks view's filters (SPEC.md, Tasks and Today). */
export type TaskScope = "open" | "to_sort" | "done" | "cancelled" | "all";

const SCOPES: Array<[TaskScope, string]> = [
  ["open", "Open"],
  ["to_sort", "To sort"],
  ["done", "Done"],
  ["cancelled", "Cancelled"],
  ["all", "All"],
];

function filterOf(scope: TaskScope): Partial<TaskFilter> {
  switch (scope) {
    case "open":
      return { statuses: ["todo", "in_progress"] };
    case "to_sort":
      return { to_sort: true };
    case "done":
      return { statuses: ["done"] };
    case "cancelled":
      return { statuses: ["cancelled"] };
    default:
      return {};
  }
}

/** Reload a list when tasks or notes change, coalescing bursts of events. */
export function useTaskEvents(reload: () => void) {
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const soon = () => {
      clearTimeout(timer);
      timer = setTimeout(reload, 120);
    };
    const off = subscribe(onTaskChanged(soon), onNoteChanged(soon));
    return () => {
      clearTimeout(timer);
      off();
    };
  }, [reload]);
}

/** Mark a task done or open again, sending the version it was read at. */
export async function toggleTask(task: Task): Promise<void> {
  await ipc.updateTask({
    task_id: task.id,
    expected_version: task.version,
    fields: toggled(task),
  });
}

export default function TasksView({
  snapshot,
  scope,
  onScope,
  onEdit,
  onNew,
  onOpenNote,
  onOpenRepo,
  onError,
}: {
  snapshot: AppSnapshot;
  scope: TaskScope;
  onScope: (scope: TaskScope) => void;
  onEdit: (task: Task) => void;
  onNew: () => void;
  onOpenNote: (noteId: string) => void;
  onOpenRepo: (repositoryId: string) => void;
  onError: (message: string | null) => void;
}) {
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const latest = useMemo(() => createLatest(), []);
  const repos = useMemo(
    () => new Map(snapshot.repositories.map((r) => [r.id, r])),
    [snapshot],
  );

  const reload = useCallback(() => {
    void latest.run(
      () => ipc.listTasks(filterOf(scope)),
      setTasks,
      (e) => onError(errorMessage(e)),
    );
  }, [latest, scope, onError]);
  useEffect(reload, [reload]);
  useTaskEvents(reload);

  const toggle = (task: Task) =>
    void toggleTask(task)
      .catch((e) => onError(errorMessage(e)))
      .finally(reload);

  const list = tasks ?? [];
  const current = list.find((t) => t.id === selected) ?? null;
  useKeys({
    j: () => setSelected(step(list, current, 1)?.id ?? null),
    k: () => setSelected(step(list, current, -1)?.id ?? null),
    ArrowDown: () => setSelected(step(list, current, 1)?.id ?? null),
    ArrowUp: () => setSelected(step(list, current, -1)?.id ?? null),
    " ": () => current && toggle(current),
    Enter: () => current && onEdit(current),
  });
  const today = localDate();

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center gap-3 border-b bg-header pr-3 pl-4"
      >
        <TaskIcon className="text-muted" />
        <h1 className="m-0 text-[14px] font-semibold">Tasks</h1>
        <fieldset aria-label="Show" className="m-0 flex gap-1.5 border-0 p-0">
          {SCOPES.map(([s, label]) => (
            <button
              key={s}
              type="button"
              className="chip"
              aria-pressed={scope === s}
              onClick={() => onScope(s)}
            >
              {label}
            </button>
          ))}
        </fieldset>
        <div data-tauri-drag-region className="h-full flex-1" />
        <button type="button" className="btn btn-primary" onClick={onNew}>
          <PlusIcon size={12} />
          New Task
          <span className="text-[11px] opacity-80">⇧⌘N</span>
        </button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {tasks && tasks.length === 0 && (
          <div className="flex flex-col items-center gap-3 px-6 py-16 text-center text-muted">
            <TaskIcon size={28} />
            <p className="m-0 max-w-[360px]">
              {scope === "open"
                ? "No open tasks. Create one with New Task (⇧⌘N); give it a date to see it in Today."
                : scope === "to_sort"
                  ? "Nothing to sort: every task has a date or is marked sorted."
                  : "No tasks here."}
            </p>
            {scope === "open" && (
              <button type="button" className="btn" onClick={onNew}>
                New Task
              </button>
            )}
          </div>
        )}
        {list.map((t) => (
          <TaskRow
            key={t.id}
            task={t}
            repos={repos}
            today={today}
            selected={t.id === selected}
            onToggle={() => toggle(t)}
            onSelect={() => {
              setSelected(t.id);
              onEdit(t);
            }}
            onOpenNote={onOpenNote}
            onOpenRepo={onOpenRepo}
          />
        ))}
      </div>
      {tasks && tasks.length > 0 && (
        <div className="border-t px-4 py-1.5 text-[12px] text-muted">
          {plural(tasks.length, "task")}
        </div>
      )}
    </div>
  );
}
