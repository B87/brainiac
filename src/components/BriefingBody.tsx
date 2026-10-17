import { openUrl } from "@tauri-apps/plugin-opener";
import { relativeTime } from "../lib/format";
import type { BriefingFinding, BriefingResult } from "../lib/ipc";

const SEVERITY_CLASS = {
  info: "text-muted",
  low: "text-fg-2",
  medium: "text-dirty",
  high: "text-conflict",
} as const;

/**
 * One briefing, as a routine's Latest and History show it. A run that left
 * none says why and never reads as "nothing found".
 */
export default function BriefingBody({
  result,
  busy,
  onTask,
  onNote,
  onSaveNote,
  onOpenRun,
}: {
  result: BriefingResult | null;
  busy: string | null;
  onTask: (key: string) => void;
  onNote: (key: string) => void;
  onSaveNote: () => void;
  onOpenRun: (runId: string) => void;
}) {
  if (!result) {
    return (
      <p className="m-0 px-5 py-4 text-muted">
        Not run yet. Run Now starts it when you ask.
      </p>
    );
  }
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
      <div className="flex items-center gap-2 px-5 py-4 text-muted">
        Running since {when}. {runLink}
      </div>
    );
  }
  if (result.state === "failed") {
    return (
      <div className="flex flex-col gap-1 px-5 py-4" role="alert">
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
    <div className="flex flex-col gap-3 px-5 py-4">
      <div>
        <strong>
          {result.status === "nothing_found"
            ? "Nothing found"
            : result.status === "findings"
              ? "Findings"
              : "Report"}
        </strong>{" "}
        <span className="text-muted">· {when}</span>
        <div className="selectable mt-0.5">{result.summary}</div>
      </div>
      {result.findings.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
          {result.findings.map((f) => (
            <FindingRow key={f.key} finding={f} />
          ))}
        </ul>
      )}
      {result.proposals.length > 0 && (
        <div className="flex flex-col gap-1">
          <span className="section-label">Proposed</span>
          {result.proposals.map((p) => (
            <div key={p.key} className="flex items-center gap-2">
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
    <li className="flex min-w-0 items-baseline gap-2">
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
