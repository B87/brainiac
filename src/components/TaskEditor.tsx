import { useEffect, useRef, useState } from "react";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
  isAppError,
  type Task,
  type TaskFields,
  type TaskStatus,
} from "../lib/ipc";
import { emptyFields, fieldsOf, isOpen, STATUS_LABEL } from "../lib/tasks";
import Dialog from "./Dialog";
import NotePicker from "./NotePicker";

const STATUSES: TaskStatus[] = ["todo", "in_progress", "done", "cancelled"];

/**
 * Create or edit a task. Edits are sent whole with the version they started
 * from; if the task changed elsewhere meanwhile, the save is refused and the
 * current version is shown (SPEC.md, Tasks and Today).
 */
export default function TaskEditor({
  snapshot,
  task,
  initial,
  initialNote,
  onClose,
  onSaved,
}: {
  snapshot: AppSnapshot;
  /** The task to edit; a new task when absent. */
  task?: Task;
  /** Fields of a new task, such as its linked note or repository. */
  initial?: Partial<TaskFields>;
  initialNote?: { id: string; title: string } | null;
  onClose: () => void;
  onSaved: (task: Task | null) => void;
}) {
  const [current, setCurrent] = useState(task);
  const [fields, setFields] = useState<TaskFields>(
    task ? fieldsOf(task) : emptyFields(initial),
  );
  const [note, setNote] = useState<{ id: string; title: string } | null>(
    task?.note
      ? { id: task.note.id, title: task.note.title }
      : (initialNote ?? null),
  );
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const titleRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    titleRef.current?.focus();
  }, []);

  const set = (patch: Partial<TaskFields>) =>
    setFields((f) => ({ ...f, ...patch }));
  const repos = [...snapshot.repositories].sort((a, b) =>
    a.name.localeCompare(b.name),
  );
  const hasDate = !!(fields.planned_date || fields.due_date);

  const save = async () => {
    setBusy(true);
    setError(null);
    const sent = { ...fields, note_id: note?.id ?? null };
    try {
      const saved = current
        ? await ipc.updateTask({
            task_id: current.id,
            expected_version: current.version,
            fields: sent,
          })
        : await ipc.createTask(sent);
      onSaved(saved);
    } catch (e) {
      if (isAppError(e) && e.code === "CONFLICT" && current) {
        // Show what is there now; the user decides again.
        try {
          const fresh = await ipc.getTask(current.id);
          setCurrent(fresh);
          setFields(fieldsOf(fresh));
          setNote(
            fresh.note ? { id: fresh.note.id, title: fresh.note.title } : null,
          );
        } catch {
          // Deleted meanwhile; the message below says the save failed.
        }
      }
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!current) return;
    setBusy(true);
    try {
      await ipc.deleteTask(current.id, current.version);
      onSaved(null);
    } catch (e) {
      if (isAppError(e) && e.code === "CONFLICT") {
        // Show what is there now; deleting again names that version.
        try {
          const fresh = await ipc.getTask(current.id);
          setCurrent(fresh);
          setFields(fieldsOf(fresh));
          setNote(
            fresh.note ? { id: fresh.note.id, title: fresh.note.title } : null,
          );
        } catch {
          // Deleted meanwhile.
        }
      }
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={current ? "Task" : "New Task"}
      onClose={onClose}
      width={540}
      footer={
        <>
          {current && (
            <button
              type="button"
              className="btn mr-auto text-conflict"
              disabled={busy}
              onClick={() => void remove()}
            >
              Delete Task
            </button>
          )}
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || !fields.title.trim()}
            onClick={() => void save()}
          >
            {current ? "Save" : "Create Task"}
          </button>
        </>
      }
    >
      <form
        className="flex flex-col gap-3.5"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && e.metaKey) {
            e.preventDefault();
            void save();
          }
        }}
      >
        {error && (
          <div
            role="alert"
            className="rounded-md border border-conflict/40 px-3 py-2 text-conflict"
          >
            {error}
          </div>
        )}
        <label className="flex flex-col gap-1">
          <span className="section-label">Title</span>
          <input
            ref={titleRef}
            className="text-input h-8 text-[14px]"
            value={fields.title}
            maxLength={300}
            onChange={(e) => set({ title: e.target.value })}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="section-label">Description</span>
          <textarea
            className="text-input selectable h-20 resize-none py-1.5"
            value={fields.description}
            maxLength={2000}
            placeholder="A short note; longer material belongs in a linked note."
            onChange={(e) => set({ description: e.target.value })}
          />
        </label>
        <div className="flex flex-col gap-1">
          <span className="section-label">Status</span>
          <fieldset aria-label="Status" className="seg m-0 self-start border-0">
            {STATUSES.map((s) => (
              <button
                key={s}
                type="button"
                aria-pressed={fields.status === s}
                onClick={() => set({ status: s })}
              >
                {STATUS_LABEL[s]}
              </button>
            ))}
          </fieldset>
        </div>
        <div className="flex gap-4">
          <DateField
            label="Planned for"
            value={fields.planned_date}
            onChange={(planned_date) => set({ planned_date })}
          />
          <DateField
            label="Deadline"
            value={fields.due_date}
            onChange={(due_date) => set({ due_date })}
          />
        </div>
        {!hasDate && isOpen(fields) && (
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={fields.sorted}
              onChange={(e) => set({ sorted: e.target.checked })}
            />
            <span>Sorted</span>
            <span className="text-[12px] text-muted">
              Without a date, a task is “to sort” until marked sorted.
            </span>
          </label>
        )}
        <div className="flex flex-col gap-1">
          <span className="section-label">Note</span>
          <NotePicker value={note} onChange={setNote} />
        </div>
        <label className="flex flex-col gap-1">
          <span className="section-label">Repository</span>
          <select
            className="text-input"
            value={fields.repository_id ?? ""}
            onChange={(e) => set({ repository_id: e.target.value || null })}
          >
            <option value="">None</option>
            {fields.repository_id &&
              !repos.some((r) => r.id === fields.repository_id) && (
                <option value={fields.repository_id}>Removed repository</option>
              )}
            {repos.map((r) => (
              <option key={r.id} value={r.id}>
                {r.name}
              </option>
            ))}
          </select>
          <span className="text-[12px] text-muted">
            Work spanning several repositories links a note that covers them.
          </span>
        </label>
      </form>
    </Dialog>
  );
}

function DateField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string | null;
  onChange: (value: string | null) => void;
}) {
  return (
    <label className="flex flex-col gap-1">
      <span className="section-label">{label}</span>
      <span className="flex items-center gap-1">
        <input
          type="date"
          className="text-input"
          value={value ?? ""}
          onChange={(e) => onChange(e.target.value || null)}
        />
        {value && (
          <button
            type="button"
            className="btn btn-sm btn-ghost"
            onClick={() => onChange(null)}
          >
            Clear
          </button>
        )}
      </span>
    </label>
  );
}
