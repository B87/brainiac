import { useCallback, useEffect, useState } from "react";
import { providerName } from "../lib/agentRuns";
import { relativeTime } from "../lib/format";
import {
  type AgentSettings,
  type BriefingResult,
  errorMessage,
  ipc,
  type RepositorySummary,
  type Routine,
  type RoutineOverview,
} from "../lib/ipc";
import { resultWord, routineState } from "../lib/routines";
import BriefingBody from "./BriefingBody";
import RoutineForm from "./RoutineForm";
import ScheduleTab from "./ScheduleTab";
import StatePill from "./StatePill";

export type RoutineTab = "latest" | "history" | "definition" | "schedule";

const TABS: Array<[RoutineTab, string]> = [
  ["latest", "Latest"],
  ["history", "History"],
  ["definition", "Definition"],
  ["schedule", "Schedule"],
];

/**
 * A routine's page (docs/design/routines-ux.md): the state and the disclosure
 * above, then Latest, History and Definition. A new routine opens it on
 * Definition with nothing saved yet.
 */
export default function RoutinePage({
  item,
  repositories,
  tab,
  onTab,
  onBack,
  onChanged,
  onCreated,
  onDeleted,
  onOpenRun,
  onOpenNote,
  onError,
}: {
  /** `null`: a routine being added. */
  item: RoutineOverview | null;
  repositories: RepositorySummary[];
  tab: RoutineTab;
  onTab: (tab: RoutineTab) => void;
  onBack: () => void;
  onChanged: () => void;
  onCreated: (r: Routine) => void;
  onDeleted: () => void;
  onOpenRun: (runId: string) => void;
  onOpenNote: (noteId: string) => void;
  onError: (message: string | null) => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [menu, setMenu] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [settings, setSettings] = useState<AgentSettings | null>(null);
  useEffect(() => {
    let alive = true;
    ipc
      .getAgentSettings()
      .then((s) => alive && setSettings(s))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const routine = item?.routine ?? null;
  const act = async (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    try {
      await action();
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setBusy(null);
      onChanged();
    }
  };

  const crumb = (
    <nav className="flex items-center gap-1.5 text-[12px] text-muted">
      <button
        type="button"
        className="btn btn-sm btn-ghost p-0 text-link"
        onClick={onBack}
      >
        Routines
      </button>
      <span aria-hidden="true">›</span>
      <span className="truncate text-fg">{routine?.name ?? "New routine"}</span>
    </nav>
  );

  if (!item || !routine) {
    return (
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
        <header className="flex flex-col gap-1 border-b bg-header px-5 py-3">
          {crumb}
          <h1 className="m-0 text-[17px] font-semibold">New routine</h1>
        </header>
        <RoutineForm
          routine={null}
          repositories={repositories}
          onSaved={onCreated}
          onCancel={onBack}
          onError={onError}
        />
      </div>
    );
  }

  const state = routineState(item);
  const r = item.latest;
  const profile = settings?.profiles.find((p) => p.id === routine.profile_id);
  const host = settings?.hosts.find(
    (h) => (h.kind === "local" ? "" : h.id) === routine.host_id,
  );
  const running = r?.state === "working";

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <header className="flex flex-col gap-1.5 border-b bg-header px-5 py-3">
        {crumb}
        <div className="flex flex-wrap items-center gap-2">
          <h1 className="m-0 min-w-0 flex-1 truncate text-[17px] font-semibold">
            {routine.name}
          </h1>
          <button
            type="button"
            className="btn btn-primary"
            disabled={running || busy === "run"}
            onClick={() => void act("run", () => ipc.runRoutineNow(routine.id))}
          >
            {running ? "Running…" : "Run Now"}
          </button>
          <div className="relative">
            <button
              type="button"
              className="btn"
              aria-label="More"
              aria-expanded={menu}
              onClick={() => setMenu(!menu)}
            >
              ⋯
            </button>
            {menu && (
              <div className="absolute right-0 z-10 mt-1 w-44 rounded-md border bg-header p-1 shadow-lg">
                <button
                  type="button"
                  className="btn btn-ghost w-full justify-start text-conflict"
                  onClick={() => {
                    setMenu(false);
                    setConfirmDelete(true);
                  }}
                >
                  Delete…
                </button>
              </div>
            )}
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[12.5px]">
          <StatePill tone={state.tone}>{state.word}</StatePill>
          {r && (
            <span className="text-muted">
              {relativeTime(r.finished_at ?? r.started_at)}
              {r.duration_secs != null &&
                ` · ${Math.max(1, Math.round(r.duration_secs / 60))} min`}
            </span>
          )}
          {r?.run_id && (
            <button
              type="button"
              className="btn btn-sm btn-ghost p-0 text-link"
              onClick={() => r.run_id && onOpenRun(r.run_id)}
            >
              Open the run
            </button>
          )}
        </div>
      </header>

      {confirmDelete && (
        <div
          role="alertdialog"
          aria-label="Delete routine"
          className="m-4 flex flex-col gap-2 rounded-md border border-conflict bg-panel p-3"
        >
          <span>
            Delete <strong>{routine.name}</strong>? Its briefings and memory go
            with it. The runs it started stay in Runs.
          </span>
          <div className="flex gap-2">
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => setConfirmDelete(false)}
            >
              Cancel
            </button>
            <button
              type="button"
              className="btn btn-sm text-conflict"
              onClick={() =>
                void act("delete", async () => {
                  await ipc.deleteRoutine(routine.id);
                  onDeleted();
                })
              }
            >
              Delete Routine
            </button>
          </div>
        </div>
      )}

      {item.schedule?.state === "needs_approval" && (
        <Banner tone="warn">
          <strong>Needs approval.</strong> {item.schedule.changed.join(", ")}{" "}
          changed since it was approved, so the schedule is off.
          <button
            type="button"
            className="btn btn-sm ml-2"
            onClick={() => onTab("schedule")}
          >
            Review and approve…
          </button>
        </Banner>
      )}
      {item.schedule?.state === "overdue" && (
        <Banner tone="bad">
          <strong>Overdue.</strong> {item.host_name} has not answered since a
          run was due
          {item.schedule.synced_at
            ? ` (last heard ${relativeTime(item.schedule.synced_at)})`
            : ""}
          . It may be off or unreachable; nothing here says it ran.
        </Banner>
      )}
      {item.schedule?.notice && (
        <Banner tone="warn">{item.schedule.notice}</Banner>
      )}
      <div className="flex gap-2 border-b bg-panel px-5 py-2 text-[12.5px] text-muted">
        <span aria-hidden="true">⚠</span>
        <span>
          Sends <strong className="text-fg">{item.repository_name}</strong>'s{" "}
          {routine.start} commit and the prompt to{" "}
          <strong className="text-fg">
            {profile ? providerName(profile.provider) : "the agent's provider"}
          </strong>{" "}
          from{" "}
          <strong className="text-fg">{host ? host.name : "this Mac"}</strong>.
          The network is unrestricted.
        </span>
      </div>

      <div className="flex gap-0.5 border-b px-4" role="tablist">
        {TABS.map(([key, label]) => (
          <button
            key={key}
            type="button"
            role="tab"
            aria-selected={tab === key}
            className={`border-0 border-b-2 bg-transparent px-3 py-2 font-[inherit] ${
              tab === key
                ? "border-accent font-medium text-fg"
                : "border-transparent text-muted"
            }`}
            onClick={() => onTab(key)}
          >
            {label}
          </button>
        ))}
      </div>

      {tab === "latest" && (
        <Briefing
          routineId={routine.id}
          result={r}
          busy={busy}
          act={act}
          onOpenRun={onOpenRun}
          onOpenNote={onOpenNote}
        />
      )}
      {tab === "history" && (
        <History
          routineId={routine.id}
          previous={routine.previous_results}
          latestId={r?.id ?? null}
          busy={busy}
          act={act}
          onOpenRun={onOpenRun}
          onOpenNote={onOpenNote}
        />
      )}
      {tab === "schedule" && (
        <ScheduleTab item={item} onChanged={onChanged} onError={onError} />
      )}
      {tab === "definition" && (
        <RoutineForm
          key={routine.version}
          routine={routine}
          repositories={repositories}
          onSaved={() => onChanged()}
          onError={onError}
        />
      )}
    </div>
  );
}

type Act = (key: string, action: () => Promise<unknown>) => Promise<void>;

function Briefing({
  routineId: _routineId,
  result,
  busy,
  act,
  onOpenRun,
  onOpenNote,
}: {
  routineId: string;
  result: BriefingResult | null;
  busy: string | null;
  act: Act;
  onOpenRun: (runId: string) => void;
  onOpenNote: (noteId: string) => void;
}) {
  return (
    <BriefingBody
      result={result}
      busy={busy}
      onTask={(key) =>
        result &&
        void act(`task-${result.id}-${key}`, () =>
          ipc.createBriefingTask(result.id, key),
        )
      }
      onNote={(key) =>
        result &&
        void act(`proposal-note-${result.id}-${key}`, async () => {
          const note = await ipc.createBriefingNote(result.id, key);
          onOpenNote(note.id);
        })
      }
      onSaveNote={() =>
        result &&
        void act(`note-${result.id}`, async () => {
          const note = await ipc.saveBriefingAsNote(result.id);
          onOpenNote(note.id);
        })
      }
      onOpenRun={onOpenRun}
    />
  );
}

function History({
  routineId,
  previous,
  latestId,
  busy,
  act,
  onOpenRun,
  onOpenNote,
}: {
  routineId: string;
  previous: number;
  latestId: string | null;
  busy: string | null;
  act: Act;
  onOpenRun: (runId: string) => void;
  onOpenNote: (noteId: string) => void;
}) {
  const [results, setResults] = useState<BriefingResult[] | null>(null);
  const [selected, setSelected] = useState(0);
  const load = useCallback(async () => {
    try {
      setResults(await ipc.listRoutineResults(routineId));
    } catch {
      setResults([]);
    }
  }, [routineId]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new latest briefing reloads the list.
  useEffect(() => {
    void load();
  }, [load, latestId]);
  if (results === null) return <p className="m-0 px-5 py-4 text-muted">…</p>;
  if (results.length === 0)
    return <p className="m-0 px-5 py-4 text-muted">No briefings yet.</p>;
  const index = Math.min(selected, results.length - 1);
  const current = results[index];
  return (
    <div className="grid min-h-0 grid-cols-[minmax(180px,250px)_minmax(0,1fr)]">
      <ul aria-label="Briefings" className="m-0 list-none border-r p-0">
        {results.map((x, i) => (
          <li key={x.id}>
            <button
              type="button"
              aria-pressed={i === index}
              className="flex w-full flex-col gap-0.5 border-0 border-b bg-transparent px-4 py-2 text-left font-[inherit] hover:bg-panel aria-pressed:bg-accent-soft"
              onClick={() => setSelected(i)}
            >
              <span>{relativeTime(x.finished_at ?? x.started_at)}</span>
              <span className="text-[12px] text-muted">
                {resultWord(x)}
                {x.duration_secs != null &&
                  ` · ${Math.max(1, Math.round(x.duration_secs / 60))} min`}
              </span>
            </button>
          </li>
        ))}
      </ul>
      <div className="min-w-0">
        <BriefingBody
          result={current}
          busy={busy}
          onTask={(key) =>
            void act(`task-${current.id}-${key}`, async () => {
              await ipc.createBriefingTask(current.id, key);
              await load();
            })
          }
          onNote={(key) =>
            void act(`proposal-note-${current.id}-${key}`, async () => {
              const note = await ipc.createBriefingNote(current.id, key);
              onOpenNote(note.id);
              await load();
            })
          }
          onSaveNote={() =>
            void act(`note-${current.id}`, async () => {
              const note = await ipc.saveBriefingAsNote(current.id);
              onOpenNote(note.id);
            })
          }
          onOpenRun={onOpenRun}
        />
        <p className="m-0 px-5 pb-4 text-[12px] text-muted">
          {index < previous
            ? `A run started now would be shown this briefing (the last ${previous} are shown).`
            : "A run started now would not be shown this briefing."}
        </p>
      </div>
    </div>
  );
}

function Banner({
  tone,
  children,
}: {
  tone: "warn" | "bad";
  children: React.ReactNode;
}) {
  return (
    <div
      role="status"
      className={`mx-5 mt-3 rounded-md px-3 py-2 ${tone === "bad" ? "bg-conflict/10 text-conflict" : "bg-dirty/10 text-dirty"}`}
    >
      <span className="text-fg">{children}</span>
    </div>
  );
}
