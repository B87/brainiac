import { useEffect, useId, useState } from "react";
import {
  destinationLabel,
  durationLabel,
  paymentLabel,
  TIME_LIMITS,
} from "../lib/agentRuns";
import {
  type AgentRun,
  type AgentSettings,
  type AppSnapshot,
  errorMessage,
  ipc,
  type RunPermissions,
  type RunStartPreview,
} from "../lib/ipc";
import Dialog from "./Dialog";

type Props = {
  snapshot: AppSnapshot;
  /** The repository New run opened from, if any. */
  repositoryId?: string;
  onClose: () => void;
  onStarted: (run: AgentRun) => void;
  onOpenSettings: () => void;
};

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
  const ready = !!preview && prompt.trim().length > 0 && missing.length === 0;
  const ends = new Date(Date.now() + timeLimit * 60_000).toLocaleTimeString(
    "en-GB",
    { hour: "2-digit", minute: "2-digit" },
  );

  const startRun = async () => {
    if (!preview) return;
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
      });
      onStarted(run);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const payment = settings?.profile.payment ?? "api_key";
  return (
    <Dialog
      title="New run"
      width={640}
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
            onClick={() => void startRun()}
          >
            {busy ? "Starting…" : "Start run"}
          </button>
        </>
      }
    >
      <div className="flex flex-col gap-4 text-[13px]">
        {error && (
          <div role="alert" className="text-[12.5px] text-conflict">
            {error}
          </div>
        )}
        {settings && missing.length > 0 && (
          <div className="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-[12.5px] text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
            <p className="m-0">Before the first run, in Settings → Agents:</p>
            <ul className="m-0 pl-5">
              {missing.map((m) => (
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
        <label className="flex flex-col gap-1">
          <span className="font-medium">Repository</span>
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
        <label className="flex flex-col gap-1">
          <span className="font-medium">Start from</span>
          <input
            className="field mono"
            value={start}
            placeholder="A branch or a commit"
            onChange={(e) => setStart(e.target.value)}
          />
          {preview ? (
            <span className="text-[12px] text-fg-2">
              <span className="mono">{preview.commit.slice(0, 10)}</span>{" "}
              {preview.subject} · {preview.author}. The container gets this
              commit and its history ({preview.history_commits} commits).
              Uncommitted changes, other branches, stashes, hooks, remotes, and
              Git settings stay on this Mac.
            </span>
          ) : previewError ? (
            <span className="text-[12px] text-conflict">{previewError}</span>
          ) : (
            <span className="text-[12px] text-muted">Resolving…</span>
          )}
        </label>
        <label className="flex flex-col gap-1">
          <span className="font-medium">Prompt</span>
          <textarea
            className="field min-h-[100px] resize-y"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="What should the agent do?"
          />
        </label>
        <fieldset className="m-0 flex flex-col gap-1 border-0 p-0">
          <legend className="mb-1 font-medium">Permissions</legend>
          <label className="flex items-start gap-2">
            <input
              type="radio"
              name={`${id}-perm`}
              checked={permissions === "ask"}
              onChange={() => setPermissions("ask")}
            />
            <span>
              <span className="font-medium">Ask before actions</span>
              <span className="block text-[12px] text-muted">
                The agent waits for you before it runs a command or edits a
                file.
              </span>
            </span>
          </label>
          <label className="flex items-start gap-2">
            <input
              type="radio"
              name={`${id}-perm`}
              checked={permissions === "act"}
              onChange={() => setPermissions("act")}
            />
            <span>
              <span className="font-medium">Act without asking</span>
              <span className="block text-[12px] text-muted">
                Anything inside its container; never push, get new credentials,
                or change where it runs.
              </span>
            </span>
          </label>
        </fieldset>
        <div className="flex flex-wrap items-end gap-3">
          <label className="flex flex-col gap-1">
            <span className="font-medium">Time limit</span>
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
          <span className="pb-1.5 text-[12px] text-muted">
            Ends at {ends}. Waiting for you and idle time count.
          </span>
          <span className="flex-1" />
          <span className="pb-1.5 text-[12px] text-fg-2">
            {cpus} CPUs · {memoryGb} GB memory · {workspaceGb} GB workspace
          </span>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => setLimits(!limits)}
          >
            Change…
          </button>
        </div>
        {limits && (
          <div className="flex flex-wrap gap-3">
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
        <div className="rounded-lg border px-3 py-2 text-[12px] text-fg-2">
          <p className="m-0 font-medium text-fg">Before you start</p>
          <ul className="m-0 pl-5">
            <li>
              Code and prompts go to {destinationLabel(payment)} (
              {paymentLabel(payment)}) during the run.
            </li>
            <li>
              The network is unrestricted: the agent can reach any site,
              including services on your network.
            </li>
            <li>
              The token or key can be read by the agent, by the repository's
              code and its Claude Code settings, and by whoever controls the
              engine. Run only repositories you would trust with it.
            </li>
          </ul>
        </div>
      </div>
    </Dialog>
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
