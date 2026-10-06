import { useEffect, useId, useState } from "react";
import {
  durationLabel,
  MODEL_SUGGESTIONS,
  parseModel,
  paymentLabel,
  TIME_LIMITS,
} from "../lib/agentRuns";
import { relativeTime } from "../lib/format";
import {
  type AgentHost,
  type AgentRun,
  type AgentSettings,
  type AppSnapshot,
  errorMessage,
  ipc,
  type RunPermissions,
  type RunStartPreview,
} from "../lib/ipc";
import { plural } from "../lib/repo";
import Dialog from "./Dialog";
import {
  AlertIcon,
  BoxIcon,
  ExternalIcon,
  GlobeIcon,
  KeyIcon,
  TerminalIcon,
} from "./icons";

type Props = {
  snapshot: AppSnapshot;
  /** The repository New run opened from, if any. */
  repositoryId?: string;
  onClose: () => void;
  onStarted: (run: AgentRun) => void;
  onOpenSettings: () => void;
};

/** "This Mac · OrbStack", "build-01 · remote". */
function hostChoice(host: AgentHost): string {
  if (host.kind === "local")
    return host.engine_name ? `This Mac · ${host.engine_name}` : "This Mac";
  return `${host.name} · remote`;
}

/**
 * New run (SPEC.md, New run): a repository, a start resolved once to a
 * commit, the prompt, permissions, the time limit and resources, and what
 * the run sends and who can read the credential, before Start run.
 */
export default function NewRunDialog({
  snapshot,
  repositoryId,
  onClose,
  onStarted,
  onOpenSettings,
}: Props) {
  const repos = [...snapshot.repositories].sort((a, b) =>
    a.name.localeCompare(b.name),
  );
  const [settings, setSettings] = useState<AgentSettings | null>(null);
  const [repo, setRepo] = useState(repositoryId ?? repos[0]?.id ?? "");
  const [start, setStart] = useState("HEAD");
  const [preview, setPreview] = useState<RunStartPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [prompt, setPrompt] = useState("");
  const [permissions, setPermissions] = useState<RunPermissions>("ask");
  const [timeLimit, setTimeLimit] = useState(60);
  const [cpus, setCpus] = useState(4);
  const [memoryGb, setMemoryGb] = useState(8);
  const [workspaceGb, setWorkspaceGb] = useState(20);
  const [model, setModel] = useState("");
  const [hostId, setHostId] = useState("");
  const [limits, setLimits] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const id = useId();

  useEffect(() => {
    let alive = true;
    ipc
      .getAgentSettings()
      .then((s) => {
        if (!alive) return;
        setSettings(s);
        setPermissions(s.profile.permissions);
        setTimeLimit(s.profile.time_limit_minutes);
        setCpus(s.profile.cpus);
        setMemoryGb(Math.round(s.profile.memory_mib / 1024));
        setWorkspaceGb(s.profile.workspace_gib);
        setModel(s.profile.model);
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, []);

  // The start is resolved once, as the user leaves the field or picks a repository.
  useEffect(() => {
    if (!repo) return;
    let alive = true;
    setPreview(null);
    setPreviewError(null);
    const timer = setTimeout(() => {
      ipc
        .previewRunStart(repo, start)
        .then((p) => alive && setPreview(p))
        .catch((e) => alive && setPreviewError(errorMessage(e)));
    }, 300);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [repo, start]);

  const missing = settings?.missing ?? [];
  const hosts = (settings?.hosts ?? []).filter(
    (h) => h.kind === "local" || (h.approved && h.installed && !h.state_kept),
  );
  const askHost = hosts.length > 1;
  const chosen =
    hosts.find((h) => (h.kind === "local" ? "" : h.id) === hostId) ?? null;
  const remote = chosen?.kind === "ssh";
  const hostName = remote && chosen ? chosen.name : "This Mac";
  const engine =
    hosts.find((h) => h.kind === "local")?.engine_name ?? "its Docker engine";
  const blocked = remote
    ? missing.filter(
        (m) =>
          !m.startsWith("Choose where") &&
          !m.startsWith("Build the image") &&
          !m.startsWith("Rebuild the image") &&
          !m.startsWith("Test again") &&
          !m.startsWith("Pass a test") &&
          !m.startsWith("Run the test"),
      )
    : missing;
  const ready = !!preview && prompt.trim().length > 0 && blocked.length === 0;
  const ends = new Date(Date.now() + timeLimit * 60_000).toLocaleTimeString(
    "en-GB",
    { hour: "2-digit", minute: "2-digit" },
  );
  const repoSummary = repos.find((r) => r.id === repo);
  const uncommitted = repoSummary?.counts?.unique_paths ?? 0;
  const branch = repoSummary?.head?.branch ?? null;

  const modelProblem = (() => {
    const parsed = parseModel(model);
    return "error" in parsed ? parsed.error : null;
  })();

  const startRun = async () => {
    if (!preview || !ready || busy) return;
    if (modelProblem) {
      setError(modelProblem);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const run = await ipc.startAgentRun({
        repository_id: repo,
        start_commit: preview.commit,
        prompt: prompt.trim(),
        permissions,
        time_limit_minutes: timeLimit,
        cpus,
        memory_mib: memoryGb * 1024,
        workspace_gib: workspaceGb,
        model: model.trim(),
        host_id: hostId,
      });
      onStarted(run);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const payment = settings?.profile.payment ?? "api_key";
  const credential =
    payment === "claude_plan" ? "Claude Code token" : "API key";
  return (
    <Dialog
      title="New run"
      width={680}
      onClose={onClose}
      footer={
        <>
          <span className="mr-auto text-[12px] text-muted">
            Nothing is pushed until you review the result.
          </span>
          <button type="button" className="btn btn-sm" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-sm btn-primary"
            disabled={!ready || busy}
            title="Start run (⌘↩)"
            onClick={() => void startRun()}
          >
            {busy ? "Starting…" : "Start run"}
            <span className="font-normal opacity-80">⌘↩</span>
          </button>
        </>
      }
    >
      {/* biome-ignore lint/a11y/noStaticElementInteractions: ⌘↩ from any field starts the run; the button does the same. */}
      <div
        className="flex flex-col gap-4 text-[13px]"
        onKeyDown={(e) => {
          if (e.key === "Enter" && e.metaKey) {
            e.preventDefault();
            void startRun();
          }
        }}
      >
        {error && (
          <div role="alert" className="text-[12.5px] text-conflict">
            {error}
          </div>
        )}
        {settings && blocked.length > 0 && (
          <div className="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-[12.5px] text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
            <p className="m-0">Before the first run, in Settings → Agents:</p>
            <ul className="m-0 pl-5">
              {blocked.map((m) => (
                <li key={m}>{m}</li>
              ))}
            </ul>
            <button
              type="button"
              className="btn btn-sm mt-2"
              onClick={onOpenSettings}
            >
              Open Settings → Agents
            </button>
          </div>
        )}
        <div className="flex flex-wrap gap-3.5">
          <label className="flex min-w-[200px] flex-1 flex-col gap-1">
            <FieldLabel>Repository</FieldLabel>
            <select
              className="field"
              value={repo}
              onChange={(e) => setRepo(e.target.value)}
            >
              {repos.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name}
                </option>
              ))}
            </select>
            {repos.length === 0 && (
              <span className="text-[12px] text-muted">
                Add a repository first.
              </span>
            )}
          </label>
          <label className="flex min-w-[200px] flex-1 flex-col gap-1">
            <FieldLabel>Start from</FieldLabel>
            <input
              className="field mono"
              value={start}
              list={`${id}-starts`}
              placeholder="A branch or a commit"
              spellCheck={false}
              onChange={(e) => setStart(e.target.value)}
            />
            <datalist id={`${id}-starts`}>
              <option value="HEAD" />
              {branch && <option value={branch} />}
            </datalist>
          </label>
        </div>
        <div className="flex flex-col gap-2 rounded-lg bg-panel px-3 py-2.5 text-[12px] leading-relaxed text-fg-3">
          {preview ? (
            <>
              <div className="selectable">
                <code className="mono" title={preview.commit}>
                  {preview.commit.slice(0, 7)}
                </code>{" "}
                · {preview.subject} · {preview.author} ·{" "}
                {relativeTime(preview.committed_at)}
              </div>
              {uncommitted > 0 && (
                <div className="flex items-start gap-2">
                  <AlertIcon size={14} className="mt-0.5 shrink-0 text-dirty" />
                  <span>
                    Your checkout has{" "}
                    {plural(uncommitted, "uncommitted change")}. They aren't
                    part of the run; it starts from this commit.
                  </span>
                </div>
              )}
              <div className="flex items-start gap-2">
                <BoxIcon size={14} className="mt-0.5 shrink-0 text-fg-2" />
                <span>
                  {remote
                    ? `${hostName} gets this commit and its history (${plural(preview.history_commits, "commit")}) over SSH.`
                    : `The container gets this commit and its history (${plural(preview.history_commits, "commit")}).`}{" "}
                  Uncommitted changes, other branches, stashes, hooks, remotes,
                  and Git settings stay on this Mac.
                </span>
              </div>
            </>
          ) : previewError ? (
            <span className="text-conflict">{previewError}</span>
          ) : (
            <span className="text-muted">Resolving…</span>
          )}
        </div>
        <label className="flex flex-col gap-1">
          <FieldLabel>Prompt</FieldLabel>
          <textarea
            className="field min-h-[100px] resize-y leading-relaxed"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="What should the agent do?"
          />
        </label>
        {askHost && (
          <div className="flex flex-col gap-1.5">
            <FieldLabel id={`${id}-host`}>Runs on</FieldLabel>
            <fieldset
              className="seg m-0 self-start border-0"
              aria-labelledby={`${id}-host`}
            >
              {hosts.map((h) => {
                const value = h.kind === "local" ? "" : h.id;
                return (
                  <button
                    key={h.id}
                    type="button"
                    aria-pressed={hostId === value}
                    onClick={() => setHostId(value)}
                  >
                    {hostChoice(h)}
                  </button>
                );
              })}
            </fieldset>
            <span className="text-[12px] leading-relaxed text-fg-2">
              {remote
                ? `Keeps working while this Mac sleeps, Brainiac is closed, or SSH drops. Questions wait on ${hostName} until you're back.`
                : "Pauses while this Mac sleeps. When it wakes, a run past its time limit is stopped, possibly a few seconds after wake."}
            </span>
          </div>
        )}
        <div className="flex flex-col gap-1.5">
          <div className="flex flex-wrap items-end gap-3.5">
            <label className="flex min-w-[180px] flex-1 flex-col gap-1">
              <FieldLabel>Agent</FieldLabel>
              <select className="field" defaultValue="claude">
                <option value="claude">
                  Claude Code · {paymentLabel(payment)}
                </option>
              </select>
            </label>
            <label className="flex w-[170px] flex-col gap-1">
              <FieldLabel>Model</FieldLabel>
              <input
                className="field"
                value={model}
                list={`${id}-models`}
                placeholder="Claude Code's default"
                spellCheck={false}
                aria-invalid={modelProblem ? true : undefined}
                title={modelProblem ?? undefined}
                onChange={(e) => setModel(e.target.value)}
              />
              <datalist id={`${id}-models`}>
                {MODEL_SUGGESTIONS.map((m) => (
                  <option key={m} value={m} />
                ))}
              </datalist>
            </label>
            <label className="flex w-[150px] flex-col gap-1">
              <FieldLabel>Time limit</FieldLabel>
              <select
                className="field"
                value={timeLimit}
                onChange={(e) => setTimeLimit(Number(e.target.value))}
              >
                {TIME_LIMITS.map((m) => (
                  <option key={m} value={m}>
                    {durationLabel(m)}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <span className="text-[12px] leading-relaxed text-fg-2">
            Ends at <strong className="text-fg">{ends}</strong>, even while it
            waits for you{remote ? " or this Mac sleeps" : ""} · {cpus} CPU ·{" "}
            {memoryGb} GB memory · {workspaceGb} GB workspace{" "}
            <button
              type="button"
              className="text-link hover:underline"
              aria-expanded={limits}
              onClick={() => setLimits(!limits)}
            >
              Change…
            </button>
          </span>
          {modelProblem && (
            <span className="text-[12px] text-conflict">{modelProblem}</span>
          )}
        </div>
        {limits && (
          <div className="-mt-1 flex flex-wrap gap-3">
            <Num
              label="CPUs"
              value={cpus}
              min={1}
              max={64}
              onChange={setCpus}
            />
            <Num
              label="Memory (GB)"
              value={memoryGb}
              min={2}
              max={256}
              onChange={setMemoryGb}
            />
            <Num
              label="Workspace (GB)"
              value={workspaceGb}
              min={1}
              max={500}
              onChange={setWorkspaceGb}
            />
          </div>
        )}
        <div className="flex flex-col gap-1.5">
          <FieldLabel id={`${id}-perm`}>Permissions</FieldLabel>
          <fieldset
            className="seg m-0 self-start border-0"
            aria-labelledby={`${id}-perm`}
          >
            <button
              type="button"
              aria-pressed={permissions === "ask"}
              onClick={() => setPermissions("ask")}
            >
              Ask before actions
            </button>
            <button
              type="button"
              aria-pressed={permissions === "act"}
              onClick={() => setPermissions("act")}
            >
              Act without asking
            </button>
          </fieldset>
          <span className="text-[12px] leading-relaxed text-fg-2">
            {permissions === "ask"
              ? "The agent asks before running commands or editing files, and waits for you."
              : "The agent runs any command and edits any file inside its container without asking. It can never push, get new credentials, or change where the run executes."}
          </span>
        </div>
        <section
          aria-labelledby={`${id}-before`}
          className="flex flex-col gap-2.5 rounded-[10px] border px-3.5 py-3 text-[12.5px] leading-relaxed"
        >
          <h3
            id={`${id}-before`}
            className="m-0 text-[12px] font-semibold text-fg-2"
          >
            Before you start
          </h3>
          <Disclosure icon={<ExternalIcon size={15} />}>
            {payment === "claude_plan" ? (
              <>
                Code and prompts go to <strong>Anthropic</strong> under your{" "}
                <strong>Claude plan</strong>, and use its usage limits.
              </>
            ) : (
              <>
                Code and prompts go to <strong>Anthropic</strong>, paid with
                your <strong>API key</strong>.
              </>
            )}
          </Disclosure>
          <Disclosure icon={<TerminalIcon size={15} />}>
            {remote
              ? `Code, prompts, and the ${credential} go to ${hostName}. Anyone who administers it can read them. Its work stays there until you collect or discard it.`
              : `The run's container is on this Mac, in ${engine}. Its work stays there until you collect or discard it.`}
          </Disclosure>
          <Disclosure icon={<GlobeIcon size={15} />}>
            The network is unrestricted: the agent can reach any site, including
            services on {remote ? `${hostName}'s` : "your"} network.
          </Disclosure>
          <Disclosure icon={<KeyIcon size={15} />}>
            The agent can read your {credential}, and so can the repository's
            code and its Claude Code settings, and anyone who{" "}
            {remote ? `administers ${hostName}` : "controls this Docker engine"}
            . Run only repositories you would trust with it.
          </Disclosure>
        </section>
      </div>
    </Dialog>
  );
}

function FieldLabel({
  id,
  children,
}: {
  id?: string;
  children: React.ReactNode;
}) {
  return (
    <span id={id} className="text-[12px] font-semibold text-fg-2">
      {children}
    </span>
  );
}

function Disclosure({
  icon,
  children,
}: {
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-2.5">
      <span className="mt-0.5 shrink-0 text-accent">{icon}</span>
      <span>{children}</span>
    </div>
  );
}

function Num({
  label,
  value,
  min,
  max,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  onChange: (n: number) => void;
}) {
  return (
    <label className="flex flex-col gap-1 text-[12px]">
      <span>{label}</span>
      <input
        className="field w-24"
        type="number"
        min={min}
        max={max}
        value={value}
        onChange={(e) => {
          const n = Number(e.target.value);
          if (Number.isFinite(n)) onChange(Math.min(max, Math.max(min, n)));
        }}
      />
    </label>
  );
}
