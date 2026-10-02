/**
 * Task presentation (SPEC.md, Tasks and Today): dates are local calendar
 * days, an overdue task says so in words, and the editor sends whole fields.
 */
import type { Task, TaskFields, TaskStatus } from "./ipc";

export const STATUS_LABEL: Record<TaskStatus, string> = {
  todo: "To do",
  in_progress: "In progress",
  done: "Done",
  cancelled: "Cancelled",
};

/** The Mac's local date as `YYYY-MM-DD`. */
export function localDate(now = new Date()): string {
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, "0");
  const d = String(now.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

/** A calendar date, read as local: never shifted by the time zone. */
function parseDay(day: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) return null;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
}

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
export const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

/** "Fri 3 Oct", with the year when it is not this year's. */
export function formatDay(day: string, today = localDate()): string {
  const date = parseDay(day);
  if (!date) return day;
  const year =
    day.slice(0, 4) === today.slice(0, 4) ? "" : ` ${date.getFullYear()}`;
  return `${WEEKDAYS[date.getDay()]} ${date.getDate()} ${MONTHS[date.getMonth()]}${year}`;
}

/** What a task's dates mean today. Overdue is always said in words. */
export type DateNote = { text: string; overdue: boolean };

export function dateNotes(task: Task, today = localDate()): DateNote[] {
  const notes: DateNote[] = [];
  const open = task.status === "todo" || task.status === "in_progress";
  if (task.due_date) {
    const overdue = open && task.due_date < today;
    notes.push({
      text: overdue
        ? `Overdue · due ${formatDay(task.due_date, today)}`
        : task.due_date === today
          ? "Due today"
          : `Due ${formatDay(task.due_date, today)}`,
      overdue,
    });
  }
  if (task.planned_date && task.planned_date !== task.due_date) {
    notes.push({
      text:
        task.planned_date === today
          ? "Planned today"
          : `Planned ${formatDay(task.planned_date, today)}`,
      overdue: false,
    });
  }
  return notes;
}

export function emptyFields(over: Partial<TaskFields> = {}): TaskFields {
  return {
    title: "",
    description: "",
    status: "todo",
    planned_date: null,
    due_date: null,
    note_id: null,
    repository_id: null,
    sorted: false,
    ...over,
  };
}

/** A task's editable fields, as the editor sends them back. */
export function fieldsOf(task: Task): TaskFields {
  return {
    title: task.title,
    description: task.description,
    status: task.status,
    planned_date: task.planned_date,
    due_date: task.due_date,
    note_id: task.note?.id ?? null,
    repository_id: task.repository_id,
    sorted: !task.to_sort,
  };
}

export function isOpen(task: Pick<Task, "status">): boolean {
  return task.status === "todo" || task.status === "in_progress";
}

/** The fields after the round check is clicked: done, or open again. */
export function toggled(task: Task): TaskFields {
  return { ...fieldsOf(task), status: isOpen(task) ? "done" : "todo" };
}
