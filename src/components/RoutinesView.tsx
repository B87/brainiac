import { useState } from "react";
import { durationLabel } from "../lib/agentRuns";
import { relativeTime } from "../lib/format";
import {
  errorMessage,
  ipc,
  type RepositorySummary,
  type RoutineOverview,
} from "../lib/ipc";
import { resultWord, routineState, sortRoutines } from "../lib/routines";
import RoutinePage, { type RoutineTab } from "./RoutinePage";
import StatePill from "./StatePill";

/**
 * Routines (docs/design/routines-ux.md): every saved routine in a table with a
 * side panel for the selected one, or one routine's page. Things that need
 * the user sort first.
 */
export default function RoutinesView({
  routines,
  repositories,
  routineId,
  tab,
  onOpen,
  onBack,
  onReload,
  onOpenRun,
  onOpenNote,
  onError,
}: {
  routines: RoutineOverview[] | null;
  repositories: RepositorySummary[];
  /** `"new"` for a routine being added; an ID for its page; `null` for the list. */
  routineId: string | null;
  tab: RoutineTab;
  onOpen: (routineId: string | null, tab?: RoutineTab) => void;
  onBack: () => void;
  onReload: () => void;
  onOpenRun: (runId: string) => void;
  onOpenNote: (noteId: string) => void;
  onError: (message: string | null) => void;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  if (routineId) {
    const item = routines?.find((i) => i.routine.id === routineId) ?? null;
    if (routineId === "new" || item) {
      return (
        <RoutinePage
          key={routineId}
          item={routineId === "new" ? null : item}
          repositories={repositories}
          tab={routineId === "new" ? "definition" : tab}
          onTab={(t) => onOpen(routineId, t)}
          onBack={onBack}
          onChanged={onReload}
          onCreated={(r) => {
            onReload();
            onOpen(r.id, "latest");
          }}
          onDeleted={onBack}
          onOpenRun={onOpenRun}
          onOpenNote={onOpenNote}
          onError={onError}
        />
      );
    }
  }

  const list = sortRoutines(routines ?? []);
  const current =
    list.find((i) => i.routine.id === selected) ?? list[0] ?? null;
  const run = async (id: string) => {
    setBusy(id);
    try {
      await ipc.runRoutineNow(id);
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setBusy(null);
      onReload();
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center gap-3 border-b bg-header pr-3 pl-4 pl-lead"
      >
        <h1 className="m-0 text-[14px] font-semibold">Routines</h1>
        <div data-tauri-drag-region className="h-full flex-1" />
        <button
          type="button"
          className="btn btn-primary"
          disabled={repositories.length === 0}
          onClick={() => onOpen("new")}
        >
          New Routine…
        </button>
      </header>
      {routines === null ? null : list.length === 0 ? (
        <div className="flex flex-col items-center gap-3 px-6 py-16 text-center text-muted">
          <p className="m-0 max-w-[440px]">
            <strong className="font-medium text-fg">
              A routine is a run that checks something and leaves a short
              report, a briefing.
            </strong>{" "}
            For example: "Check production errors every morning and tell me what
            is new."
          </p>
          <button
            type="button"
            className="btn btn-primary"
            disabled={repositories.length === 0}
            onClick={() => onOpen("new")}
          >
            New Routine…
          </button>
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-1 overflow-y-auto @min-[760px]:grid-cols-[minmax(0,1fr)_290px]">
          <div className="min-w-0 @container">
            <div className="routine-row bg-panel py-1.5 text-[11px] uppercase tracking-wider text-muted">
              <span>Name</span>
              <span className="routine-opt">Runs on</span>
              <span>Last briefing</span>
              <span>Status</span>
              <span className="routine-opt text-right">Cost</span>
            </div>
            <ul aria-label="Routines" className="m-0 list-none p-0">
              {list.map((i) => {
                const st = routineState(i);
                return (
                  <li key={i.routine.id}>
                    <button
                      type="button"
                      aria-pressed={i.routine.id === current?.routine.id}
                      className="routine-row w-full border-0 border-b bg-transparent text-left font-[inherit] hover:bg-panel aria-pressed:bg-accent-soft"
                      onClick={() => setSelected(i.routine.id)}
                      onDoubleClick={() => onOpen(i.routine.id)}
                    >
                      <span className="min-w-0">
                        <span className="block truncate font-medium">
                          {i.routine.name}
                        </span>
                        <span className="block truncate text-[12px] text-muted">
                          {i.repository_name}
                        </span>
                      </span>
                      <span className="routine-opt truncate">
                        {hostName(i)}
                      </span>
                      <span className="min-w-0">
                        <span className="block truncate">
                          {i.latest ? resultWord(i.latest) : "—"}
                        </span>
                        <span className="block truncate text-[12px] text-muted">
                          {i.latest
                            ? relativeTime(
                                i.latest.finished_at ?? i.latest.started_at,
                              )
                            : ""}
                        </span>
                      </span>
                      <span className="min-w-0">
                        <StatePill tone={st.tone}>{st.word}</StatePill>
                        {st.key === "failed" && i.latest?.error && (
                          <span className="block truncate text-[12px] text-muted">
                            {i.latest.error}
                          </span>
                        )}
                      </span>
                      <span className="routine-opt mono text-right text-muted">
                        —
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
          {current && (
            <aside
              aria-label={current.routine.name}
              className="flex min-w-0 flex-col gap-3 border-t p-4 @min-[760px]:border-t-0 @min-[760px]:border-l"
            >
              <h2 className="m-0 text-[14px] font-semibold">
                {current.routine.name}
              </h2>
              <StatePill tone={routineState(current).tone}>
                {routineState(current).word}
              </StatePill>
              <dl className="m-0 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
                <dt className="text-muted">Repository</dt>
                <dd className="m-0">{current.repository_name}</dd>
                <dt className="text-muted">Runs on</dt>
                <dd className="m-0">{hostName(current)}</dd>
                <dt className="text-muted">Limit</dt>
                <dd className="m-0">
                  {durationLabel(current.routine.time_limit_minutes)}
                </dd>
              </dl>
              {current.latest?.summary && (
                <p className="m-0 text-muted">{current.latest.summary}</p>
              )}
              {current.latest?.state === "failed" && (
                <p className="m-0 text-conflict">{current.latest.error}</p>
              )}
              <div className="flex flex-wrap gap-1.5">
                <button
                  type="button"
                  className="btn btn-sm btn-primary"
                  disabled={
                    current.latest?.state === "working" ||
                    busy === current.routine.id
                  }
                  onClick={() => void run(current.routine.id)}
                >
                  {current.latest?.state === "working" ? "Running…" : "Run Now"}
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => onOpen(current.routine.id)}
                >
                  Open
                </button>
              </div>
            </aside>
          )}
        </div>
      )}
    </div>
  );
}

function hostName(i: RoutineOverview): string {
  return i.routine.host_id === "" ? "This Mac" : i.host_name;
}
