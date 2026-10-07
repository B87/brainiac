import { useEffect, useId, useState } from "react";
import {
  dayLabel,
  destinationLabel,
  durationLabel,
  MODEL_SUGGESTIONS,
  parseModel,
  paymentLabel,
  settingsRequest,
  TIME_LIMITS,
} from "../lib/agentRuns";
import type { SaveAgentSettingsRequest } from "../lib/generated/SaveAgentSettingsRequest";
import {
  hostState,
  hostSummary,
  jobElapsed,
  stepOf,
  useHostJobsContext,
  useNow,
} from "../lib/hostJobs";
import {
  type AgentHost,
  type AgentPayment,
  type AgentSettings,
  errorMessage,
  type HostJob,
  ipc,
} from "../lib/ipc";
import {
  commandPreview,
  draftOf,
  pendingLabel,
  type SourceDraft,
  type SourceKind,
  sourceLabel,
  sourceOf,
} from "../lib/secrets";
import { parseInRange } from "../lib/settings";
import AddHostDialog from "./AddHostDialog";
import Dialog from "./Dialog";
import { HostPill, JobBar } from "./HostJobView";
import { LocalHostPage, RemoteHostPage } from "./RunHostPage";
import SecretSourceFields from "./SecretSourceFields";
import { CommitField, Group, Hint, Lede } from "./SettingsPanes";

/**
 * Settings → Agents (SPEC.md, Agent runs — v0.5): how Claude Code is paid
 * for and the agreement to send code, the run hosts (This Mac and approved
 * Linux hosts, each with its own page), and new runs' defaults. The token
 * or key is read only when a run starts; this pane never shows it.
 */
export default function AgentRunsPane({
  hostId,
  onHost,
  onOpenRun,
}: {
  /** The host whose page is open (Agents › Run hosts › host), or none. */
  hostId?: string | null;
  onHost: (hostId: string | null) => void;
  onOpenRun?: (runId: string) => void;
}) {
  const [settings, setSettings] = useState<AgentSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [dockerfile, setDockerfile] = useState<string | null>(null);
  /** Add host…, or confirming a saved host's key again. */
  const [adding, setAdding] = useState<{ existing?: AgentHost } | null>(null);

  const reload = () =>
    ipc.getAgentSettings().then(setSettings, (e) => setError(errorMessage(e)));

  useEffect(() => {
    let alive = true;
    ipc
      .getAgentSettings()
      .then((s) => alive && setSettings(s))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, []);

  // A page opened from elsewhere starts without the pane's last error.
  // biome-ignore lint/correctness/useExhaustiveDependencies: clear on navigation only
  useEffect(() => setError(null), [hostId]);

  /** Run a change; the pane shows what it returned, or why it failed. */
  const act = async (action: () => Promise<AgentSettings>) => {
    setBusy(true);
    setError(null);
    try {
      setSettings(await action());
      return true;
    } catch (e) {
      setError(errorMessage(e));
      // A change made elsewhere: show the current state again.
      ipc.getAgentSettings().then(setSettings, () => {});
      return false;
    } finally {
      setBusy(false);
    }
  };

  if (!settings) {
    return error ? (
      <div role="alert" className="text-[12.5px] text-conflict">
        {error}
      </div>
    ) : (
      <Hint>Loading…</Hint>
    );
  }

  const profile = settings.profile;
  const save = (patch: Partial<SaveAgentSettingsRequest>) =>
    act(() => ipc.saveAgentSettings(settingsRequest(profile, patch)));

  const dialogs = (
    <>
      {dockerfile !== null && (
        <Dialog
          title="Dockerfile"
          width={720}
          onClose={() => setDockerfile(null)}
        >
          <pre className="mono selectable m-0 overflow-auto px-4 py-3 text-[11.5px] leading-relaxed">
            {dockerfile}
          </pre>
        </Dialog>
      )}
      {adding && (
        <AddHostDialog
          existing={adding.existing}
          onClose={() => setAdding(null)}
          onAdded={(id) => {
            setAdding(null);
            void reload();
            onHost(id);
          }}
        />
      )}
    </>
  );

  const open = hostId ? settings.hosts.find((h) => h.id === hostId) : null;
  const errorLine = error && (
    <div role="alert" className="text-[12.5px] text-conflict">
      {error}
    </div>
  );
  if (open?.kind === "local") {
    return (
      <>
        {errorLine}
        <LocalHostPage
          host={open}
          settings={settings}
          busy={busy}
          onBack={() => onHost(null)}
          onChooseEngine={(socket) => void save({ engine_socket: socket })}
          onSettings={setSettings}
          onError={setError}
        />
        {dialogs}
      </>
    );
  }
  if (open) {
    return (
      <>
        <RemoteHostPage
          host={open}
          onBack={() => onHost(null)}
          onChanged={() => void reload()}
          onConfirmKey={(existing) => setAdding({ existing })}
          onOpenRun={onOpenRun}
        />
        {dialogs}
      </>
    );
  }

  return (
    <>
      <Lede>
        Hand a repository to Claude Code in a container. Its work comes back for
        you to review; nothing is pushed.
      </Lede>
      {errorLine}

      {profile.credential.needs_approval && (
        <div className="settings-group">
          <div
            className="settings-row flex-col items-stretch gap-2"
            role="note"
          >
            <p className="m-0 text-[12.5px]">
              Restored from a backup: Brainiac does not read its token or key,
              build its image, or start a run until you confirm it.{" "}
              {profile.credential_source.kind === "none" ? (
                "It has no token or key yet."
              ) : (
                <>
                  It reads{" "}
                  <span className="mono">
                    {sourceLabel(profile.credential_source)}
                  </span>{" "}
                  and sends code to {destinationLabel(profile.payment)}.
                </>
              )}
              {profile.engine_socket && (
                <>
                  {" "}
                  Runs execute on the engine at{" "}
                  <span className="mono">{profile.engine_socket}</span>.
                </>
              )}
            </p>
            {profile.credential_source.kind === "command" && (
              <>
                <code className="mono selectable break-all rounded-md border bg-header px-2 py-1 text-[11.5px]">
                  {commandPreview(
                    profile.credential_source.program,
                    profile.credential_source.args,
                  )}
                </code>
                <p className="m-0 text-[12px] text-muted">
                  The program runs with your permissions, without a shell.
                  Confirm only if you recognize it and its arguments.
                </p>
              </>
            )}
            <button
              type="button"
              className="btn btn-sm self-start"
              disabled={busy}
              onClick={() =>
                void act(() =>
                  ipc.approveAgentSettings(
                    profile.id,
                    profile.credential.revision,
                  ),
                )
              }
            >
              Confirm This Setup
            </button>
          </div>
        </div>
      )}

      {settings.missing.length > 0 && (
        <Group label="Before the first run">
          <ul className="m-0 flex flex-col gap-1 pl-5 text-[12.5px] text-fg-2">
            {settings.missing.map((m) => (
              <li key={m}>{m}</li>
            ))}
          </ul>
        </Group>
      )}

      <Group label="Claude Code">
        <CredentialSection
          settings={settings}
          busy={busy}
          onSave={(request) => act(() => ipc.saveAgentCredential(request))}
          onRemove={() => act(() => ipc.removeAgentCredential(profile.version))}
          onRetry={() =>
            act(async () => {
              await ipc.retryCredentialCleanup({
                kind: "agent_profile",
                id: profile.id,
              });
              return ipc.getAgentSettings();
            })
          }
        />
        <div className="settings-group">
          <label className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">
                Send code and prompts from runs to{" "}
                {destinationLabel(profile.payment)}
              </span>
              <Hint>
                A run sends the repository's history up to its start commit,
                your prompts, and anything the agent reads. Reviewing a run
                decides what leaves Brainiac as a patch, not what is sent.
                {profile.payment === "claude_plan" &&
                  " Runs use your plan's usage limits, the same ones as Claude Code in your terminal, and your plan's terms apply."}
              </Hint>
            </span>
            <input
              type="checkbox"
              role="switch"
              className="switch"
              aria-checked={profile.sends_code_agreed}
              checked={profile.sends_code_agreed}
              disabled={busy}
              onChange={(e) =>
                void save({ sends_code_agreed: e.target.checked })
              }
            />
          </label>
          <div className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">Image recipe</span>
              <Hint>
                Claude Code and its ACP adapter, each at a pinned version, from
                a Dockerfile you can read. Each run host builds it for itself.
              </Hint>
            </span>
            <button
              type="button"
              className="btn btn-sm"
              onClick={() =>
                ipc
                  .agentDockerfile()
                  .then(setDockerfile)
                  .catch((e) => setError(errorMessage(e)))
              }
            >
              View Dockerfile
            </button>
          </div>
        </div>
      </Group>

      <Group label="Run hosts">
        <div className="flex flex-wrap items-end justify-between gap-3">
          <Hint>
            Where runs execute: this Mac's Docker engine, and Linux machines
            Brainiac installs its run controller on. Each builds the image and
            passes the test for itself.
          </Hint>
          <button
            type="button"
            className="btn btn-sm"
            disabled={profile.credential.needs_approval}
            onClick={() => setAdding({})}
          >
            Add Host…
          </button>
        </div>
        <ul aria-label="Run hosts" className="settings-group m-0 list-none p-0">
          {settings.hosts.map((host) => (
            <HostRow key={host.id} host={host} onOpen={() => onHost(host.id)} />
          ))}
        </ul>
        <Hint>
          A remote host's administrator can see the repository and the
          credential. Brainiac reaches it with your SSH setup and keeps no
          private key.
        </Hint>
      </Group>

      <Group label="New runs">
        <div className="settings-group">
          <div className="settings-row flex-col items-stretch gap-2.5">
            <fieldset
              aria-label="Permissions"
              className="seg m-0 self-start border-0"
            >
              {(
                [
                  ["ask", "Ask before actions"],
                  ["act", "Act without asking"],
                ] as const
              ).map(([value, label]) => (
                <button
                  key={value}
                  type="button"
                  aria-pressed={profile.permissions === value}
                  disabled={busy}
                  onClick={() => void save({ permissions: value })}
                >
                  {label}
                </button>
              ))}
            </fieldset>
            <Hint>
              {profile.permissions === "ask"
                ? "The agent waits for you before it runs a command or edits a file."
                : "The agent does anything inside its container without asking. It never pushes, gets new credentials, or changes where it runs."}
            </Hint>
          </div>
          <label className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">Time limit</span>
              <Hint>
                Counts idle time and waiting for you too. A run is stopped when
                it is reached.
              </Hint>
            </span>
            <select
              className="text-input w-auto"
              value={profile.time_limit_minutes}
              disabled={busy}
              onChange={(e) =>
                void save({ time_limit_minutes: Number(e.target.value) })
              }
            >
              {[...new Set([...TIME_LIMITS, profile.time_limit_minutes])]
                .sort((a, b) => a - b)
                .map((m) => (
                  <option key={m} value={m}>
                    {durationLabel(m)}
                  </option>
                ))}
            </select>
          </label>
          <CommitField
            label="Model"
            hint={
              <>
                An alias (opus, sonnet, haiku, opusplan) or a full model name;
                empty for Claude Code's default. The agent reports the model it
                opened with, so a name the plan or key cannot use shows there.
              </>
            }
            value={profile.model}
            format={(m) => m}
            parse={parseModel}
            onCommit={(model) => save({ model })}
            placeholder="Claude Code's default"
            suggestions={MODEL_SUGGESTIONS}
            mono
            width={180}
          />
          <CommitField
            label="CPUs"
            value={profile.cpus}
            format={String}
            parse={(t) => parseInRange(t, 1, 64)}
            onCommit={(cpus) => save({ cpus })}
            unit="CPUs"
            width={70}
          />
          <CommitField
            label="Memory"
            value={profile.memory_mib / 1024}
            format={String}
            parse={(t) => parseInRange(t, 2, 256)}
            onCommit={(gb) => save({ memory_mib: gb * 1024 })}
            unit="GB"
            width={70}
          />
          <CommitField
            label="Workspace"
            hint="The size of the run's working copy. Writes past it fail; the run is not stopped."
            value={profile.workspace_gib}
            format={String}
            parse={(t) => parseInRange(t, 1, 500)}
            onCommit={(workspace_gib) => save({ workspace_gib })}
            unit="GB"
            width={70}
          />
        </div>
      </Group>

      {dialogs}
    </>
  );
}

/** A host in Run hosts: its state, and a running job's step and time. */
function HostRow({ host, onOpen }: { host: AgentHost; onOpen: () => void }) {
  const jobs = useHostJobsContext();
  const job: HostJob | undefined = jobs[host.id];
  const running = job?.state === "running";
  const now = useNow(running);
  const state = hostState(host, job);
  const step = running ? job.steps.find((s) => s.state === "running") : null;
  return (
    <li className="border-line-soft border-b last:border-b-0">
      <button
        type="button"
        className="flex w-full flex-col gap-2.5 px-3.5 py-3 text-left hover:bg-control"
        aria-label={`${host.name}, ${state.label}`}
        onClick={onOpen}
      >
        <span className="flex w-full items-center gap-3">
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="text-[13px] font-semibold">{host.name}</span>
            <span className="truncate text-[12px] text-muted">
              {hostSummary(host) ||
                (host.kind === "local"
                  ? "Choose its Docker engine"
                  : "Not set up")}
            </span>
          </span>
          <HostPill label={state.label} tone={state.tone} />
          <span aria-hidden="true" className="text-[16px] text-muted">
            ›
          </span>
        </span>
        {running && job && (
          <span className="flex w-full flex-col gap-1.5">
            <JobBar job={job} />
            <span className="flex flex-wrap gap-x-3 gap-y-1 text-[12px]">
              <span className="text-fg-2">
                {step?.progress
                  ? `${step.title}: ${step.progress}`
                  : step?.title}
              </span>
              <span className="tabular text-muted">
                {jobElapsed(job, now)} so far · {stepOf(job)}
              </span>
            </span>
          </span>
        )}
      </button>
    </li>
  );
}

function CredentialSection({
  settings,
  busy,
  onSave,
  onRemove,
  onRetry,
}: {
  settings: AgentSettings;
  busy: boolean;
  onSave: (
    request: Parameters<typeof ipc.saveAgentCredential>[0],
  ) => Promise<boolean>;
  onRemove: () => Promise<boolean>;
  onRetry: () => Promise<boolean>;
}) {
  const id = useId();
  const profile = settings.profile;
  const has = profile.credential_source.kind !== "none";
  // The payment a form is open for, or null when none is.
  const [editing, setEditing] = useState<AgentPayment | null>(null);
  const [source, setSource] = useState<SourceDraft>(() =>
    draftOf(has ? profile.credential_source : null, "store"),
  );
  const [secret, setSecret] = useState("");

  const open = (payment: AgentPayment) => {
    setSource(draftOf(has ? profile.credential_source : null, "store"));
    setSecret("");
    setEditing(payment);
  };
  const close = () => {
    setEditing(null);
    setSecret("");
  };

  const payments: AgentPayment[] = settings.plan_offered
    ? ["claude_plan", "api_key"]
    : ["api_key"];
  const what = editing === "claude_plan" ? "token" : "API key";
  // The Keychain item holds a value for the saved payment only.
  const keepsItem =
    profile.credential_source.kind === "store" &&
    profile.payment === editing &&
    profile.credential.pending !== "save";
  const ready =
    source.kind === "store"
      ? secret.trim() !== "" || keepsItem
      : source.kind === "environment"
        ? source.name.trim() !== ""
        : source.program.trim() !== "";

  const submit = async () => {
    if (!editing || !ready) return;
    const saved = await onSave({
      expected_version: profile.version,
      payment: editing,
      source: sourceOf(source),
      secret: source.kind === "store" && secret.trim() ? secret : null,
    });
    if (saved) close();
  };

  return (
    <div className="settings-group">
      <div className="settings-row flex-col items-stretch gap-2.5">
        <div className="flex items-center gap-2">
          <span className="text-[12.5px] text-fg-2">Pay with</span>
          <fieldset aria-label="Pay with" className="seg m-0 border-0">
            {payments.map((p) => (
              <button
                key={p}
                type="button"
                aria-pressed={(editing ?? profile.payment) === p}
                disabled={busy}
                onClick={() => {
                  if (p !== profile.payment || editing !== null) open(p);
                }}
              >
                {paymentLabel(p)}
              </button>
            ))}
          </fieldset>
        </div>
        {editing === null && (
          <div className="flex items-start gap-2">
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="text-[12.5px]">
                {has
                  ? `${profile.payment === "claude_plan" ? "Token" : "API key"} · ${sourceLabel(profile.credential_source)}`
                  : profile.payment === "claude_plan"
                    ? "No token yet."
                    : "No API key yet."}
              </span>
              {has && profile.credential_saved_at && (
                <Hint>
                  Saved {dayLabel(profile.credential_saved_at)}
                  {profile.payment === "claude_plan" && "; tokens last a year"}.
                </Hint>
              )}
              {profile.credential_ageing && (
                <span role="note" className="text-[12px] text-conflict">
                  This token is eleven months old or more. Make a new one with
                  claude setup-token before it expires.
                </span>
              )}
            </div>
            <button
              type="button"
              className="btn btn-sm shrink-0"
              disabled={busy}
              onClick={() => open(profile.payment)}
            >
              {has ? "Change…" : "Add…"}
            </button>
            {has && (
              <button
                type="button"
                className="btn btn-sm shrink-0"
                disabled={busy}
                onClick={() => void onRemove()}
              >
                Remove
              </button>
            )}
          </div>
        )}
        {editing === null && profile.credential.pending && (
          <div className="flex items-center gap-2" role="note">
            <p className="m-0 flex-1 text-[12.5px]">
              {pendingLabel(profile.credential.pending)}
            </p>
            {profile.credential.pending !== "save" && (
              <button
                type="button"
                className="btn btn-sm"
                disabled={busy}
                onClick={() => void onRetry()}
              >
                Retry
              </button>
            )}
          </div>
        )}
        {editing !== null && (
          <form
            className="flex flex-col gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
          >
            <label className="flex flex-col gap-1" htmlFor={`${id}-source`}>
              <span className="text-[12px]">
                {editing === "claude_plan" ? "Token" : "API key"} from
              </span>
              <select
                id={`${id}-source`}
                className="text-input"
                value={source.kind}
                onChange={(e) =>
                  setSource({ ...source, kind: e.target.value as SourceKind })
                }
              >
                <option value="store">The Keychain</option>
                <option value="environment">An environment variable</option>
                <option value="command">A command</option>
              </select>
            </label>
            {source.kind === "store" && (
              <label className="flex flex-col gap-1" htmlFor={`${id}-secret`}>
                <span className="text-[12px]">
                  {editing === "claude_plan"
                    ? "Token from claude setup-token"
                    : "Anthropic API key"}
                </span>
                {/* A password field also drops the line breaks Terminal adds to a wrapped token. */}
                <input
                  id={`${id}-secret`}
                  type="password"
                  className="text-input mono"
                  value={secret}
                  spellCheck={false}
                  autoComplete="off"
                  placeholder={
                    keepsItem ? "Leave empty to keep the saved one" : undefined
                  }
                  onChange={(e) => setSecret(e.target.value)}
                />
              </label>
            )}
            <SecretSourceFields
              draft={source}
              onChange={setSource}
              what={what}
            />
            <Hint>
              {editing === "claude_plan"
                ? "Run claude setup-token in Terminal and paste the token it prints, even if Terminal wrapped it over two lines: Brainiac joins the pieces, drops the spaces the wrap adds, and refuses anything that does not look like one token. Brainiac never signs in to claude.ai for you. "
                : ""}
              Read when a run starts and handed to the agent in memory; never
              put in the container's settings, the image, logs, or Brainiac's
              files.
            </Hint>
            <div className="flex gap-2">
              <button
                type="submit"
                className="btn btn-sm btn-primary"
                disabled={busy || !ready}
              >
                Save
              </button>
              <button type="button" className="btn btn-sm" onClick={close}>
                Cancel
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}
