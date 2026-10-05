import { ask, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  activityLabel,
  costLabel,
  foldTurns,
  GROUP_LABEL,
  groupOf,
  type PermissionState,
  producedLabel,
  type RunGroup,
  shortClock,
  type ToolState,
  type Turn,
  timeLeft,
  turnSummary,
  visibilityLabel,
} from "../lib/agentRuns";
import { absoluteTime } from "../lib/format";
import {
  type AgentRun,
  type AppSnapshot,
  type CommitFile,
  type DiffResult,
  errorMessage,
  ipc,
  onAgentRunChanged,
  type RunEvent,
  subscribe,
} from "../lib/ipc";
import { plural, splitPath } from "../lib/repo";
import DiffView from "./DiffView";
import { ChevronDown, ChevronLeft, ChevronRight, PlusIcon } from "./icons";
import { RepoChip } from "./RepoChip";

type Props = {
  snapshot: AppSnapshot;
  runs: AgentRun[] | null;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onNewRun: () => void;
  onOpenRepo: (id: string) => void;
  onError: (text: string) => void;
  onNotice: (text: string) => void;
};

const GROUPS: RunGroup[] = ["needs_you", "review", "active", "ended"];

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
        onError={props.onError}
        onNotice={props.onNotice}
      />
    );
  }
  return <RunList {...props} />;
}

function RunList({ snapshot, runs, onSelect, onNewRun }: Props) {
  const [repoFilter, setRepoFilter] = useState<string>("");
  const repos = useMemo(() => {
    const seen = new Map<string, string>();
    for (const r of runs ?? []) seen.set(r.repository_id, r.repository_name);
    return [...seen.entries()].sort((a, b) => a[1].localeCompare(b[1]));
  }, [runs]);
  const shown = (runs ?? []).filter(
    (r) => !repoFilter || r.repository_id === repoFilter,
  );
  const byId = new Map(snapshot.repositories.map((r) => [r.id, r]));
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-3 border-b px-4 pl-lead">
        <h1 className="m-0 text-[15px] font-semibold">Runs</h1>
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
        <span className="flex-1" />
        <button type="button" className="btn btn-sm" onClick={onNewRun}>
          <PlusIcon size={12} /> New run…
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {runs === null ? (
          <p className="text-muted">Loading…</p>
        ) : shown.length === 0 ? (
          <div className="flex flex-col items-start gap-2 py-8 text-[13px] text-fg-2">
            <p className="m-0">
              No runs yet. A run hands a repository's commit to Claude Code in a
              container on this Mac; its work comes back as changes you review
              before anything leaves Brainiac.
            </p>
            <button type="button" className="btn btn-sm" onClick={onNewRun}>
              New run…
            </button>
          </div>
        ) : (
          GROUPS.map((group) => {
            const members = shown.filter((r) => groupOf(r) === group);
            if (members.length === 0) return null;
            return (
              <section key={group} className="mb-4">
                <h2 className="section-label m-0 px-1 pb-1">
                  {GROUP_LABEL[group]}
                  <span className="ml-2 text-muted">{members.length}</span>
                </h2>
                <ul className="m-0 flex list-none flex-col gap-px p-0">
                  {members.map((run) => (
                    <li key={run.id}>
                      <button
                        type="button"
                        className="side-row h-auto w-full items-start gap-3 py-2 text-left"
                        onClick={() => onSelect(run.id)}
                      >
                        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                          <span className="truncate font-medium">
                            {run.title}
                          </span>
                          <span className="flex flex-wrap items-center gap-x-2 text-[11.5px] text-muted">
                            <RepoChip
                              repo={byId.get(run.repository_id)}
                              fallbackName={run.repository_name}
                            />
                            <span>{activityLabel(run)}</span>
                            {producedLabel(run) && (
                              <span>· {producedLabel(run)}</span>
                            )}
                            {visibilityLabel(run) && (
                              <span className="text-conflict">
                                · {visibilityLabel(run)}
                              </span>
                            )}
                          </span>
                        </span>
                        <span className="shrink-0 text-[11.5px] text-muted">
                          {shortClock(run.ended_at ?? run.created_at)}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            );
          })
        )}
      </div>
    </div>
  );
}

type Tab = "conversation" | "changes";

function RunView({
  run: initial,
  snapshot,
  onBack,
  onOpenRepo,
  onError,
  onNotice,
}: {
  run: AgentRun;
  snapshot: AppSnapshot;
  onBack: () => void;
  onOpenRepo: (id: string) => void;
  onError: (text: string) => void;
  onNotice: (text: string) => void;
}) {
  const [run, setRun] = useState(initial);
  const [events, setEvents] = useState<RunEvent[]>([]);
  const [tab, setTab] = useState<Tab>("conversation");
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(Date.now());
  const cursor = useRef(0);

  // The list's copy moves on with the backend's events; this view's does too.
  useEffect(() => setRun(initial), [initial]);

  const loadEvents = useCallback(async () => {
    try {
      for (;;) {
        const page = await ipc.listRunEvents(initial.id, cursor.current);
        if (page.events.length === 0) break;
        cursor.current = page.events[page.events.length - 1].seq;
        setEvents((old) => [...old, ...page.events]);
        if (cursor.current >= page.cursor) break;
      }
    } catch (e) {
      onError(errorMessage(e));
    }
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

  useEffect(() => {
    if (run.phase === "ended") return;
    const timer = setInterval(() => setNow(Date.now()), 30_000);
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
    if (
      !(await ask(
        "Cancel this run? The agent stops, and its work so far is collected for review.",
        { title: "Cancel Run", kind: "warning", okLabel: "Cancel Run" },
      ))
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
    const lost =
      run.kept && !run.snapshot_accepted
        ? run.collection === "failed" || run.collection === "none"
          ? " Its uncollected work is lost."
          : run.left_out.length > 0
            ? ` ${plural(run.left_out.length + run.left_out_more, "left-out file")} never added or accepted ${run.left_out.length === 1 ? "is" : "are"} lost.`
            : ""
        : "";
    if (
      !(await ask(
        `Delete this run? The conversation, the review, and the stopped container with its files are removed.${lost}`,
        { title: "Delete Run", kind: "warning", okLabel: "Delete" },
      ))
    )
      return;
    await act(async () => {
      await ipc.deleteAgentRun(run.id);
      onBack();
      return undefined;
    });
  };

  const repo = snapshot.repositories.find((r) => r.id === run.repository_id);
  const produced = producedLabel(run);
  const visibility = visibilityLabel(run, now);
  const idle =
    run.phase === "running" &&
    (run.activity === "idle" || run.activity === "plan_limit");
  const live = run.phase !== "ended";
  const collected =
    run.collection === "ready" || run.collection === "no_changes";
  const folded = useMemo(() => foldTurns(events), [events]);
  const reported = useMemo(() => reportedFiles(folded.turns), [folded]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex shrink-0 flex-col gap-2 border-b px-4 py-3 pl-lead">
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="btn btn-sm btn-ghost px-1.5"
            aria-label="Back to runs"
            onClick={onBack}
          >
            <ChevronLeft />
          </button>
          <h1 className="m-0 min-w-0 flex-1 truncate text-[15px] font-semibold">
            {run.title}
          </h1>
          {live && (
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={() => void cancel()}
            >
              Cancel run…
            </button>
          )}
          {live && (
            <button
              type="button"
              className="btn btn-sm btn-primary"
              disabled={busy || !idle || !run.connected}
              title={
                idle
                  ? "Stop the agent and collect its work"
                  : "Possible while the agent waits for a prompt"
              }
              onClick={() => void finish()}
            >
              Finish and collect
            </button>
          )}
          {!live &&
            run.kept &&
            run.collection === "none" &&
            run.stop_confirmed && (
              <button
                type="button"
                className="btn btn-sm btn-primary"
                disabled={busy}
                onClick={() => void collect()}
              >
                Collect work
              </button>
            )}
          {!live && (
            <button
              type="button"
              className="btn btn-sm"
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
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px] text-fg-2">
          <RepoChip
            repo={repo}
            fallbackName={run.repository_name}
            onClick={repo ? () => onOpenRepo(repo.id) : undefined}
          />
          <span className="mono" title={run.start_commit}>
            {run.start_commit.slice(0, 7)}
          </span>
          <span>Claude Code · {run.engine_name}</span>
          <span>
            {run.permissions === "ask"
              ? "Ask before actions"
              : "Act without asking"}
          </span>
        </div>
        <div className="flex flex-wrap items-center gap-2 text-[12px]">
          <Badge tone={live ? "accent" : "plain"}>{activityLabel(run)}</Badge>
          {produced && (
            <Badge tone={run.collection === "failed" ? "conflict" : "plain"}>
              {produced}
            </Badge>
          )}
          {visibility && <Badge tone="conflict">{visibility}</Badge>}
          {run.cleanup_pending && (
            <Badge tone="conflict">Cleanup pending</Badge>
          )}
          {run.deadline_at && live && (
            <span className="text-muted">
              Ends {shortClock(run.deadline_at, now)} ·{" "}
              {timeLeft(run.deadline_at, now)}
            </span>
          )}
          <span className="text-muted">{costLabel(run.payment)}</span>
        </div>
        {run.error && (
          <p className="m-0 text-[12.5px] text-conflict">{run.error}</p>
        )}
        {run.cleanup_pending && (
          <p className="m-0 flex items-center gap-2 text-[12.5px] text-conflict">
            <span>{run.cleanup_pending}</span>
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={() => void act(() => ipc.retryRunCleanup(run.id))}
            >
              Retry cleanup
            </button>
          </p>
        )}
        <div className="flex gap-1">
          <TabButton
            selected={tab === "conversation"}
            onClick={() => setTab("conversation")}
          >
            Conversation
          </TabButton>
          <TabButton
            selected={tab === "changes"}
            disabled={!collected && run.collection !== "failed"}
            onClick={() => setTab("changes")}
          >
            Changes
          </TabButton>
        </div>
      </header>
      <div className="flex min-h-0 flex-1">
        {tab === "conversation" ? (
          <Conversation
            run={run}
            turns={folded.turns}
            notices={folded.notices}
            ended={folded.ended}
            busy={busy}
            onPermit={(id, allow) =>
              act(() => ipc.answerRunPermission(run.id, id, allow))
            }
            onPrompt={(text) => act(() => ipc.sendRunPrompt(run.id, text))}
            idle={idle && run.connected}
          />
        ) : (
          <Changes
            run={run}
            busy={busy}
            onCollect={collect}
            onAccept={() => act(() => ipc.acceptRunSnapshot(run.id))}
            onDiscard={() => void discard()}
            onError={onError}
            onNotice={onNotice}
          />
        )}
        <aside className="w-[260px] shrink-0 overflow-y-auto border-l px-4 py-3 text-[12px]">
          <Fact label="Start">
            <span className="mono">{run.start_commit.slice(0, 10)}</span>{" "}
            {run.start_subject}
          </Fact>
          <Fact label="Started">{absoluteTime(run.created_at)}</Fact>
          <Fact label="Where it runs">
            {run.engine_name}, in a container on this Mac
          </Fact>
          <Fact label="Provider">
            Anthropic,{" "}
            {run.payment === "claude_plan" ? "Claude plan" : "API key"} from{" "}
            {run.credential_source}
          </Fact>
          <Fact label="Network">Unrestricted</Fact>
          <Fact label="Limits">
            {run.time_limit_minutes} min · {run.cpus} CPUs ·{" "}
            {Math.round(run.memory_mib / 1024)} GB memory · {run.workspace_gib}{" "}
            GB workspace
          </Fact>
          <Fact label="Image">
            <span className="mono break-all">{run.image_name}</span>
          </Fact>
          <Fact label="Files the agent says it changed">
            {reported.length === 0 ? (
              <span className="text-muted">None reported</span>
            ) : (
              <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
                {reported.map((p) => (
                  <li key={p} className="mono truncate" title={p}>
                    {p}
                  </li>
                ))}
              </ul>
            )}
            <span className="text-muted">
              As reported by the agent; the collected snapshot is what counts.
            </span>
          </Fact>
        </aside>
      </div>
    </div>
  );
}

function Badge({
  tone,
  children,
}: {
  tone: "accent" | "plain" | "conflict";
  children: React.ReactNode;
}) {
  const cls =
    tone === "accent"
      ? "badge"
      : tone === "conflict"
        ? "rounded-lg border border-conflict/40 px-1.5 text-conflict"
        : "rounded-lg border px-1.5 text-fg-2";
  return <span className={`${cls} text-[11px] leading-4`}>{children}</span>;
}

function TabButton({
  selected,
  disabled,
  onClick,
  children,
}: {
  selected: boolean;
  disabled?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      className="btn btn-sm btn-ghost"
      aria-pressed={selected}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
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
    <div className="mb-3 flex flex-col gap-0.5">
      <span className="section-label">{label}</span>
      <span className="selectable text-fg-2">{children}</span>
    </div>
  );
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

/** Paths of the agent's edit tools, in order, once each. */
function reportedFiles(turns: Turn[]): string[] {
  const seen = new Set<string>();
  for (const t of turns)
    for (const tool of t.tools)
      if (
        tool.kind === "edit" ||
        tool.kind === "delete" ||
        tool.kind === "move"
      )
        for (const l of tool.locations) seen.add(l);
  return [...seen];
}

function Conversation({
  run,
  turns,
  notices,
  ended,
  busy,
  idle,
  onPermit,
  onPrompt,
}: {
  run: AgentRun;
  turns: Turn[];
  notices: string[];
  ended: { outcome: string; message: string | null } | null;
  busy: boolean;
  idle: boolean;
  onPermit: (id: string, allow: boolean) => Promise<void>;
  onPrompt: (text: string) => Promise<void>;
}) {
  const [text, setText] = useState("");
  const [open, setOpen] = useState<Set<number>>(new Set());
  const bottom = useRef<HTMLDivElement>(null);
  const last = turns[turns.length - 1];
  // biome-ignore lint/correctness/useExhaustiveDependencies: new events scroll the latest into view.
  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end" });
  }, [turns.length, last?.message.length, last?.tools.length]);
  const pending = new Set(run.pending_permissions.map((p) => p.permission_id));
  const send = async () => {
    const prompt = text.trim();
    if (!prompt) return;
    await onPrompt(prompt);
    setText("");
  };
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 pl-lead">
        {run.phase === "preparing" && turns.length === 0 && (
          <p className="text-muted">
            Preparing: the container starts and clones the start commit.
          </p>
        )}
        {notices.map((n) => (
          <p key={n} className="m-0 mb-2 text-[12px] text-muted">
            {n}
          </p>
        ))}
        {turns.map((t) => {
          const isLast = t === last;
          const foldedTurn = !!t.ended && !isLast && !open.has(t.turn);
          return (
            <section key={t.turn} className="mb-4">
              {t.prompt !== null && (
                <div className="mb-2 flex flex-col gap-0.5">
                  <span className="text-[11px] text-muted">
                    You · {t.promptAt ? shortClock(t.promptAt) : ""}
                  </span>
                  <p className="selectable m-0 whitespace-pre-wrap rounded-lg bg-header px-3 py-2 text-[13px]">
                    {t.prompt}
                  </p>
                </div>
              )}
              {foldedTurn ? (
                <button
                  type="button"
                  className="flex items-center gap-1 text-[12px] text-fg-2 hover:text-fg"
                  onClick={() => setOpen((s) => new Set(s).add(t.turn))}
                >
                  <ChevronRight size={12} /> {turnSummary(t)}
                </button>
              ) : (
                <TurnBody
                  turn={t}
                  pending={pending}
                  busy={busy}
                  onPermit={onPermit}
                  onFold={
                    t.ended && !isLast
                      ? () =>
                          setOpen((s) => {
                            const next = new Set(s);
                            next.delete(t.turn);
                            return next;
                          })
                      : undefined
                  }
                />
              )}
            </section>
          );
        })}
        {ended && (
          <p className="m-0 text-[12px] text-muted">
            {activityLabel(run)}
            {ended.message ? `: ${ended.message}` : ""}
          </p>
        )}
        <div ref={bottom} />
      </div>
      {run.phase !== "ended" && (
        <div className="flex shrink-0 items-end gap-2 border-t px-4 py-3 pl-lead">
          <textarea
            className="field min-h-[60px] flex-1 resize-y text-[13px]"
            placeholder={
              idle
                ? "Next prompt"
                : run.activity === "permission"
                  ? "Answer the permission request first"
                  : "The agent is working"
            }
            aria-label="Next prompt"
            value={text}
            disabled={!idle || busy}
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
            className="btn btn-sm btn-primary"
            disabled={!idle || busy || !text.trim()}
            title="Send (⌘↩)"
            onClick={() => void send()}
          >
            Send
          </button>
        </div>
      )}
    </div>
  );
}

function TurnBody({
  turn: t,
  pending,
  busy,
  onPermit,
  onFold,
}: {
  turn: Turn;
  pending: Set<string>;
  busy: boolean;
  onPermit: (id: string, allow: boolean) => Promise<void>;
  onFold?: () => void;
}) {
  const [thinking, setThinking] = useState(false);
  return (
    <div className="flex flex-col gap-2">
      {onFold && (
        <button
          type="button"
          className="flex items-center gap-1 self-start text-[12px] text-fg-2 hover:text-fg"
          onClick={onFold}
        >
          <ChevronDown size={12} /> {turnSummary(t)}
        </button>
      )}
      {t.plan && t.plan.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-0.5 rounded-lg border px-3 py-2 text-[12px]">
          {withKeys(t.plan).map(([key, p]) => (
            <li key={key} className="flex gap-2">
              <span className="w-20 shrink-0 text-muted">{p.status ?? ""}</span>
              <span className="selectable">{p.content}</span>
            </li>
          ))}
        </ul>
      )}
      {t.thought && (
        <div className="text-[12px]">
          <button
            type="button"
            className="text-muted hover:text-fg"
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
      {t.tools.map((tool) => (
        <ToolRow key={tool.id} tool={tool} />
      ))}
      {t.permissions.map((p) => (
        <PermissionRow
          key={p.permission_id}
          permission={p}
          pending={pending.has(p.permission_id)}
          busy={busy}
          onPermit={onPermit}
        />
      ))}
      {t.message && (
        <p className="selectable m-0 whitespace-pre-wrap text-[13px] leading-relaxed">
          {t.message}
        </p>
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
    </div>
  );
}

function ToolRow({ tool }: { tool: ToolState }) {
  const [open, setOpen] = useState(false);
  const word =
    tool.status === "completed"
      ? "done"
      : tool.status === "failed"
        ? "failed"
        : tool.status === "in_progress"
          ? "running"
          : (tool.status ?? "");
  return (
    <div className="rounded-md border px-2 py-1 text-[12px]">
      <button
        type="button"
        className="flex w-full items-center gap-2 text-left"
        onClick={() => setOpen(!open)}
      >
        <span className="w-14 shrink-0 text-muted">{tool.kind ?? "tool"}</span>
        <span className="mono min-w-0 flex-1 truncate">{tool.title}</span>
        <span
          className={`shrink-0 ${tool.status === "failed" ? "text-conflict" : "text-muted"}`}
        >
          {word}
        </span>
      </button>
      {open && (
        <div className="mt-1 flex flex-col gap-1">
          {tool.locations.length > 0 && (
            <p className="mono selectable m-0 break-all text-muted">
              {tool.locations.join(", ")}
            </p>
          )}
          {tool.output && (
            <pre className="mono selectable m-0 max-h-60 overflow-auto whitespace-pre-wrap text-[11.5px]">
              {tool.output}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

function PermissionRow({
  permission: p,
  pending,
  busy,
  onPermit,
}: {
  permission: PermissionState;
  pending: boolean;
  busy: boolean;
  onPermit: (id: string, allow: boolean) => Promise<void>;
}) {
  return (
    <div
      className={`flex flex-col gap-1 rounded-lg border px-3 py-2 text-[12.5px] ${pending ? "border-accent" : ""}`}
      role={pending ? "alert" : undefined}
    >
      <span className="font-medium">
        {pending ? "Permission request" : "Permission"} · {p.title}
      </span>
      {p.detail && (
        <code className="mono selectable break-all text-[11.5px]">
          {p.detail}
        </code>
      )}
      <span className="text-[11.5px] text-muted">
        Runs inside the container · asked {shortClock(p.asked_at)} · the run
        still ends at its time limit
      </span>
      {pending ? (
        <div className="flex gap-2">
          <button
            type="button"
            className="btn btn-sm btn-primary"
            disabled={busy}
            onClick={() => void onPermit(p.permission_id, true)}
          >
            Allow once
          </button>
          <button
            type="button"
            className="btn btn-sm"
            disabled={busy}
            onClick={() => void onPermit(p.permission_id, false)}
          >
            Reject
          </button>
        </div>
      ) : (
        <span className="text-[11.5px] text-fg-2">
          {p.outcome === "allowed"
            ? "Allowed"
            : p.outcome === "rejected"
              ? "Rejected"
              : p.outcome === "cancelled"
                ? "Cancelled"
                : "Answered"}
          {p.by === "auto"
            ? " (Act without asking)"
            : p.by === "run"
              ? " (the run ended)"
              : ""}
        </span>
      )}
    </div>
  );
}

function Changes({
  run,
  busy,
  onCollect,
  onAccept,
  onDiscard,
  onError,
  onNotice,
}: {
  run: AgentRun;
  busy: boolean;
  onCollect: (include: string[]) => Promise<void>;
  onAccept: () => Promise<void>;
  onDiscard: () => void;
  onError: (text: string) => void;
  onNotice: (text: string) => void;
}) {
  const [files, setFiles] = useState<CommitFile[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState<DiffResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [ignoreWhitespace, setIgnoreWhitespace] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const collected =
    run.collection === "ready" || run.collection === "no_changes";

  useEffect(() => {
    if (!collected || !run.result_commit) return;
    let alive = true;
    ipc
      .getRunChanges(run.id)
      .then((c) => {
        if (!alive) return;
        setFiles(c.files);
        setSelected((s) => s ?? c.files[0]?.path ?? null);
      })
      .catch((e) => alive && onError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [run.id, run.result_commit, collected, onError]);

  const file = files?.find((f) => f.path === selected) ?? null;
  useEffect(() => {
    if (!file) {
      setDiff(null);
      return;
    }
    let alive = true;
    setLoading(true);
    ipc
      .getRunDiff({
        run_id: run.id,
        path: file.path,
        old_path: file.old_path,
        options: { ignore_whitespace: ignoreWhitespace },
      })
      .then((d) => alive && setDiff(d))
      .catch((e) => alive && onError(errorMessage(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [run.id, file, ignoreWhitespace, onError]);

  const copyPatch = async () => {
    try {
      const patch = await ipc.copyRunPatch(run.id);
      await navigator.clipboard?.writeText(patch);
      onNotice("Patch copied");
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
  const index = files?.findIndex((f) => f.path === selected) ?? -1;
  const stepper =
    files && files.length > 1 && index >= 0
      ? {
          index,
          total: files.length,
          onPrev: () => setSelected(files[Math.max(0, index - 1)].path),
          onNext: () =>
            setSelected(files[Math.min(files.length - 1, index + 1)].path),
        }
      : undefined;

  if (run.collection === "failed") {
    return (
      <div className="flex flex-1 flex-col gap-2 px-4 py-3 pl-lead text-[13px]">
        <p className="m-0 font-medium">Collection failed</p>
        <p className="m-0 text-fg-2">
          {run.collection_error ?? "The work could not be collected."} The
          stopped container and its files are kept.
        </p>
        <div className="flex gap-2">
          <button
            type="button"
            className="btn btn-sm btn-primary"
            disabled={busy || !run.kept}
            onClick={() => void onCollect([])}
          >
            Retry collection
          </button>
          <button
            type="button"
            className="btn btn-sm"
            disabled={busy || !run.kept}
            onClick={onDiscard}
          >
            Discard work…
          </button>
        </div>
      </div>
    );
  }

  const leftOut = run.left_out;
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b px-4 py-2 pl-lead text-[12px]">
        <span className="text-fg-2">
          {run.collection === "no_changes"
            ? "The agent changed nothing."
            : `${plural(files?.length ?? run.changed_files ?? 0, "file")} against the start · includes edits the agent did not commit`}
        </span>
        <span className="flex-1" />
        {run.collection === "ready" && (
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
          </>
        )}
      </div>
      {(leftOut.length > 0 || run.left_out_more > 0) && (
        <div className="shrink-0 border-b px-4 py-2 pl-lead text-[12px]">
          <div className="flex items-center gap-2">
            <span className="font-medium">
              Left out: {plural(leftOut.length + run.left_out_more, "new file")}
            </span>
            <span className="flex-1" />
            {!run.snapshot_accepted && run.kept && (
              <>
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={busy}
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
            {run.snapshot_accepted && (
              <span className="text-muted">Snapshot kept</span>
            )}
          </div>
          <ul className="m-0 mt-1 flex max-h-40 list-none flex-col gap-0.5 overflow-y-auto p-0">
            {leftOut.map((l) => (
              <li key={l.path} className="flex items-center gap-2">
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
                <span className="mono truncate" title={l.path}>
                  {l.path}
                </span>
                <span className="truncate text-muted">{l.reason}</span>
              </li>
            ))}
            {run.left_out_more > 0 && (
              <li className="text-muted">
                … and {plural(run.left_out_more, "more file")}
              </li>
            )}
          </ul>
          {choosing && (
            <div className="mt-1 flex gap-2">
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
        </div>
      )}
      {run.collection === "ready" && (
        <div className="flex min-h-0 flex-1">
          <ul className="m-0 w-[260px] shrink-0 list-none overflow-y-auto border-r p-1">
            {(files ?? []).map((f) => {
              const { dir, name } = splitPath(f.path);
              return (
                <li key={f.path}>
                  <button
                    type="button"
                    className="side-row w-full text-left"
                    aria-current={f.path === selected}
                    onClick={() => setSelected(f.path)}
                  >
                    <span className="min-w-0 flex-1 truncate">
                      {dir && <span className="text-muted">{dir}/</span>}
                      {name}
                    </span>
                    <span className="shrink-0 text-[11px] tabular">
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
          <div className="flex min-w-0 flex-1 flex-col">
            <DiffView
              diff={diff}
              loading={loading}
              empty="Select a file"
              historical
              onOpenInEditor={() =>
                onNotice(
                  "The file is in the run's snapshot, not on this Mac. Save the patch to apply it.",
                )
              }
              stepper={stepper}
              ignoreWhitespace={ignoreWhitespace}
              onIgnoreWhitespace={setIgnoreWhitespace}
            />
          </div>
        </div>
      )}
    </div>
  );
}
