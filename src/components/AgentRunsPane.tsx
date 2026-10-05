import { useCallback, useEffect, useId, useState } from "react";
import {
  dayLabel,
  destinationLabel,
  durationLabel,
  engineSummary,
  imageSummary,
  paymentLabel,
  settingsRequest,
  TIME_LIMITS,
} from "../lib/agentRuns";
import type { SaveAgentSettingsRequest } from "../lib/generated/SaveAgentSettingsRequest";
import {
  type AgentEngine,
  type AgentPayment,
  type AgentSettings,
  type AgentTestResult,
  errorMessage,
  ipc,
  type RunControllerStatus,
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
import Dialog from "./Dialog";
import SecretSourceFields from "./SecretSourceFields";
import { CommitField, Group, Hint, Lede } from "./SettingsPanes";

/**
 * Settings → Agents (SPEC.md, Agent runs — v0.5): where runs execute, how
 * Claude Code is paid for, the agreement to send code, the image, and new
 * runs' defaults. The token or key is read only when a run starts; this
 * pane never shows it.
 */
export default function AgentRunsPane() {
  const [settings, setSettings] = useState<AgentSettings | null>(null);
  const [engines, setEngines] = useState<AgentEngine[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [building, setBuilding] = useState(false);
  const [dockerfile, setDockerfile] = useState<string | null>(null);
  const [controller, setController] = useState<RunControllerStatus | null>(
    null,
  );
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<AgentTestResult | null>(null);

  useEffect(() => {
    let alive = true;
    const load = () =>
      ipc
        .getRunControllerStatus()
        .then((c) => alive && setController(c))
        .catch(() => {});
    void load();
    const timer = setInterval(load, 10_000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  /** Test: minutes when a wrong token is retried; the pane stays usable. */
  const runTest = async () => {
    setTesting(true);
    setError(null);
    setTestResult(null);
    try {
      setTestResult(await ipc.testAgentSetup());
      setSettings(await ipc.getAgentSettings());
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setTesting(false);
    }
  };

  const loadEngines = useCallback(() => {
    setEngines(null);
    ipc
      .listAgentEngines()
      .then(setEngines)
      .catch((e) => setError(errorMessage(e)));
  }, []);

  useEffect(() => {
    let alive = true;
    ipc
      .getAgentSettings()
      .then((s) => alive && setSettings(s))
      .catch((e) => alive && setError(errorMessage(e)));
    loadEngines();
    return () => {
      alive = false;
    };
  }, [loadEngines]);

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

  // A build takes minutes: the rest of the pane stays usable meanwhile.
  const build = async () => {
    setBuilding(true);
    setError(null);
    try {
      setSettings(await ipc.buildAgentImage());
    } catch (e) {
      setError(errorMessage(e));
      ipc.getAgentSettings().then(setSettings, () => {});
    } finally {
      setBuilding(false);
    }
  };

  return (
    <>
      <Lede>
        Hand a repository to Claude Code running in a container on this Mac. Its
        work comes back for you to review; nothing is pushed.
      </Lede>
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}

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

      <Group label="Where runs execute">
        <div className="settings-group">
          {engines === null ? (
            <div className="settings-row">
              <Hint>Looking for Docker engines…</Hint>
            </div>
          ) : engines.length === 0 ? (
            <div className="settings-row">
              <Hint>
                No Docker engine was found on this Mac. Install and start
                OrbStack or Docker Desktop.
              </Hint>
            </div>
          ) : (
            engines.map((engine) => (
              <EngineRow
                key={engine.socket}
                engine={engine}
                chosen={profile.engine_socket === engine.socket}
                disabled={busy}
                onChoose={() => void save({ engine_socket: engine.socket })}
              />
            ))
          )}
        </div>
        <div className="flex items-start gap-2">
          <Hint>
            Brainiac starts and stops its own containers on this engine. Runs
            pause while this Mac sleeps; when it wakes, a run past its time
            limit is stopped. Choosing another engine means building the image
            there.{" "}
            {controller?.running
              ? `The run controller is running (process ${controller.pid}${
                  controller.live_runs
                    ? `, ${controller.live_runs} live ${controller.live_runs === 1 ? "run" : "runs"}`
                    : ""
                }).`
              : "The run controller is not running; it starts with the next run."}
          </Hint>
          <button
            type="button"
            className="btn btn-sm shrink-0"
            onClick={loadEngines}
          >
            Look Again
          </button>
        </div>
      </Group>

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
      </Group>

      <Group label="Sends code to">
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
        </div>
      </Group>

      <Group label="Image">
        <div className="settings-group">
          <div className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">
                {profile.image ? (
                  <span className="mono text-[12px]">
                    {imageSummary(profile.image)}
                  </span>
                ) : (
                  "Not built"
                )}
              </span>
              <Hint>
                Claude Code and its ACP adapter, each at a pinned version, from
                a Dockerfile you can read.
                {profile.image && ` Built ${dayLabel(profile.image.built_at)}.`}
                {profile.image &&
                  !profile.image.current &&
                  " This version of Brainiac changed the Dockerfile: rebuild it."}
                {building &&
                  " Building downloads the base image and packages; it takes a few minutes."}
              </Hint>
            </span>
            <span className="flex shrink-0 gap-2">
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
              <button
                type="button"
                className="btn btn-sm"
                disabled={
                  building ||
                  busy ||
                  !profile.engine_socket ||
                  profile.credential.needs_approval
                }
                onClick={() => void build()}
              >
                {building
                  ? "Building…"
                  : profile.image
                    ? "Rebuild"
                    : "Build Image"}
              </button>
            </span>
          </div>
        </div>
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

      <Group label="Test">
        <div className="settings-group">
          <div className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">
                {profile.test_passed_at && profile.test_current
                  ? `Passed on ${dayLabel(profile.test_passed_at)}`
                  : profile.test_passed_at
                    ? `Passed on ${dayLabel(profile.test_passed_at)}, before the token or key, the image, or the engine changed`
                    : "Not tested"}
              </span>
              <Hint>
                A test starts a short run, sends a prompt, cancels, and
                collects. A run cannot start until one passes for this token or
                key, image, and engine: a wrong token can come back looking like
                an ordinary reply. A wrong token or key can take a few minutes
                to be refused: Claude Code retries it first.
              </Hint>
            </span>
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy || testing}
              onClick={() => void runTest()}
            >
              {testing ? "Testing…" : "Test"}
            </button>
          </div>
          {testResult && (
            <div className="settings-row flex-col items-stretch gap-1">
              {testResult.steps.map((step) => (
                <div key={step.name} className="flex gap-2 text-[12.5px]">
                  <span
                    className={step.passed ? "text-added" : "text-conflict"}
                  >
                    {step.passed ? "✓" : "✕"}
                  </span>
                  <span>{step.name}</span>
                  {step.detail && (
                    <span className="truncate text-muted">{step.detail}</span>
                  )}
                </div>
              ))}
              <Hint>
                {testResult.passed
                  ? `Passed at ${dayLabel(testResult.tested_at)}.`
                  : "The test did not pass; runs cannot start yet."}
              </Hint>
            </div>
          )}
        </div>
      </Group>

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
    </>
  );
}

function EngineRow({
  engine,
  chosen,
  disabled,
  onChoose,
}: {
  engine: AgentEngine;
  chosen: boolean;
  disabled: boolean;
  onChoose: () => void;
}) {
  const id = useId();
  const usable = engine.reachable && engine.supported;
  return (
    <label className="settings-row" htmlFor={id}>
      <input
        id={id}
        type="radio"
        name="agent-engine"
        checked={chosen}
        disabled={disabled || (!usable && !chosen)}
        onChange={onChoose}
      />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="font-medium">
          {engine.name}
          {usable ? " · Ready" : ""}
        </span>
        <span className="mono truncate text-[11.5px] text-muted">
          {engine.socket}
        </span>
        <span
          className={`text-[12px] ${usable ? "text-fg-2" : "text-conflict"}`}
        >
          {usable
            ? engineSummary(engine)
            : (engine.problem ?? "Not answering.")}
        </span>
      </span>
    </label>
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
                ? "Run claude setup-token in Terminal and paste the token it prints, even if Terminal wrapped it over two lines: Brainiac joins the lines and refuses anything that is not one token. Brainiac never signs in to claude.ai for you. "
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
