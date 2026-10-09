import { ask, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  activityLabel,
  activityTone,
  agentName,
  agoLabel,
  costLabel,
  destinationLabel,
  durationLabel,
  foldTurns,
  GROUP_LABEL,
  groupOf,
  hostLabel,
  isRemote,
  keyLabel,
  leftOutUndecided,
  listResult,
  listWhen,
  modelLabel,
  type PermissionState,
  permissionHeading,
  producedLabel,
  producedTone,
  type RunGroup,
  type RunTone,
  reportedCost,
  samePreview,
  shortClock,
  spanLabel,
  startSteps,
  type ToolState,
  type Turn,
  timeLeft,
  toolVerb,
  turnSummary,
  visibilityLabel,
} from "../lib/agentRuns";
import { editCounts, editLines, workspacePath } from "../lib/editDiff";
import { clock, useNow } from "../lib/hostJobs";
import {
  type AgentKind,
  type AgentRun,
  type AppSnapshot,
  type CommitFile,
  type DiffResult,
  errorMessage,
  ipc,
  onAgentRunChanged,
  type RunEvent,
  type RunFileDiff,
  type RunPreview,
  subscribe,
} from "../lib/ipc";
import { KIND_LETTER, kindTone, plural, splitPath } from "../lib/repo";
import DiffView from "./DiffView";
import { StepIcon } from "./HostJobView";
import {
  CheckIcon,
  ChevronDown,
  ChevronRight,
  CircleIcon,
  ClockIcon,
  CrossIcon,
  PlusIcon,
  ProgressIcon,
  TerminalIcon,
} from "./icons";
import { Markdown } from "./Markdown";
import { RepoChip } from "./RepoChip";
import { OrderSwitch, useExplainedPatch } from "./useExplainedPatch";

type Props = {
  snapshot: AppSnapshot;
  runs: AgentRun[] | null;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onNewRun: () => void;
  onOpenRepo: (id: string) => void;
  onOpenSettings: () => void;
  onError: (text: string) => void;
  onNotice: (text: string) => void;
};

const GROUPS: RunGroup[] = ["needs_you", "review", "active", "ended"];

/** The list's Show filter: Ended covers runs ready to review too. */
type Show = "all" | "needs_you" | "active" | "ended";
const SHOWN: Record<Show, RunGroup[]> = {
  all: GROUPS,
  needs_you: ["needs_you"],
  active: ["active"],
  ended: ["review", "ended"],
};

/** Runs (SPEC.md, The run): the list, or one run. */
export default function RunsView(props: Props) {
  const { runs, selectedId } = props;
  const selected = runs?.find((r) => r.id === selectedId) ?? null;
  if (selectedId && selected) {
    return (
      <RunView
        key={selected.id}
        run={selected}
        snapshot={props.snapshot}
        onBack={() => props.onSelect(null)}
        onOpenRepo={props.onOpenRepo}
        onOpenSettings={props.onOpenSettings}
        onError={props.onError}
        onNotice={props.onNotice}
      />
    );
  }
  return <RunList {...props} />;
}

/** A run's state in words, tinted by tone; a dot while the agent is live. */
function StatePill({
  tone,
  dot = false,
  children,
}: {
  tone: RunTone;
  dot?: boolean;
  children: React.ReactNode;
}) {
  return (
    <span className="state-pill" data-tone={tone}>
      {dot && <span className="dot" aria-hidden="true" />}
      {children}
    </span>
  );
}

function RunList({ snapshot, runs, onSelect, onNewRun }: Props) {
  const [show, setShow] = useState<Show>("all");
  const [repoFilter, setRepoFilter] = useState<string>("");
  const [hostFilter, setHostFilter] = useState<string>("");
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);
  const repos = useMemo(() => {
    const seen = new Map<string, string>();
    for (const r of runs ?? []) seen.set(r.repository_id, r.repository_name);
    return [...seen.entries()].sort((a, b) => a[1].localeCompare(b[1]));
  }, [runs]);
  const hosts = useMemo(() => {
    const seen = new Map<string, string>();
    for (const r of runs ?? [])
      seen.set(isRemote(r) ? r.host_id : "local", hostLabel(r));
    return [...seen.entries()];
  }, [runs]);
  const filtered = (runs ?? []).filter(
    (r) =>
      (!repoFilter || r.repository_id === repoFilter) &&
      (!hostFilter || (isRemote(r) ? r.host_id : "local") === hostFilter),
  );
  const needing = filtered.filter((r) => groupOf(r) === "needs_you").length;
  const shown = filtered.filter((r) => SHOWN[show].includes(groupOf(r)));
  const byId = new Map(snapshot.repositories.map((r) => [r.id, r]));
  const filters: Array<[Show, string]> = [
    ["all", "All"],
    ["needs_you", "Needs you"],
    ["active", "Active"],
    ["ended", "Ended"],
  ];
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex min-h-12 shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b px-4 py-2 pl-lead">
        <h1 className="m-0 text-[15px] font-semibold">Runs</h1>
        {(runs?.length ?? 0) > 0 && (
          <fieldset className="seg seg-sm m-0 border-0" aria-label="Show">
            {filters.map(([value, label]) => (
              <button
                key={value}
                type="button"
                aria-pressed={show === value}
                onClick={() => setShow(value)}
              >
                {label}
                {value === "needs_you" && needing > 0 && (
                  <>
                    {" "}
                    <span className="tabular text-muted">{needing}</span>
                  </>
                )}
              </button>
            ))}
          </fieldset>
        )}
        {repos.length > 1 && (
          <select
            className="field h-7 text-[12.5px]"
            aria-label="Repository"
            value={repoFilter}
            onChange={(e) => setRepoFilter(e.target.value)}
          >
            <option value="">All repositories</option>
            {repos.map(([id, name]) => (
              <option key={id} value={id}>
                {name}
              </option>
            ))}
          </select>
        )}
        {hosts.length > 1 && (
          <select
            className="field h-7 text-[12.5px]"
            aria-label="Host"
            value={hostFilter}
            onChange={(e) => setHostFilter(e.target.value)}
          >
            <option value="">All hosts</option>
            {hosts.map(([id, name]) => (
              <option key={id} value={id}>
                {name}
              </option>
            ))}
          </select>
        )}
        <span className="flex-1" />
        <button
          type="button"
          className="btn btn-sm"
          onClick={onNewRun}
          title="New run (⌥⌘N)"
        >
          <PlusIcon size={12} /> New run…
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {runs === null ? (
          <p className="text-muted">Loading…</p>
        ) : runs.length === 0 ? (
          <div className="flex max-w-[560px] flex-col items-start gap-2 py-8 text-[13px] text-fg-2">
            <p className="m-0">
              No runs yet. A run hands a commit of one of your repositories to a
              coding agent in a container. Its work comes back as changes you
              review before anything leaves Brainiac.
            </p>
            <button type="button" className="btn btn-sm" onClick={onNewRun}>
              New run… <span className="kbd">⌥⌘N</span>
            </button>
          </div>
        ) : (
          <>
            <div className="rounded-[10px] border">
              <div
                className="run-grid section-label border-b bg-header px-3 py-1.5"
                aria-hidden="true"
              >
                <span>Run</span>
                <span>Now</span>
                <span>Result</span>
                <span>When</span>
              </div>
              {shown.length === 0 && (
                <p className="m-0 px-3 py-4 text-[12.5px] text-muted">
                  No runs here.
                </p>
              )}
              {GROUPS.map((group) => {
                const members = shown.filter((r) => groupOf(r) === group);
                if (members.length === 0) return null;
                return (
                  <section key={group} aria-label={GROUP_LABEL[group]}>
                    <h2 className="m-0 border-b bg-panel px-3 py-1 text-[11.5px] font-semibold text-fg-2">
                      {GROUP_LABEL[group]}
                      <span className="ml-2 font-normal text-muted">
                        {members.length}
                      </span>
                    </h2>
                    <ul className="m-0 list-none p-0">
                      {members.map((run) => (
                        <li key={run.id} className="border-b last:border-b-0">
                          <RunRow
                            run={run}
                            now={now}
                            repo={byId.get(run.repository_id)}
                            onOpen={() => onSelect(run.id)}
                          />
                        </li>
                      ))}
                    </ul>
                  </section>
                );
              })}
            </div>
            <p className="m-0 mt-3 text-[12px] text-muted">
              Ended runs are removed 30 days after they end. Work that waits for
              your decision stays until you decide.
            </p>
          </>
        )}
      </div>
    </div>
  );
}

function RunRow({
  run,
  now,
  repo,
  onOpen,
}: {
  run: AgentRun;
  now: number;
  repo: AppSnapshot["repositories"][number] | undefined;
  onOpen: () => void;
}) {
  const result = listResult(run);
  const live = run.phase !== "ended";
  const label = live && !run.connected ? visibilityLabel(run) : null;
  return (
    <button
      type="button"
      className="run-grid w-full px-3 py-2 text-left hover:bg-panel"
      onClick={onOpen}
    >
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="truncate font-medium">{run.title}</span>
        <span className="flex min-w-0 items-center gap-1.5 text-[11.5px] text-muted">
          <RepoChip repo={repo} fallbackName={run.repository_name} />
          <span className="truncate">
            · {agentName(run.agent)} on {hostLabel(run)}
          </span>
        </span>
      </span>
      <span className="flex min-w-0 flex-wrap items-center gap-1">
        <StatePill tone={activityTone(run)} dot={live && run.connected}>
          {label ?? activityLabel(run)}
        </StatePill>
        {run.cancel_requested && (
          <StatePill tone="red">Cancel requested</StatePill>
        )}
      </span>
      <span
        className={`min-w-0 truncate text-[12px] ${
          run.collection === "ready"
            ? "text-clean"
            : run.collection === "failed" ||
                (run.kept && run.collection === "none" && !live)
              ? "text-conflict"
              : "text-fg-2"
        }`}
      >
        {result ?? "—"}
      </span>
      <span className="min-w-0 truncate text-[12px] text-muted">
        {listWhen(run, now)}
      </span>
    </button>
  );
}

type Tab = "conversation" | "changes";

function RunView({
  run: initial,
  snapshot,
  onBack,
  onOpenRepo,
  onOpenSettings,
  onError,
  onNotice,
}: {
  run: AgentRun;
  snapshot: AppSnapshot;
  onBack: () => void;
  onOpenRepo: (id: string) => void;
  onOpenSettings: () => void;
  onError: (text: string) => void;
  onNotice: (text: string) => void;
}) {
  const [run, setRun] = useState(initial);
  const [events, setEvents] = useState<RunEvent[]>([]);
  const [tab, setTab] = useState<Tab>("conversation");
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(Date.now());
  const cursor = useRef(0);
  /** One read of the mirror at a time; a change during it reads again. */
  const reading = useRef<Promise<void> | null>(null);
  const again = useRef(false);

  // The list's copy moves on with the backend's events; this view's does
  // too, unless an action here already got something newer.
  useEffect(() => {
    setRun((current) =>
      initial.version >= current.version ? initial : current,
    );
  }, [initial]);

  const loadEvents = useCallback(() => {
    if (reading.current) {
      again.current = true;
      return reading.current;
    }
    const read = (async () => {
      try {
        for (;;) {
          const page = await ipc.listRunEvents(initial.id, cursor.current);
          const fresh = page.events.filter((e) => e.seq > cursor.current);
          if (fresh.length === 0) break;
          cursor.current = fresh[fresh.length - 1].seq;
          setEvents((old) => [...old, ...fresh]);
          if (cursor.current >= page.cursor) break;
        }
      } catch (e) {
        onError(errorMessage(e));
      } finally {
        reading.current = null;
        if (again.current) {
          again.current = false;
          void loadEvents();
        }
      }
    })();
    reading.current = read;
    return read;
  }, [initial.id, onError]);

  useEffect(() => {
    void loadEvents();
    return subscribe(
      onAgentRunChanged((e) => {
        if (e.run_id !== initial.id || e.deleted) return;
        void loadEvents();
        ipc.getAgentRun(initial.id).then(setRun, () => {});
      }),
    );
  }, [initial.id, loadEvents]);

  // A live run's clocks ("last update 3 s ago", the time left) move on.
  useEffect(() => {
    if (run.phase === "ended") return;
    const timer = setInterval(() => setNow(Date.now()), 5_000);
    return () => clearInterval(timer);
  }, [run.phase]);

  const act = async (action: () => Promise<AgentRun | undefined>) => {
    setBusy(true);
    try {
      const next = await action();
      if (next) setRun(next);
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const cancel = async () => {
    const message = run.starting
      ? "Cancel this run? It has not reached the agent yet, so nothing is collected."
      : run.connected
        ? "Cancel this run? The agent stops, and its work so far is collected for review."
        : "Cancel this run? Brainiac can't reach it now, so the cancel is sent when it reconnects. Until then, the time limit still ends the run.";
    if (
      !(await ask(message, {
        title: "Cancel Run",
        kind: "warning",
        okLabel: "Cancel Run",
      }))
    )
      return;
    await act(() => ipc.cancelAgentRun(run.id));
  };
  const finish = () => act(() => ipc.finishAgentRun(run.id));
  const collect = (include: string[] = []) =>
    act(() => ipc.collectAgentRun(run.id, include));
  const discard = async () => {
    if (
      !(await ask(
        "Discard the agent's work? Its container and files are removed from the engine. The conversation stays.",
        { title: "Discard Work", kind: "warning", okLabel: "Discard" },
      ))
    )
      return;
    await act(() => ipc.discardAgentRun(run.id));
  };
  const remove = async () => {
    const names = run.left_out.slice(0, 3).map((l) => l.path);
    const more = run.left_out.length - names.length + run.left_out_more;
    const lost =
      run.kept && !run.snapshot_accepted
        ? run.collection === "failed" || run.collection === "none"
          ? " Its uncollected work is lost."
          : names.length > 0
            ? ` ${plural(names.length + more, "left-out file")} never added or accepted ${names.length + more === 1 ? "is" : "are"} lost: ${names.join(", ")}${more > 0 ? `, and ${more} more` : ""}.`
            : ""
        : "";
    if (
      !(await ask(
        `Delete this run? Its conversation, the review, and the stopped container with its files are removed.${lost}`,
        { title: "Delete Run", kind: "warning", okLabel: "Delete Run" },
      ))
    )
      return;
    await act(async () => {
      await ipc.deleteAgentRun(run.id);
      onBack();
      return undefined;
    });
  };
  const copyPatch = async () => {
    try {
      const patch = await ipc.copyRunPatch(run.id);
      await navigator.clipboard?.writeText(patch);
      onNotice("Patch copied");
    } catch (e) {
      onError(errorMessage(e));
    }
  };
  const copyBranchCommand = async () => {
    try {
      const { branch, command } = await ipc.getRunBranchCommand(run.id);
      await navigator.clipboard?.writeText(command);
      onNotice(
        `Command copied. Run it in your repository to get the branch ${branch}.`,
      );
    } catch (e) {
      onError(errorMessage(e));
    }
  };
  const savePatch = async () => {
    try {
      const path = await saveDialog({
        defaultPath: `${run.title.replace(/[^\w.-]+/g, "-").slice(0, 40) || "run"}.patch`,
        filters: [{ name: "Patch", extensions: ["patch", "diff"] }],
      });
      if (!path) return;
      await ipc.saveRunPatch(run.id, path);
      onNotice("Patch saved");
    } catch (e) {
      onError(errorMessage(e));
    }
  };

  const repo = snapshot.repositories.find((r) => r.id === run.repository_id);
  const live = run.phase !== "ended";
  const idle =
    live && (run.activity === "idle" || run.activity === "plan_limit");
  const collected =
    run.collection === "ready" || run.collection === "no_changes";
  const folded = useMemo(() => foldTurns(events), [events]);
  const cost = useMemo(() => reportedCost(events), [events]);
  // An explain run is its explanation's How it was written: read only.
  const explain = run.explain;
  const reported = useMemo(() => reportedFiles(folded.turns), [folded]);
  const finishTitle = !run.connected
    ? "Possible once Brainiac reconnects"
    : idle
      ? "Stop the agent and collect its work"
      : "Available when the turn ends";

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex shrink-0 flex-col border-b bg-header px-5 pt-3 pl-lead">
        <div className="flex flex-wrap items-start gap-x-6 gap-y-2 pb-2">
          <div className="flex min-w-0 flex-[999_1_380px] flex-col gap-1">
            <nav
              aria-label="Breadcrumb"
              className="flex items-center gap-1 text-[12px] text-muted"
            >
              <button
                type="button"
                className="text-muted hover:text-fg"
                onClick={onBack}
              >
                {explain ? "Back" : "Runs"}
              </button>
              <span aria-hidden="true">›</span>
              <span className="truncate">{run.repository_name}</span>
            </nav>
            <h1 className="selectable m-0 text-[17px] font-semibold leading-snug">
              {explain ? `How it was written: ${run.title}` : run.title}
            </h1>
            <div className="flex flex-wrap items-center gap-x-3.5 gap-y-1 text-[12px] text-fg-2">
              <RepoChip
                repo={repo}
                fallbackName={run.repository_name}
                onClick={repo ? () => onOpenRepo(repo.id) : undefined}
              />
              <span>
                from{" "}
                <code className="mono" title={run.start_commit}>
                  {run.start_commit.slice(0, 7)}
                </code>
              </span>
              <span>
                {agentName(run.agent)} on {hostLabel(run)} · {modelLabel(run)}
              </span>
              <span>
                {run.permissions === "ask"
                  ? "Asks before actions"
                  : "Acts without asking"}
              </span>
            </div>
          </div>
          <div className="flex flex-[1_1_auto] flex-wrap items-center justify-end gap-x-2.5 gap-y-2">
            {live ? (
              <LiveStatus run={run} now={now} cost={cost} />
            ) : (
              <EndedStatus run={run} turns={folded.turns.length} cost={cost} />
            )}
            {live && !explain && (
              <button
                type="button"
                className="btn btn-sm text-conflict"
                disabled={busy || run.cancel_requested}
                onClick={() => void cancel()}
              >
                {run.cancel_requested ? "Cancel requested" : "Cancel run…"}
              </button>
            )}
            {live && !explain && (
              <button
                type="button"
                className="btn btn-sm btn-primary"
                disabled={busy || !idle || !run.connected}
                title={finishTitle}
                onClick={() => void finish()}
              >
                Finish and collect
              </button>
            )}
            {!live && !explain && run.collection === "ready" && (
              <>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => void copyPatch()}
                >
                  Copy patch
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => void savePatch()}
                >
                  Save patch…
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  title="A Git command you run in your repository: it fetches the snapshot as a new branch. Brainiac doesn't run it."
                  onClick={() => void copyBranchCommand()}
                >
                  Copy branch command
                </button>
              </>
            )}
            {!live && !explain && (
              <button
                type="button"
                className="btn btn-sm text-conflict"
                disabled={busy || !run.stop_confirmed}
                title={
                  run.stop_confirmed
                    ? undefined
                    : "Possible once the engine confirms the run stopped"
                }
                onClick={() => void remove()}
              >
                Delete run…
              </button>
            )}
          </div>
        </div>
        <div
          role="tablist"
          aria-label="Run"
          className="-mb-px flex gap-1"
          hidden={explain}
        >
          <button
            type="button"
            role="tab"
            className="tab"
            aria-selected={tab === "conversation"}
            onClick={() => setTab("conversation")}
          >
            Conversation
          </button>
          <button
            type="button"
            role="tab"
            className="tab"
            aria-selected={tab === "changes"}
            disabled={!collected && !live}
            title={
              collected
                ? undefined
                : live
                  ? "The agent's working tree so far, read while it works"
                  : "After the work is collected"
            }
            onClick={() => setTab("changes")}
          >
            {collected || !live ? "Changes" : "Changes so far"}
            {run.collection === "ready" && run.changed_files != null && (
              <span className="rounded-lg bg-control px-1.5 text-[11px] font-normal text-fg-2">
                {run.changed_files}
              </span>
            )}
          </button>
        </div>
      </header>
      {live && !run.connected && <AwayBanner run={run} now={now} />}
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          {!explain && (
            <RunCards
              run={run}
              busy={busy}
              onCollect={() => void collect()}
              onDiscard={() => void discard()}
              onRetryCleanup={() => void act(() => ipc.retryRunCleanup(run.id))}
              onOpenSettings={onOpenSettings}
            />
          )}
          {tab === "changes" && !collected && live && !explain ? (
            <ChangesSoFar run={run} onError={onError} onNotice={onNotice} />
          ) : tab === "conversation" || !collected || explain ? (
            <Conversation
              run={run}
              readOnly={explain}
              turns={folded.turns}
              notices={folded.notices}
              ended={folded.ended}
              busy={busy}
              now={now}
              onPermit={(id, allow) =>
                act(() => ipc.answerRunPermission(run.id, id, allow))
              }
              onPrompt={(text) => act(() => ipc.sendRunPrompt(run.id, text))}
            />
          ) : (
            <Changes
              run={run}
              busy={busy}
              onCollect={collect}
              onAccept={() => act(() => ipc.acceptRunSnapshot(run.id))}
              onError={onError}
              onNotice={onNotice}
            />
          )}
        </div>
        <RunDetails run={run} reported={reported} />
      </div>
    </div>
  );
}

/** A live run's header: what the agent is doing, when it ends, what it costs. */
function LiveStatus({
  run,
  now,
  cost,
}: {
  run: AgentRun;
  now: number;
  cost: ReturnType<typeof reportedCost>;
}) {
  const label = run.connected ? activityLabel(run) : visibilityLabel(run);
  return (
    <>
      <StatePill tone={activityTone(run)} dot={run.connected}>
        {label}
      </StatePill>
      {run.cancel_requested && (
        <StatePill tone="red">Cancel requested</StatePill>
      )}
      {run.deadline_at && (
        <span
          className="inline-flex items-center gap-1 text-[12px] text-fg-2"
          title="The time limit counts idle and waiting time too"
        >
          <ClockIcon size={13} />
          Ends {shortClock(run.deadline_at, now)} ·{" "}
          {timeLeft(run.deadline_at, now)}
        </span>
      )}
      <span className="text-[12px] text-fg-2">
        {costLabel(run.payment, cost)}
      </span>
    </>
  );
}

/** An ended run's header: how it ended, what it produced, and how long it took. */
function EndedStatus({
  run,
  turns,
  cost,
}: {
  run: AgentRun;
  turns: number;
  cost: ReturnType<typeof reportedCost>;
}) {
  const produced = producedLabel(run);
  const visibility = visibilityLabel(run);
  const took =
    run.accepted_at && run.ended_at
      ? spanLabel(
          new Date(run.ended_at).getTime() -
            new Date(run.accepted_at).getTime(),
        )
      : null;
  return (
    <>
      <StatePill tone={activityTone(run)}>
        {activityLabel(run)}
        {run.ended_at ? ` ${shortClock(run.ended_at)}` : ""}
      </StatePill>
      {produced && <StatePill tone={producedTone(run)}>{produced}</StatePill>}
      {visibility && <StatePill tone="dashed">{visibility}</StatePill>}
      {run.cleanup_pending && (
        <StatePill tone="grey">Cleanup pending</StatePill>
      )}
      <span className="text-[12px] text-fg-2">
        {[
          turns > 0 && plural(turns, "turn"),
          took,
          run.payment === "claude_plan"
            ? "used your Claude plan"
            : costLabel(run.payment, cost),
        ]
          .filter(Boolean)
          .join(" · ")}
      </span>
    </>
  );
}

/** Brainiac cannot reach a live run: its last report, and that it may go on. */
function AwayBanner({ run, now }: { run: AgentRun; now: number }) {
  const remote = isRemote(run);
  const last = run.reported_at
    ? `Last report ${shortClock(run.reported_at, now)}, ${agoLabel(run.reported_at, now)}.`
    : "No report yet.";
  const ends = run.deadline_at ? shortClock(run.deadline_at, now) : null;
  return (
    <div
      role="status"
      className="flex shrink-0 flex-col gap-1 border-b border-dashed bg-panel px-5 py-2.5 pl-lead text-[12.5px] leading-relaxed text-fg-2"
    >
      <p className="m-0">
        <strong className="text-fg">
          {remote
            ? `Can't reach ${run.host_name}.`
            : `Can't reach the run on This Mac.`}
        </strong>{" "}
        {last}{" "}
        {remote
          ? `The run keeps going on ${run.host_name}${
              run.activity === "permission"
                ? ": the request below waits there, unanswered, until you're back"
                : ""
            }${ends ? ` and ends at ${ends} at the latest` : ""}.`
          : "The agent may still be working; below is what it last reported."}{" "}
        Brainiac keeps trying.
      </p>
      {run.cancel_requested && (
        <p className="m-0">
          Cancel is sent when Brainiac reconnects; until then the time limit
          still stops the run.
          {remote &&
            " To stop it without this Mac, use the host's emergency stop in Settings → Agents."}
        </p>
      )}
    </div>
  );
}

/** What an ended run waits on, each with what you can do about it. */
function RunCards({
  run,
  busy,
  onCollect,
  onDiscard,
  onRetryCleanup,
  onOpenSettings,
}: {
  run: AgentRun;
  busy: boolean;
  onCollect: () => void;
  onDiscard: () => void;
  onRetryCleanup: () => void;
  onOpenSettings: () => void;
}) {
  if (run.phase !== "ended") return null;
  const cards: React.ReactNode[] = [];
  const uncollected = run.kept && run.collection === "none";
  const credential = /token|api key|credential|unauthori[sz]ed|401/i.test(
    run.error ?? "",
  );
  const collectButtons = (
    <>
      <button
        type="button"
        className="btn btn-sm btn-primary"
        disabled={busy || !run.stop_confirmed}
        onClick={onCollect}
      >
        Collect work
      </button>
      <button
        type="button"
        className="btn btn-sm text-conflict"
        disabled={busy || !run.stop_confirmed}
        onClick={onDiscard}
      >
        Discard work…
      </button>
    </>
  );
  if (uncollected && run.outcome === "interrupted") {
    cards.push(
      <RunCard
        key="interrupted"
        tone="red"
        badge="Interrupted"
        title={`The run stopped unexpectedly${run.ended_at ? ` at ${shortClock(run.ended_at)}` : ""}`}
      >
        <p className="m-0">
          {run.error ? `${run.error} ` : ""}The container was stopped and kept
          with its files; only Discard removes them. The conversation can't
          continue, and updates after the last one recorded may be missing.
        </p>
        <div className="flex flex-wrap gap-2">{collectButtons}</div>
      </RunCard>,
    );
  } else if (uncollected && run.outcome === "failed") {
    cards.push(
      <RunCard
        key="failed"
        tone="red"
        badge="Failed"
        title={run.error ?? "The run failed"}
      >
        <p className="m-0">
          The agent's work so far is kept, and Brainiac collects it once the
          run's host answers. A run can't take a new token or key
          {credential ? ", so update the credential in Settings and" : ", so"}{" "}
          start a new run.
        </p>
        <div className="flex flex-wrap gap-2">
          {credential && (
            <button
              type="button"
              className="btn btn-sm"
              onClick={onOpenSettings}
            >
              Settings → Agents…
            </button>
          )}
        </div>
      </RunCard>,
    );
  } else if (uncollected && run.stop_confirmed) {
    // Brainiac collects every run that was not interrupted on its own; Collect
    // work here would only race it.
    cards.push(
      <RunCard
        key="kept"
        tone="grey"
        badge="Not collected yet"
        title="Brainiac collects the agent's work"
      >
        <p className="m-0">
          {run.error ? `${run.error} ` : ""}It starts as soon as the run's host
          answers; the stopped container and its files are kept until then.
        </p>
      </RunCard>,
    );
  } else if (
    run.outcome === "failed" &&
    !run.kept &&
    run.stop_confirmed &&
    run.collection === "none"
  ) {
    // It failed before anything of it was made on the engine, or its work
    // was discarded since.
    const image = /\bimage\b/i.test(run.error ?? "");
    cards.push(
      <RunCard
        key="failed"
        tone="red"
        badge="Failed"
        title={run.error ?? "The run failed"}
      >
        <p className="m-0">
          Nothing of this run is on the engine, so there is no work to collect.
          {image || credential ? " Fix this in Settings, then" : " Then"} start
          a new run.
        </p>
        {(image || credential) && (
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              className="btn btn-sm"
              onClick={onOpenSettings}
            >
              Settings → Agents…
            </button>
          </div>
        )}
      </RunCard>,
    );
  } else if (run.error && !run.stop_confirmed) {
    // Shown with the stop card below.
  } else if (run.error && run.collection !== "failed") {
    cards.push(
      <p key="error" className="m-0 text-[12.5px] text-conflict">
        {run.error}
      </p>,
    );
  }
  if (run.collection === "failed") {
    cards.push(
      <RunCard
        key="collection"
        tone="red"
        badge="Collection failed"
        title="Couldn't collect the work"
      >
        <p className="m-0">
          {run.collection_error ?? "The work could not be collected."} Nothing
          was removed from the engine.
        </p>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            className="btn btn-sm btn-primary"
            disabled={busy || !run.kept}
            onClick={onCollect}
          >
            Retry collection
          </button>
          <button
            type="button"
            className="btn btn-sm text-conflict"
            disabled={busy || !run.kept}
            onClick={onDiscard}
          >
            Discard work…
          </button>
        </div>
      </RunCard>,
    );
  }
  if (!run.stop_confirmed) {
    cards.push(
      <RunCard
        key="stop"
        tone="dashed"
        badge="Stop not confirmed"
        title="Brainiac can't confirm the run stopped"
      >
        <p className="m-0">
          {run.error ?? "The engine isn't answering."} Collecting and deleting
          stay off until it does; the run's time limit still applies on the
          engine.
        </p>
      </RunCard>,
    );
  }
  if (run.cleanup_pending) {
    cards.push(
      <RunCard
        key="cleanup"
        tone="grey"
        badge="Cleanup pending"
        title={
          run.collection === "ready" || run.collection === "no_changes"
            ? "The review is ready; cleanup didn't finish"
            : "Cleanup didn't finish"
        }
      >
        <p className="m-0">
          {run.cleanup_pending}
          {run.collection === "ready" || run.collection === "no_changes"
            ? " Your result is safe on this Mac."
            : ""}
        </p>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            className="btn btn-sm"
            disabled={busy}
            onClick={onRetryCleanup}
          >
            Retry cleanup
          </button>
        </div>
      </RunCard>,
    );
  }
  if (cards.length === 0) return null;
  return (
    <div className="flex shrink-0 flex-col gap-2 px-5 pt-3 pl-lead">
      <div className="flex max-w-[720px] flex-col gap-2">{cards}</div>
    </div>
  );
}

function RunCard({
  tone,
  badge,
  title,
  children,
}: {
  tone: RunTone;
  badge: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section
      className="run-card text-[12.5px] leading-relaxed text-fg-2"
      data-tone={tone}
      aria-label={badge}
    >
      <div className="flex flex-wrap items-center gap-2">
        <StatePill tone={tone}>{badge}</StatePill>
        <h2 className="m-0 text-[13px] font-semibold text-fg">{title}</h2>
      </div>
      {children}
    </section>
  );
}

/** The side panel: where the run came from, where it runs, and what it may reach. */
function RunDetails({
  run,
  reported,
}: {
  run: AgentRun;
  reported: Array<{ path: string; letter: string; tone: string }>;
}) {
  const remote = isRemote(run);
  return (
    <aside
      aria-label="Run details"
      className="w-[264px] shrink-0 overflow-y-auto border-l bg-panel px-4 py-4 text-[12px]"
    >
      <Fact label="Started from">
        <span className="mono" title={run.start_commit}>
          {run.start_commit.slice(0, 7)}
        </span>{" "}
        · {run.start_subject}
        <Note>Uncommitted changes in your checkout were not included.</Note>
      </Fact>
      <Fact label="Runs on">
        {remote ? run.host_name : "This Mac"} · {run.engine_name}
        <Note>
          {remote
            ? "Keeps working while this Mac sleeps or Brainiac is closed."
            : "Pauses while this Mac sleeps."}
        </Note>
      </Fact>
      <Fact label="Agent">{agentName(run.agent)}</Fact>
      <Fact label="Code and prompts go to">
        {destinationLabel(run)}
        <Note>
          {run.payment === "claude_plan" ? "Token" : keyLabel(run.provider)}{" "}
          from {run.credential_source}.
        </Note>
      </Fact>
      <Fact label="Network">
        Unrestricted
        <Note>The agent can reach any site.</Note>
      </Fact>
      <Fact label="Model">
        {modelLabel(run)}
        {run.model_used && run.model !== "" && (
          <Note>Asked for {run.model}.</Note>
        )}
        {!run.model_used && run.model !== "" && (
          <Note>Not yet reported by the agent.</Note>
        )}
      </Fact>
      <Fact label="Limits">
        {durationLabel(run.time_limit_minutes)} · {run.cpus} CPU ·{" "}
        {Math.round(run.memory_mib / 1024)} GB memory · {run.workspace_gib} GB
        workspace
      </Fact>
      <Fact label="Image">
        <span className="mono break-all">{run.image_name}</span>
      </Fact>
      <div className="mt-1 flex flex-col gap-1.5 border-t pt-3">
        <span className="section-label">Files the agent says it changed</span>
        {reported.length === 0 ? (
          <span className="text-muted">None reported yet.</span>
        ) : (
          <ul className="m-0 flex list-none flex-col gap-1 p-0">
            {reported.map((f) => (
              <li key={f.path} className="flex min-w-0 items-center gap-2">
                <span className="kind" data-tone={f.tone}>
                  {f.letter}
                </span>
                <span className="mono selectable truncate" title={f.path}>
                  {f.path}
                </span>
              </li>
            ))}
          </ul>
        )}
        <span className="text-muted">
          Reported by the agent. Changes shows its working tree itself.
        </span>
      </div>
    </aside>
  );
}

function Fact({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="mb-3.5 flex flex-col gap-0.5">
      <span className="section-label">{label}</span>
      <span className="selectable leading-snug text-fg">{children}</span>
    </div>
  );
}

function Note({ children }: { children: React.ReactNode }) {
  return <span className="block text-muted">{children}</span>;
}

/** Plan entries with a key each: the content, numbered when it repeats. */
function withKeys<T extends { content: string }>(
  items: T[],
): Array<[string, T]> {
  const seen = new Map<string, number>();
  return items.map((item) => {
    const n = (seen.get(item.content) ?? 0) + 1;
    seen.set(item.content, n);
    return [n === 1 ? item.content : `${item.content}#${n}`, item];
  });
}

/** Paths of the agent's edit tools, in order, once each, with a change letter. */
function reportedFiles(
  turns: Turn[],
): Array<{ path: string; letter: string; tone: string }> {
  const seen = new Map<
    string,
    { path: string; letter: string; tone: string }
  >();
  for (const t of turns)
    for (const tool of t.tools) {
      const kind =
        tool.kind === "edit"
          ? { letter: "M", tone: "mod" }
          : tool.kind === "delete"
            ? { letter: "D", tone: "del" }
            : tool.kind === "move"
              ? { letter: "R", tone: "ren" }
              : null;
      if (kind)
        for (const location of tool.locations) {
          const path = workspacePath(location);
          seen.set(path, { path, ...kind });
        }
    }
  return [...seen.values()];
}

/**
 * A run that has not reached the agent yet (SPEC.md, The run: Starting):
 * Brainiac's steps, then the controller's, with the time since Start run.
 */
function RunStarting({ run }: { run: AgentRun }) {
  const now = useNow(true);
  const steps = startSteps(run);
  const elapsed = clock((now - Date.parse(run.created_at)) / 1000);
  return (
    <section
      aria-label="Starting the run"
      className="flex flex-col gap-3 rounded-lg border bg-app px-4 py-3.5 text-[12.5px]"
    >
      <div className="flex items-baseline gap-2">
        <span className="font-semibold">Starting on {hostLabel(run)}</span>
        <span className="mono text-[12px] text-muted">{elapsed}</span>
      </div>
      <ol className="m-0 flex list-none flex-col gap-2 p-0">
        {steps.map((step) => (
          <li
            key={step.label}
            aria-current={step.state === "running" ? "step" : undefined}
            className={`flex items-center gap-2 ${
              step.state === "waiting" ? "text-muted" : ""
            }`}
          >
            <StepIcon state={step.state} />
            {step.label}
          </li>
        ))}
      </ol>
      <p className="m-0 text-[12px] text-muted">
        Nothing has reached the provider yet.
        {isRemote(run) &&
          " Copying a large repository to a remote host can take a few minutes."}{" "}
        You can leave this page; the run keeps starting.
      </p>
    </section>
  );
}

function Conversation({
  run,
  readOnly = false,
  turns,
  notices,
  ended,
  busy,
  now,
  onPermit,
  onPrompt,
}: {
  run: AgentRun;
  /** An explain run's conversation: no prompt box, no answers. */
  readOnly?: boolean;
  turns: Turn[];
  notices: string[];
  ended: { outcome: string; message: string | null } | null;
  busy: boolean;
  now: number;
  onPermit: (id: string, allow: boolean) => Promise<void>;
  onPrompt: (text: string) => Promise<void>;
}) {
  const [text, setText] = useState("");
  const bottom = useRef<HTMLDivElement>(null);
  const last = turns[turns.length - 1];
  // biome-ignore lint/correctness/useExhaustiveDependencies: new events scroll the latest into view.
  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end" });
  }, [
    turns.length,
    last?.message.length,
    last?.tools.length,
    run.pending_permissions.length,
  ]);
  const pending = new Set(run.pending_permissions.map((p) => p.permission_id));
  const live = run.phase !== "ended";
  const canSend =
    live &&
    run.connected &&
    (run.activity === "idle" || run.activity === "plan_limit");
  const working =
    live && run.connected && run.activity === "working" && !last?.ended;
  const send = async () => {
    const prompt = text.trim();
    if (!prompt || !canSend) return;
    await onPrompt(prompt);
    setText("");
  };
  const placeholder = !run.connected
    ? "You can send prompts again once Brainiac reconnects."
    : run.activity === "permission"
      ? "Answer the request above first."
      : run.activity === "idle"
        ? "Ask for another change…"
        : run.activity === "plan_limit"
          ? "Continue where it stopped…"
          : "You can send the next prompt when this turn ends.";
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4 pl-lead">
        <div className="flex max-w-[720px] flex-col gap-4">
          {live && run.activity === "preparing" && turns.length === 0 && (
            <RunStarting run={run} />
          )}
          {notices.map((n) => (
            <p key={n} className="m-0 text-[12px] text-muted">
              {n}
            </p>
          ))}
          {turns.map((t) => (
            <TurnView
              key={t.turn}
              turn={t}
              agent={run.agent}
              latest={t === last}
              pending={pending}
              busy={busy}
              connected={run.connected}
              deadline={run.deadline_at}
              onPermit={onPermit}
            />
          ))}
          {working && last && (
            <p className="m-0 flex items-center gap-1.5 text-[12px] text-muted">
              <ProgressIcon
                size={13}
                className="text-accent motion-safe:animate-spin"
              />
              {agentName(run.agent)} is working
              {last.lastAt
                ? ` · last update ${agoLabel(last.lastAt, now)}`
                : ""}
            </p>
          )}
          {live && run.activity === "idle" && last?.ended && (
            <Divider>
              Turn {last.turn} ended {shortClock(last.ended.at, now)} ·{" "}
              {turnSummary(last)}
            </Divider>
          )}
          {live && run.activity === "plan_limit" && (
            <section
              className="run-card text-[12.5px] leading-relaxed text-fg-2"
              data-tone="amber"
              aria-labelledby="plan-limit"
            >
              <h3
                id="plan-limit"
                className="m-0 text-[13px] font-semibold text-fg"
              >
                {run.payment === "claude_plan"
                  ? "Your Claude plan's usage limit stopped this turn"
                  : "A usage limit stopped this turn"}
              </h3>
              <p className="m-0">
                {run.payment === "claude_plan"
                  ? "Runs share limits with Claude Code in your terminal and on claude.ai, which shows when they reset. "
                  : ""}
                The session stays open and its work is kept. The time limit
                keeps running
                {run.deadline_at
                  ? `: the run ends at ${shortClock(run.deadline_at, now)}`
                  : ""}
                .
              </p>
            </section>
          )}
          {ended && (
            <Divider>
              {activityLabel(run)}
              {run.ended_at ? ` ${shortClock(run.ended_at, now)}` : ""}
              {ended.message ? ` · ${ended.message}` : ""}
            </Divider>
          )}
          <div ref={bottom} />
        </div>
      </div>
      {live && !readOnly && (
        <div className="shrink-0 border-t px-5 pt-2.5 pb-3.5 pl-lead">
          <div className="flex max-w-[720px] flex-col gap-1.5">
            <label
              htmlFor={`prompt-${run.id}`}
              className="text-[12px] text-muted"
            >
              Next prompt
            </label>
            <div className="flex items-end gap-2">
              <textarea
                id={`prompt-${run.id}`}
                className="field min-h-[56px] flex-1 resize-y text-[13px]"
                rows={canSend ? 3 : 2}
                placeholder={placeholder}
                value={text}
                disabled={!canSend || busy}
                onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && e.metaKey) {
                    e.preventDefault();
                    void send();
                  }
                }}
              />
              <button
                type="button"
                className="btn btn-primary"
                disabled={!canSend || busy || !text.trim()}
                title="Send (⌘↩)"
                onClick={() => void send()}
              >
                Send <span className="opacity-80">⌘↩</span>
              </button>
            </div>
            {canSend && (
              <p className="m-0 text-[12px] text-muted">
                {run.activity === "plan_limit"
                  ? "Sending before the limit resets stops again right away. Or finish and collect what's done."
                  : "Finish and collect stops the agent and snapshots its work for review. You can't send more prompts after that."}
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function Divider({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-3 text-[12px] text-muted">
      <span className="h-px flex-1 bg-line" aria-hidden="true" />
      <span className="text-center">{children}</span>
      <span className="h-px flex-1 bg-line" aria-hidden="true" />
    </div>
  );
}

/**
 * One turn: the prompt, the agent's steps (folded to a summary once the
 * turn ended, unless it is the latest), its questions, and its reply.
 */
function TurnView({
  turn: t,
  agent,
  latest,
  pending,
  busy,
  connected,
  deadline,
  onPermit,
}: {
  turn: Turn;
  agent: AgentKind;
  latest: boolean;
  pending: Set<string>;
  busy: boolean;
  connected: boolean;
  deadline: string | null;
  onPermit: (id: string, allow: boolean) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [thinking, setThinking] = useState(false);
  const waiting = t.permissions.filter((p) => pending.has(p.permission_id));
  const answered = t.permissions.filter((p) => !pending.has(p.permission_id));
  const hasSteps = t.tools.length > 0 || answered.length > 0 || !!t.thought;
  const foldable = !!t.ended && hasSteps;
  const showSteps = !foldable || open;
  return (
    <section className="flex flex-col gap-3" aria-label={`Turn ${t.turn}`}>
      {t.prompt !== null && (
        <div className="flex flex-col gap-1">
          <span className="text-[12px] text-muted">
            <strong className="font-semibold text-fg">You</strong>
            {t.promptAt ? ` · ${shortClock(t.promptAt)}` : ""}
            {t.turn > 1 ? ` · turn ${t.turn}` : ""}
          </span>
          <p className="selectable m-0 whitespace-pre-wrap text-[13px] leading-relaxed">
            {t.prompt}
          </p>
        </div>
      )}
      {foldable && (
        <button
          type="button"
          className="flex items-center gap-1.5 self-start rounded-md border bg-panel py-1 pr-2.5 pl-1.5 text-[12px] text-fg-2 hover:text-fg"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          {open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
          {turnSummary(t)}
        </button>
      )}
      {(t.message ||
        (showSteps && hasSteps) ||
        t.plan ||
        waiting.length > 0) && (
        <div className="flex flex-col gap-2.5">
          <span className="text-[12px] text-muted">
            <strong className="font-semibold text-fg">
              {agentName(agent)}
            </strong>
            {t.lastAt ? ` · ${shortClock(t.ended?.at ?? t.lastAt)}` : ""}
            {t.ended ? ` · turn ${t.turn} ended` : ""}
          </span>
          {t.plan && t.plan.length > 0 && (latest || showSteps) && (
            <Plan entries={t.plan} />
          )}
          {showSteps && t.thought && (
            <div className="text-[12px]">
              <button
                type="button"
                className="text-muted hover:text-fg"
                aria-expanded={thinking}
                onClick={() => setThinking(!thinking)}
              >
                {thinking ? "Hide thinking" : "Thinking…"}
              </button>
              {thinking && (
                <p className="selectable m-0 mt-1 whitespace-pre-wrap text-fg-2">
                  {t.thought}
                </p>
              )}
            </div>
          )}
          {showSteps && t.tools.length > 0 && <Tools tools={t.tools} />}
          {showSteps &&
            answered.map((p) => (
              <AnsweredRow key={p.permission_id} permission={p} />
            ))}
          {t.message && <AgentReply text={t.message} />}
          {waiting.map((p) => (
            <PermissionCard
              key={p.permission_id}
              agent={agent}
              permission={p}
              busy={busy}
              connected={connected}
              deadline={deadline}
              onPermit={onPermit}
            />
          ))}
        </div>
      )}
      {t.notices.map((n) => (
        <p key={n} className="m-0 text-[12px] text-muted">
          {n}
        </p>
      ))}
      {t.ended && t.ended.reason === "error" && (
        <p className="m-0 text-[12px] text-conflict">
          The turn failed{t.ended.message ? `: ${t.ended.message}` : "."}
        </p>
      )}
      {t.ended && t.ended.reason === "cancelled" && (
        <p className="m-0 text-[12px] text-muted">The turn was cancelled.</p>
      )}
    </section>
  );
}

/** The agent's reply. The journal keeps the text; this draws it as Markdown. */
function AgentReply({ text }: { text: string }) {
  const [html, setHtml] = useState<string | null>(null);
  const [plain, setPlain] = useState(false);
  useEffect(() => {
    let live = true;
    setPlain(false);
    ipc.renderMarkdown(text).then(
      (next) => {
        if (live) setHtml(next);
      },
      () => {
        if (live) setPlain(true);
      },
    );
    return () => {
      live = false;
    };
  }, [text]);
  if (plain || html === null) {
    return (
      <p className="selectable m-0 whitespace-pre-wrap text-[13px] leading-relaxed">
        {text}
      </p>
    );
  }
  return <Markdown html={html} />;
}

function Plan({ entries }: { entries: NonNullable<Turn["plan"]> }) {
  return (
    <div className="flex flex-col gap-1.5 rounded-lg border px-3 py-2.5 text-[12.5px]">
      <span className="text-[12px] font-semibold text-fg-2">Plan</span>
      <ul className="m-0 flex list-none flex-col gap-1 p-0">
        {withKeys(entries).map(([key, p]) => (
          <li key={key} className="flex items-start gap-2">
            <span className="mt-px shrink-0">
              {p.status === "completed" ? (
                <CheckIcon size={14} className="text-clean" />
              ) : p.status === "in_progress" ? (
                <ProgressIcon size={14} className="text-accent" />
              ) : (
                <CircleIcon size={14} className="text-faint" />
              )}
            </span>
            <span
              className={`selectable ${p.status === "completed" || p.status === "in_progress" ? "" : "text-muted"}`}
            >
              {p.content}
              <span className="sr-only">
                {p.status === "completed"
                  ? " (done)"
                  : p.status === "in_progress"
                    ? " (in progress)"
                    : " (not started)"}
              </span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Tools({ tools }: { tools: ToolState[] }) {
  return (
    <div className="overflow-hidden rounded-lg border">
      {tools.map((tool) => (
        <ToolRow key={tool.id} tool={tool} />
      ))}
    </div>
  );
}

function ToolRow({ tool }: { tool: ToolState }) {
  const running = tool.status === "pending" || tool.status === "in_progress";
  const [open, setOpen] = useState(
    (running && tool.kind === "execute") || tool.diffs.length > 0,
  );
  const details =
    tool.locations.length > 0 || !!tool.output || tool.diffs.length > 0;
  return (
    <div className="border-b text-[12px] last:border-b-0">
      <button
        type="button"
        className="flex w-full items-center gap-2.5 px-3 py-1.5 text-left hover:bg-panel"
        aria-expanded={details ? open : undefined}
        disabled={!details}
        onClick={() => setOpen(!open)}
      >
        <span className="shrink-0">
          {tool.status === "completed" ? (
            <CheckIcon size={14} className="text-clean" />
          ) : tool.status === "failed" ? (
            <CrossIcon size={14} className="text-conflict" />
          ) : (
            <ProgressIcon
              size={14}
              className="text-accent motion-safe:animate-spin"
            />
          )}
        </span>
        <span className="w-[60px] shrink-0 text-fg-2">{toolVerb(tool)}</span>
        <code className="mono min-w-0 flex-1 truncate">{tool.title}</code>
        {tool.status === "failed" && (
          <span className="shrink-0 text-conflict">failed</span>
        )}
      </button>
      {open && details && (
        <div className="flex flex-col gap-1 bg-panel px-3 py-2 pl-[38px]">
          {tool.locations.length > 0 && tool.diffs.length === 0 && (
            <p className="mono selectable m-0 break-all text-muted">
              {tool.locations.map(workspacePath).join(", ")}
            </p>
          )}
          {tool.diffs.length > 0 && <ReportedEdits diffs={tool.diffs} />}
          {tool.output && (
            <pre className="mono selectable m-0 max-h-60 overflow-auto whitespace-pre-wrap text-[11.5px] leading-relaxed text-fg-3">
              {tool.output}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * Edits as the agent reports them: each file's text before and after, or
 * only the part replaced, as removed and added lines. Not the workspace.
 */
function ReportedEdits({ diffs }: { diffs: RunFileDiff[] }) {
  return (
    <div className="flex flex-col gap-2">
      {withKeys(diffs.map((d) => ({ content: d.path, diff: d }))).map(
        ([key, { diff }]) => (
          <ReportedEdit key={key} diff={diff} />
        ),
      )}
    </div>
  );
}

function ReportedEdit({ diff: d }: { diff: RunFileDiff }) {
  const rows = useMemo(() => editLines(d.old_text, d.new_text), [d]);
  const { added, removed } = editCounts(rows);
  return (
    <div className="overflow-hidden rounded-md border bg-app">
      <div className="flex items-center gap-2 border-b px-2.5 py-1 text-[11.5px]">
        <code className="mono selectable min-w-0 flex-1 break-all text-fg">
          {workspacePath(d.path)}
        </code>
        {d.old_text === null && <span className="text-muted">new text</span>}
        <span className="tabular">
          <span className="text-added">+{added}</span>{" "}
          <span className="text-deleted">−{removed}</span>
        </span>
      </div>
      <div className="diff diff-wrap selectable max-h-72 overflow-auto">
        {rows.map((row, i) => (
          <div
            // Rows have no identity beyond their place in the edit.
            // biome-ignore lint/suspicious/noArrayIndexKey: the rows never reorder.
            key={i}
            className={`diff-row ${row.kind === "add" ? "diff-add" : row.kind === "del" ? "diff-del" : ""}`}
          >
            <span className="diff-sign" aria-hidden="true">
              {row.kind === "add" ? "+" : row.kind === "del" ? "−" : ""}
            </span>
            <span className="diff-text">
              <span className="sr-only">
                {row.kind === "add"
                  ? "Added: "
                  : row.kind === "del"
                    ? "Removed: "
                    : ""}
              </span>
              {row.text}
            </span>
          </div>
        ))}
      </div>
      {d.truncated && (
        <p className="m-0 border-t px-2.5 py-1 text-[11.5px] text-muted">
          Cut short: the edit is longer than the conversation keeps.
        </p>
      )}
    </div>
  );
}

function AnsweredRow({ permission: p }: { permission: PermissionState }) {
  const outcome =
    p.outcome === "allowed"
      ? "Allowed"
      : p.outcome === "rejected"
        ? "Rejected"
        : p.outcome === "cancelled"
          ? "Cancelled"
          : "Answered";
  return (
    <p className="m-0 flex items-center gap-2 text-[12px] text-fg-2">
      <TerminalIcon size={13} className="shrink-0 text-muted" />
      <span className="min-w-0 truncate">
        {outcome}: {p.detail ?? p.title}
        <span className="text-muted">
          {p.by === "auto"
            ? " (Act without asking)"
            : p.by === "run"
              ? " (the run ended)"
              : ""}
        </span>
      </span>
    </p>
  );
}

function PermissionCard({
  agent,
  permission: p,
  busy,
  connected,
  deadline,
  onPermit,
}: {
  agent: AgentKind;
  permission: PermissionState;
  busy: boolean;
  connected: boolean;
  deadline: string | null;
  onPermit: (id: string, allow: boolean) => Promise<void>;
}) {
  const heading = `perm-${p.permission_id}`;
  return (
    <section
      className="run-card text-[12.5px]"
      data-tone="amber"
      role="alert"
      aria-labelledby={heading}
    >
      <div className="flex items-center gap-2">
        <TerminalIcon size={15} className="text-dirty" />
        <h3 id={heading} className="m-0 text-[13px] font-semibold">
          {permissionHeading(agent, p.kind)}
        </h3>
      </div>
      {(p.diffs.length === 0 ||
        (p.detail !== null && p.diffs.some((d) => d.path !== p.detail))) && (
        <pre className="mono selectable m-0 whitespace-pre-wrap break-all rounded-md border bg-app px-2.5 py-2 text-[12px]">
          {p.detail ?? p.title}
        </pre>
      )}
      {p.diffs.length > 0 && <ReportedEdits diffs={p.diffs} />}
      <p className="m-0 text-[12px] leading-relaxed text-fg-2">
        Inside the run's container, not on your Mac. Asked{" "}
        {shortClock(p.asked_at)}.{" "}
        {connected
          ? `If no one answers, the run still ends${deadline ? ` at ${shortClock(deadline)}` : " at its time limit"}.`
          : "You can answer once Brainiac reconnects. While you're away nothing is allowed on your behalf."}
      </p>
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          className="btn btn-primary"
          disabled={busy || !connected}
          onClick={() => void onPermit(p.permission_id, true)}
        >
          Allow once
        </button>
        <button
          type="button"
          className="btn"
          disabled={busy || !connected}
          onClick={() => void onPermit(p.permission_id, false)}
        >
          Reject
        </button>
      </div>
    </section>
  );
}

function Changes({
  run,
  busy,
  onCollect,
  onAccept,
  onError,
  onNotice,
}: {
  run: AgentRun;
  busy: boolean;
  onCollect: (include: string[]) => Promise<void>;
  onAccept: () => Promise<void>;
  onError: (text: string) => void;
  onNotice: (text: string) => void;
}) {
  const [files, setFiles] = useState<CommitFile[] | null>(null);
  const [choosing, setChoosing] = useState(false);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const collected =
    run.collection === "ready" || run.collection === "no_changes";

  useEffect(() => {
    if (!collected || !run.result_commit) return;
    let alive = true;
    ipc
      .getRunChanges(run.id)
      .then((c) => alive && setFiles(c.files))
      .catch((e) => alive && onError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [run.id, run.result_commit, collected, onError]);

  const { added, deleted } = lineTotals(files);

  const leftOut = run.left_out;
  const leftOutCount = leftOut.length + run.left_out_more;
  const undecided = leftOutUndecided(run);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-col gap-2 border-b px-5 py-2.5 pl-lead text-[12px]">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-fg-2">
          {run.collection === "no_changes" ? (
            <span>
              The agent changed nothing: its working tree matches the start
              commit.
            </span>
          ) : (
            <>
              <span>
                Snapshot of the agent's working tree, compared with the start,{" "}
                <code className="mono">{run.start_commit.slice(0, 7)}</code>
              </span>
              {files && (
                <span className="tabular">
                  <span className="text-added">+{added}</span>{" "}
                  <span className="text-deleted">−{deleted}</span>
                </span>
              )}
              <span className="text-muted">
                Includes edits the agent didn't commit.
              </span>
            </>
          )}
        </div>
        {leftOutCount > 0 && (
          <section
            aria-label="Files left out"
            className="run-card gap-1.5 py-2.5"
            data-tone={undecided ? "amber" : "grey"}
          >
            <div className="flex flex-wrap items-center gap-2">
              <strong className="text-[12.5px] text-fg">
                {plural(leftOutCount, "new file")}{" "}
                {leftOutCount === 1 ? "was" : "were"} left out of the snapshot
              </strong>
              <span className="flex-1" />
              {undecided && (
                <>
                  <button
                    type="button"
                    className="btn btn-sm"
                    disabled={busy}
                    aria-pressed={choosing}
                    onClick={() => setChoosing(!choosing)}
                  >
                    Choose files to add…
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm btn-primary"
                    disabled={busy}
                    onClick={() => void onAccept()}
                  >
                    Keep this snapshot
                  </button>
                </>
              )}
            </div>
            <ul className="m-0 flex max-h-40 list-none flex-col gap-0.5 overflow-y-auto p-0">
              {leftOut.map((l) => (
                <li key={l.path} className="flex min-w-0 items-center gap-2">
                  {choosing && (
                    <input
                      type="checkbox"
                      aria-label={`Add ${l.path}`}
                      checked={chosen.has(l.path)}
                      onChange={(e) =>
                        setChosen((s) => {
                          const next = new Set(s);
                          if (e.target.checked) next.add(l.path);
                          else next.delete(l.path);
                          return next;
                        })
                      }
                    />
                  )}
                  <code className="mono truncate text-fg" title={l.path}>
                    {l.path}
                  </code>
                  <span className="truncate text-muted">{l.reason}</span>
                </li>
              ))}
              {run.left_out_more > 0 && (
                <li className="text-muted">
                  … and {plural(run.left_out_more, "more file")}
                </li>
              )}
            </ul>
            <span className="text-fg-2">
              {run.snapshot_accepted
                ? "You kept the snapshot without them."
                : run.kept
                  ? "The stopped container is kept until you decide."
                  : "The container is gone, so they can no longer be added."}
            </span>
            {choosing && (
              <div className="flex gap-2">
                <button
                  type="button"
                  className="btn btn-sm btn-primary"
                  disabled={busy || chosen.size === 0}
                  onClick={() => {
                    setChoosing(false);
                    void onCollect([...chosen]);
                  }}
                >
                  Collect again with {plural(chosen.size, "file")}
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => setChoosing(false)}
                >
                  Cancel
                </button>
              </div>
            )}
          </section>
        )}
      </div>
      {run.collection === "ready" && files && (
        <SnapshotFiles
          runId={run.id}
          commit={run.result_commit ?? ""}
          files={files}
          preview={false}
          repositoryId={run.repository_id}
          explainable
          onError={onError}
          onOpenInEditor={() =>
            onNotice(
              "The file is in the run's snapshot, not on this Mac. Save the patch to apply it.",
            )
          }
        />
      )}
    </div>
  );
}

function lineTotals(files: CommitFile[] | null): {
  added: number;
  deleted: number;
} {
  const added = (files ?? []).reduce((n, f) => n + (f.additions ?? 0), 0);
  const deleted = (files ?? []).reduce((n, f) => n + (f.deletions ?? 0), 0);
  return { added, deleted };
}

/** A snapshot's changed files beside the diff viewer: the collected one, or a preview. */
function SnapshotFiles({
  runId,
  commit,
  files: byPath,
  preview,
  onError,
  onOpenInEditor,
  repositoryId = "",
  explainable = false,
}: {
  runId: string;
  /** The snapshot shown: a new one reads the selected file's diff again. */
  commit: string;
  files: CommitFile[];
  preview: boolean;
  onError: (text: string) => void;
  onOpenInEditor: () => void;
  repositoryId?: string;
  /** A collected result, which can be explained (SPEC.md, section 14). */
  explainable?: boolean;
}) {
  const [selected, setSelected] = useState<string | null>(
    byPath[0]?.path ?? null,
  );
  const subject = useMemo(
    () => (explainable ? { kind: "run" as const, reference: runId } : null),
    [explainable, runId],
  );
  const explained = useExplainedPatch({
    repositoryId,
    view: "run",
    subject,
    files: byPath,
    selectedPath: selected,
    onSelectFile: setSelected,
  });
  const files = explained.ordered;
  const [diff, setDiff] = useState<DiffResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [ignoreWhitespace, setIgnoreWhitespace] = useState(false);
  // A new preview may no longer have the selected file.
  const file = files.find((f) => f.path === selected) ?? files[0] ?? null;
  const path = file?.path ?? null;
  const oldPath = file?.old_path ?? null;
  // biome-ignore lint/correctness/useExhaustiveDependencies: a newer snapshot (`commit`) reads the same file again.
  useEffect(() => {
    if (path === null) {
      setDiff(null);
      return;
    }
    let alive = true;
    setLoading(true);
    ipc
      .getRunDiff({
        run_id: runId,
        path,
        old_path: oldPath,
        options: { ignore_whitespace: ignoreWhitespace },
        preview,
      })
      .then((d) => alive && setDiff(d))
      .catch((e) => alive && onError(errorMessage(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [runId, commit, path, oldPath, ignoreWhitespace, preview, onError]);

  const index = file ? files.indexOf(file) : -1;
  const stepper =
    files.length > 1 && index >= 0
      ? {
          index,
          total: files.length,
          onPrev: () => setSelected(files[Math.max(0, index - 1)].path),
          onNext: () =>
            setSelected(files[Math.min(files.length - 1, index + 1)].path),
        }
      : undefined;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {explained.header && (
        // Explain acts on the whole result, so it sits above the files and
        // the patch rather than in the patch's toolbar.
        <div className="flex shrink-0 items-center gap-2 border-b px-4 py-1.5 text-[12.5px]">
          <span className="font-medium">The collected result</span>
          <span className="text-muted">{plural(byPath.length, "file")}</span>
          <span className="flex-1" />
          {explained.header}
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <div className="flex w-[260px] shrink-0 flex-col border-r">
          {explained.hasExplanation && (
            <div className="flex shrink-0 justify-center border-b px-2 py-1.5">
              <OrderSwitch
                order={explained.order}
                setOrder={explained.setOrder}
              />
            </div>
          )}
          <ul
            aria-label="Changed files"
            className="m-0 min-h-0 flex-1 list-none overflow-y-auto p-1"
          >
            {files.map((f) => {
              const { dir, name } = splitPath(f.path);
              return (
                <li key={f.path}>
                  <button
                    type="button"
                    className="side-row h-auto w-full items-start gap-2 py-1.5 text-left"
                    aria-current={f === file}
                    onClick={() => setSelected(f.path)}
                  >
                    {explained.steps.size > 0 && (
                      <span className="tabular mt-px w-4 shrink-0 text-right text-[11.5px] text-muted">
                        {explained.steps.get(f.path) ?? ""}
                      </span>
                    )}
                    <span className="kind mt-px" data-tone={kindTone(f.kind)}>
                      {KIND_LETTER[f.kind]}
                    </span>
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span className="truncate">{name}</span>
                      {dir && (
                        <span className="truncate text-[11px] text-muted">
                          {dir.replace(/\/$/, "")}
                        </span>
                      )}
                    </span>
                    <span className="mt-px shrink-0 text-[11px] tabular">
                      {f.additions != null && (
                        <span className="text-added">+{f.additions} </span>
                      )}
                      {f.deletions != null && (
                        <span className="text-deleted">−{f.deletions}</span>
                      )}
                      {f.is_binary && <span className="text-muted">bin</span>}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
        <div className="flex min-w-0 flex-1 flex-col">
          <DiffView
            diff={diff}
            loading={loading}
            empty="Select a file"
            historical
            onOpenInEditor={onOpenInEditor}
            stepper={stepper}
            ignoreWhitespace={ignoreWhitespace}
            onIgnoreWhitespace={setIgnoreWhitespace}
            annotate={explained.annotate}
            extra={explained.toolbar}
          />
        </div>
        {explained.panel}
      </div>
      {explained.dialog}
    </div>
  );
}

/**
 * **Changes so far** (SPEC.md, The run): a live run's working tree, read
 * at the end of each turn and on Refresh. Provisional: the agent keeps
 * working, and Finish and collect makes the result to review and export.
 */
function ChangesSoFar({
  run,
  onError,
  onNotice,
}: {
  run: AgentRun;
  onError: (text: string) => void;
  onNotice: (text: string) => void;
}) {
  const [preview, setPreview] = useState<RunPreview | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  // The run changes on every page the agent streams: only the latest answer
  // counts, and one that says what is shown already changes nothing.
  const asked = useRef(0);
  const show = useCallback((next: RunPreview) => {
    setPreview((current) =>
      current && samePreview(current, next) ? current : next,
    );
  }, []);
  const load = useCallback(() => {
    const n = ++asked.current;
    ipc.getRunPreview(run.id).then(
      (next) => n === asked.current && show(next),
      (e) => n === asked.current && onError(errorMessage(e)),
    );
  }, [run.id, onError, show]);
  useEffect(() => {
    load();
    return subscribe(
      onAgentRunChanged((e) => {
        if (e.run_id === run.id && !e.deleted) load();
      }),
    );
  }, [run.id, load]);
  const refresh = async () => {
    setRefreshing(true);
    try {
      const next = await ipc.refreshRunPreview(run.id);
      asked.current++;
      show(next);
    } catch (e) {
      onError(errorMessage(e));
      load();
    } finally {
      setRefreshing(false);
    }
  };
  const busy = refreshing || !!preview?.busy;
  const files = preview?.files ?? null;
  const { added, deleted } = lineTotals(files);
  const taken = preview?.taken_at ?? null;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-col gap-1.5 border-b px-5 py-2.5 pl-lead text-[12px]">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-fg-2">
          <span>
            {taken ? (
              <>
                The agent's working tree as of {shortClock(taken)}
                {preview && preview.turn > 0
                  ? `, after turn ${preview.turn}`
                  : ""}
                , compared with the start,{" "}
                <code className="mono">{run.start_commit.slice(0, 7)}</code>
              </>
            ) : (
              "Not read yet. The working tree is read when a turn ends, or now with Refresh."
            )}
          </span>
          {files && files.length > 0 && (
            <span className="tabular">
              <span className="text-added">+{added}</span>{" "}
              <span className="text-deleted">−{deleted}</span>
            </span>
          )}
          <span className="flex-1" />
          <button
            type="button"
            className="btn btn-sm"
            disabled={busy || !run.connected}
            title={
              run.connected
                ? "Read the working tree now"
                : "Possible once Brainiac reconnects"
            }
            onClick={() => void refresh()}
          >
            {busy && (
              <ProgressIcon size={12} className="motion-safe:animate-spin" />
            )}
            {busy ? "Reading…" : "Refresh"}
          </button>
        </div>
        <span className="text-muted">
          Provisional: the agent keeps working, and a file it was writing may be
          caught halfway. Finish and collect makes the result you review and
          export.
          {preview && preview.left_out > 0
            ? ` ${plural(preview.left_out, "new file")} would be left out, as in the collection.`
            : ""}
        </span>
        {preview?.error && (
          <span className="text-conflict">
            The last read failed: {preview.error}
            {taken ? " This is the one before it." : ""}
          </span>
        )}
      </div>
      {preview?.commit && files && files.length === 0 && (
        <p className="m-0 px-5 py-4 pl-lead text-[12.5px] text-muted">
          No changes yet: the working tree matches the start commit.
        </p>
      )}
      {files && files.length > 0 && (
        <SnapshotFiles
          runId={run.id}
          commit={preview?.commit ?? ""}
          files={files}
          preview
          onError={onError}
          onOpenInEditor={() =>
            onNotice(
              "The file is in the run's container, not on this Mac. Finish and collect, then save the patch or copy the branch command.",
            )
          }
        />
      )}
    </div>
  );
}
