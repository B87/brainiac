import { useCallback, useEffect, useMemo, useState } from "react";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
  type RepositorySummary,
  type Task,
  type TodayView as Today,
} from "../lib/ipc";
import { step, useKeys } from "../lib/keys";
import { createLatest } from "../lib/stale";
import { formatDay, localDate } from "../lib/tasks";
import {
  ChevronDown,
  ChevronRight,
  ExternalIcon,
  FetchIcon,
  PlusIcon,
  TodayIcon,
} from "./icons";
import { RepoLine } from "./RepoChip";
import TaskRow from "./TaskRow";
import { toggleTask, useTaskEvents } from "./TasksView";

/**
 * Today (SPEC.md, Tasks and Today): open tasks that are overdue, due today,
 * or planned for today or earlier, then those completed today; tasks to sort
 * fold into one line above them. "Today" follows the Mac's date, including
 * across midnight and after waking.
 */
export default function TodayView({
  snapshot,
  fetching,
  onEdit,
  onNew,
  onOpenNote,
  onOpenRepo,
  onFetch,
  onError,
}: {
  snapshot: AppSnapshot;
  fetching: ReadonlySet<string>;
  onEdit: (task: Task) => void;
  onNew: () => void;
  onOpenNote: (noteId: string) => void;
  onOpenRepo: (repositoryId: string) => void;
  onFetch: (ids: string[]) => void;
  onError: (message: string | null) => void;
}) {
  const [today, setToday] = useState<Today | null>(null);
  const [date, setDate] = useState(localDate);
  const [sortOpen, setSortOpen] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const latest = useMemo(() => createLatest(), []);
  const repos = useMemo(
    () => new Map(snapshot.repositories.map((r) => [r.id, r])),
    [snapshot],
  );

  const reload = useCallback(() => {
    void latest.run(
      () => ipc.getToday(),
      setToday,
      (e) => onError(errorMessage(e)),
    );
  }, [latest, onError]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new date reloads.
  useEffect(reload, [reload, date]);
  useTaskEvents(reload);

  // Follow the Mac's date: check every minute and when the window is shown again.
  useEffect(() => {
    const check = () => setDate(localDate());
    const timer = setInterval(check, 60_000);
    window.addEventListener("focus", check);
    document.addEventListener("visibilitychange", check);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", check);
      document.removeEventListener("visibilitychange", check);
    };
  }, []);

  const toggle = (task: Task) =>
    void toggleTask(task)
      .catch((e) => onError(errorMessage(e)))
      .finally(reload);

  const visible = [
    ...(sortOpen ? (today?.to_sort ?? []) : []),
    ...(today?.open ?? []),
    ...(today?.completed ?? []),
  ];
  const current = visible.find((t) => t.id === selected) ?? null;
  useKeys({
    j: () => setSelected(step(visible, current, 1)?.id ?? null),
    k: () => setSelected(step(visible, current, -1)?.id ?? null),
    ArrowDown: () => setSelected(step(visible, current, 1)?.id ?? null),
    ArrowUp: () => setSelected(step(visible, current, -1)?.id ?? null),
    " ": () => current && toggle(current),
    Enter: () => current && onEdit(current),
  });

  const row = (t: Task) => (
    <TaskRow
      key={t.id}
      task={t}
      repos={repos}
      today={today?.date ?? date}
      selected={t.id === selected}
      onToggle={() => toggle(t)}
      onSelect={() => {
        setSelected(t.id);
        onEdit(t);
      }}
      onOpenNote={onOpenNote}
      onOpenRepo={onOpenRepo}
    />
  );
  const todays = (today?.repository_ids ?? [])
    .map((id) => repos.get(id))
    .filter((r): r is RepositorySummary => !!r);
  const empty =
    today &&
    !today.open.length &&
    !today.completed.length &&
    !today.to_sort.length;

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col">
        <header
          data-tauri-drag-region
          className="flex h-12 shrink-0 items-center gap-3 border-b bg-header pr-3 pl-4"
        >
          <TodayIcon className="text-muted" />
          <h1 className="m-0 text-[14px] font-semibold">Today</h1>
          <span className="text-muted">
            {formatDay(today?.date ?? date, date)}
          </span>
          <div data-tauri-drag-region className="h-full flex-1" />
          <button type="button" className="btn btn-primary" onClick={onNew}>
            <PlusIcon size={12} />
            New Task
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto pb-6">
          {today && today.to_sort.length > 0 && (
            <button
              type="button"
              className="flex h-9 w-full items-center gap-2 border-0 border-b bg-panel px-4 text-left font-[inherit] text-fg-2"
              aria-expanded={sortOpen}
              onClick={() => setSortOpen(!sortOpen)}
            >
              {sortOpen ? (
                <ChevronDown size={11} />
              ) : (
                <ChevronRight size={11} />
              )}
              <span className="font-medium">
                To sort · {today.to_sort.length}
              </span>
              <span className="text-[12px] text-muted">
                New tasks without a date; give each one a date or mark it
                sorted.
              </span>
            </button>
          )}
          {sortOpen && today?.to_sort.map(row)}
          {today?.open.map(row)}
          {today && today.completed.length > 0 && (
            <>
              <div className="section-label px-4 pt-5 pb-1">
                Completed today
              </div>
              {today.completed.map(row)}
            </>
          )}
          {empty && (
            <div className="flex flex-col items-center gap-3 px-6 py-16 text-center text-muted">
              <TodayIcon size={28} />
              <p className="m-0 max-w-[380px]">
                Nothing planned or due today. Plan a task for today, or give it
                a deadline, and it shows here.
              </p>
              <button type="button" className="btn" onClick={onNew}>
                New Task
              </button>
            </div>
          )}
        </div>
      </div>
      <aside
        aria-label="Repositories in today's work"
        className="flex w-[300px] shrink-0 flex-col border-l bg-panel"
      >
        <div className="panel-title">
          <span className="section-label">Repositories in today's work</span>
          {todays.length > 1 && (
            <button
              type="button"
              className="btn btn-sm"
              onClick={() =>
                onFetch(
                  todays.filter((r) => r.state !== "missing").map((r) => r.id),
                )
              }
            >
              Fetch all
            </button>
          )}
        </div>
        <div className="flex flex-col gap-1 overflow-y-auto px-2 pb-3">
          {todays.length === 0 && (
            <p className="m-0 px-2 py-1 text-[12px] text-muted">
              Link today's tasks, or their notes, to repositories to see them
              here.
            </p>
          )}
          {todays.map((r) => (
            <div
              key={r.id}
              className="flex flex-col gap-1.5 rounded-md border bg-app px-2.5 py-2"
            >
              <RepoLine repo={r} />
              <div className="flex gap-1.5">
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={fetching.has(r.id) || r.state === "missing"}
                  onClick={() => onFetch([r.id])}
                >
                  <FetchIcon size={12} />
                  {fetching.has(r.id) ? "Fetching…" : "Fetch"}
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => onOpenRepo(r.id)}
                >
                  <ExternalIcon size={12} />
                  Open
                </button>
              </div>
            </div>
          ))}
        </div>
      </aside>
    </div>
  );
}
