import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useState } from "react";
import { durationLabel } from "../lib/agentRuns";
import { relativeTime } from "../lib/format";
import {
  type Briefing,
  type BriefingFinding,
  type BriefingOverview,
  type BriefingResult,
  errorMessage,
  ipc,
  type RepositorySummary,
} from "../lib/ipc";
import BriefingDialog from "./BriefingDialog";

const SEVERITY_CLASS = {
  info: "text-muted",
  low: "text-fg-2",
  medium: "text-dirty",
  high: "text-conflict",
} as const;

/**
 * Briefings on Today (docs/design/scheduled-runs.md): each saved briefing and
 * what its last run left. A run that left nothing usable says so; it is never
 * shown as "nothing found".
 */
export default function BriefingsSection({
  repositories,
  onOpenNote,
  onOpenRun,
  onError,
}: {
  repositories: RepositorySummary[];
  onOpenNote: (noteId: string) => void;
  onOpenRun: (runId: string) => void;
  onError: (message: string | null) => void;
}) {
  const [items, setItems] = useState<BriefingOverview[] | null>(null);
  const [editing, setEditing] = useState<Briefing | "new" | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setItems(await ipc.listBriefings());
    } catch (e) {
      onError(errorMessage(e));
    }
  }, [onError]);
  useEffect(() => {
    void load();
  }, [load]);
  // While a run is going, look again every few seconds.
  const working = items?.some((i) => i.latest?.state === "working") ?? false;
  useEffect(() => {
    if (!working) return;
    const timer = setInterval(() => void load(), 4000);
    return () => clearInterval(timer);
  }, [working, load]);

  const act = async (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    try {
      await action();
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setBusy(null);
      void load();
    }
  };

  if (items === null) return null;
  return (
    <section aria-label="Briefings" className="border-b pb-3">
      <div className="flex items-center gap-2 px-4 pt-4 pb-1">
        <span className="section-label flex-1">Briefings</span>
        <button
          type="button"
          className="btn btn-sm"
          disabled={repositories.length === 0}
          onClick={() => setEditing("new")}
        >
          New Briefing…
        </button>
      </div>
      {items.length === 0 && (
        <p className="m-0 px-4 py-2 text-[12.5px] text-muted">
          A briefing is a run that checks something and leaves a short report
          here, such as production errors each morning. Add one, then Run now.
        </p>
      )}
      {items.map((item) => (
        <BriefingCard
          key={item.briefing.id}
          item={item}
          busy={busy}
          onEdit={() => setEditing(item.briefing)}
          onRun={() =>
            void act(`run-${item.briefing.id}`, () =>
              ipc.runBriefingNow(item.briefing.id),
            )
          }
          onSaveNote={(result) =>
            void act(`note-${result.id}`, async () => {
              const note = await ipc.saveBriefingAsNote(result.id);
              onOpenNote(note.id);
            })
          }
          onTask={(result, key) =>
            void act(`task-${result.id}-${key}`, () =>
              ipc.createBriefingTask(result.id, key),
            )
          }
          onNote={(result, key) =>
            void act(`proposal-note-${result.id}-${key}`, async () => {
              const note = await ipc.createBriefingNote(result.id, key);
              onOpenNote(note.id);
            })
          }
          onOpenRun={onOpenRun}
        />
      ))}
      {editing && (
        <BriefingDialog
          briefing={editing === "new" ? null : editing}
          repositories={repositories}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            void load();
          }}
        />
      )}
    </section>
  );
}

function BriefingCard({
  item,
  busy,
  onEdit,
  onRun,
  onSaveNote,
  onTask,
  onNote,
  onOpenRun,
}: {
  item: BriefingOverview;
  busy: string | null;
  onEdit: () => void;
  onRun: () => void;
  onSaveNote: (result: BriefingResult) => void;
  onTask: (result: BriefingResult, key: string) => void;
  onNote: (result: BriefingResult, key: string) => void;
  onOpenRun: (runId: string) => void;
}) {
  const { briefing, latest } = item;
  const running = latest?.state === "working";
  return (
    <fieldset
      aria-label={briefing.name}
      className="m-0 mx-3 mt-2 flex min-w-0 flex-col gap-2 rounded-md border bg-panel px-3 py-2.5"
    >
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate font-medium">
          {briefing.name}{" "}
          <span className="text-[12px] font-normal text-muted">
            {item.repository_name} ·{" "}
            {durationLabel(briefing.time_limit_minutes)}
          </span>
        </span>
        <button type="button" className="btn btn-sm" onClick={onEdit}>
          Edit…
        </button>
        <button
          type="button"
          className="btn btn-sm btn-primary"
          disabled={running || busy === `run-${briefing.id}`}
          onClick={onRun}
        >
          {running ? "Running…" : "Run Now"}
        </button>
      </div>
      {!latest && (
        <span className="text-[12.5px] text-muted">Not run yet.</span>
      )}
      {latest && (
        <Result
          result={latest}
          busy={busy}
          onSaveNote={() => onSaveNote(latest)}
          onTask={(key) => onTask(latest, key)}
          onNote={(key) => onNote(latest, key)}
          onOpenRun={onOpenRun}
        />
      )}
    </fieldset>
  );
}

function Result({
  result,
  busy,
  onSaveNote,
  onTask,
  onNote,
  onOpenRun,
}: {
  result: BriefingResult;
  busy: string | null;
  onSaveNote: () => void;
  onTask: (key: string) => void;
  onNote: (key: string) => void;
  onOpenRun: (runId: string) => void;
}) {
  const when = relativeTime(result.finished_at ?? result.started_at);
  const runLink = result.run_id && (
    <button
      type="button"
      className="btn btn-sm btn-ghost text-link"
      onClick={() => result.run_id && onOpenRun(result.run_id)}
    >
      Open the run
    </button>
  );
  if (result.state === "working") {
    return (
      <div className="flex items-center gap-2 text-[12.5px] text-muted">
        Running since {when}. {runLink}
      </div>
    );
  }
  if (result.state === "failed") {
    return (
      <div className="flex flex-col gap-1 text-[12.5px]" role="alert">
        <span className="text-conflict">
          No briefing: {result.error ?? "the run failed."}
        </span>
        <span className="text-muted">
          This is not "nothing found". {when}. {runLink}
        </span>
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-2">
      <div className="text-[12.5px]">
        <strong>
          {result.status === "nothing_found"
            ? "Nothing found"
            : result.status === "findings"
              ? "Findings"
              : "Report"}
        </strong>{" "}
        <span className="text-muted">
          · {when}
          {result.duration_secs != null &&
            ` · ${Math.max(1, Math.round(result.duration_secs / 60))} min`}
        </span>
        <div className="selectable mt-0.5">{result.summary}</div>
      </div>
      {result.findings.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-1 p-0">
          {result.findings.map((f) => (
            <FindingRow key={f.key} finding={f} />
          ))}
        </ul>
      )}
      {result.proposals.length > 0 && (
        <div className="flex flex-col gap-1">
          <span className="section-label">Proposed</span>
          {result.proposals.map((p) => (
            <div key={p.key} className="flex items-center gap-2 text-[12.5px]">
              <span className="min-w-0 flex-1">
                <span className="text-muted">
                  {p.kind === "task"
                    ? "Task"
                    : p.kind === "note"
                      ? "Note"
                      : "Issue"}{" "}
                  ·{" "}
                </span>
                {p.title}
              </span>
              {p.kind === "task" && !p.made_id && (
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={busy === `task-${result.id}-${p.key}`}
                  onClick={() => onTask(p.key)}
                >
                  Create Task
                </button>
              )}
              {p.kind === "note" && !p.made_id && (
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={busy === `proposal-note-${result.id}-${p.key}`}
                  onClick={() => onNote(p.key)}
                >
                  Save as Note
                </button>
              )}
              {p.made_id && (
                <span className="text-muted">
                  {p.kind === "note" ? "Note saved" : "Task created"}
                </span>
              )}
              {p.kind === "issue" && (
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() =>
                    void navigator.clipboard
                      ?.writeText(`${p.title}\n\n${p.body}`)
                      .catch(() => {})
                  }
                >
                  Copy
                </button>
              )}
            </div>
          ))}
        </div>
      )}
      <div className="flex gap-1.5">
        <button
          type="button"
          className="btn btn-sm"
          disabled={busy === `note-${result.id}`}
          onClick={onSaveNote}
        >
          Save as Note
        </button>
        {runLink}
      </div>
    </div>
  );
}

function FindingRow({ finding }: { finding: BriefingFinding }) {
  return (
    <li className="flex min-w-0 items-baseline gap-2 text-[12.5px]">
      {finding.severity && (
        <span className={`w-14 shrink-0 ${SEVERITY_CLASS[finding.severity]}`}>
          {finding.severity}
        </span>
      )}
      <span className="min-w-0 flex-1">
        {finding.link ? (
          <button
            type="button"
            className="btn btn-ghost btn-sm p-0 text-link"
            onClick={() => void openUrl(finding.link).catch(() => {})}
          >
            {finding.title}
          </button>
        ) : (
          finding.title
        )}
        {finding.seen_before && (
          <span className="text-muted"> · seen before</span>
        )}
        {finding.detail && (
          <span className="selectable block text-muted">{finding.detail}</span>
        )}
      </span>
    </li>
  );
}
