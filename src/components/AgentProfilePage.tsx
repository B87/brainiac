import { useId, useState } from "react";
import {
  credentialWord,
  dayLabel,
  destinationLabel,
  durationLabel,
  keyLabel,
  modelHint,
  modelPlaceholder,
  modelSuggestions,
  parseModel,
  paymentLabel,
  paymentsOffered,
  profileName,
  settingsRequest,
  TIME_LIMITS,
} from "../lib/agentRuns";
import type { SaveAgentSettingsRequest } from "../lib/generated/SaveAgentSettingsRequest";
import {
  type AgentPayment,
  type AgentProfile,
  type AgentSettings,
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
import { HostPill } from "./HostJobView";
import { Breadcrumb } from "./RunHostPage";
import SecretSourceFields from "./SecretSourceFields";
import { CommitField, Group, Hint } from "./SettingsPanes";

/**
 * Agents › a profile (SPEC.md, Settings → Agents): how one agent and model
 * provider is paid for, the agreement to send code there, and its new
 * runs' defaults. Its tests are on each run host's page.
 */
export default function AgentProfilePage({
  profile,
  settings,
  busy,
  act,
  onBack,
  onHost,
}: {
  profile: AgentProfile;
  settings: AgentSettings;
  busy: boolean;
  /** Runs a change; the pane shows what it returned, or why it failed. */
  act: (action: () => Promise<AgentSettings>) => Promise<boolean>;
  onBack: () => void;
  onHost: (hostId: string) => void;
}) {
  const name = profileName(profile);
  const save = (patch: Partial<SaveAgentSettingsRequest>) =>
    act(() => ipc.saveAgentSettings(settingsRequest(profile, patch)));
  const tested = settings.hosts.filter((h) =>
    h.tests.some((t) => t.profile_id === profile.id && t.current),
  );

  return (
    <div className="flex flex-col gap-5">
      <Breadcrumb name={name} onBack={onBack} />
      <header className="flex min-w-0 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2.5">
          <h2 className="m-0 text-[19px] font-semibold">{name}</h2>
          {profile.missing.length === 0 ? (
            <HostPill label="Ready" tone="ready" />
          ) : (
            <HostPill label="Not ready" tone="attention" />
          )}
        </div>
        <span className="text-[12.5px] text-muted">
          Sends code to {destinationLabel(profile)}.{" "}
          {tested.length > 0 ? "Tested on " : "Not tested on any host yet."}
          {tested.map((h, i) => (
            <span key={h.id}>
              {i > 0 && ", "}
              <button
                type="button"
                className="text-link hover:underline"
                onClick={() => onHost(h.id)}
              >
                {h.name}
              </button>
            </span>
          ))}
          {tested.length > 0 && "."}
        </span>
      </header>

      {profile.credential.needs_approval && (
        <div className="settings-group">
          <div
            className="settings-row flex-col items-stretch gap-2"
            role="note"
          >
            <p className="m-0 text-[12.5px]">
              Restored from a backup: Brainiac does not read its token or key,
              or start a run with it, until you confirm it.{" "}
              {profile.credential_source.kind === "none" ? (
                "It has no token or key yet."
              ) : (
                <>
                  It reads{" "}
                  <span className="mono">
                    {sourceLabel(profile.credential_source)}
                  </span>{" "}
                  and sends code to {destinationLabel(profile)}.
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

      {profile.missing.length > 0 && (
        <Group label="Before the first run">
          <ul className="m-0 flex flex-col gap-1 pl-5 text-[12.5px] text-fg-2">
            {profile.missing.map((m) => (
              <li key={m}>{m}</li>
            ))}
          </ul>
        </Group>
      )}

      <Group label="Pay with">
        <CredentialSection
          profile={profile}
          planOffered={settings.plan_offered}
          busy={busy}
          onSave={(request) => act(() => ipc.saveAgentCredential(request))}
          onRemove={() =>
            act(() => ipc.removeAgentCredential(profile.id, profile.version))
          }
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
                Send code and prompts from runs to {destinationLabel(profile)}
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
            hint={modelHint(profile)}
            value={profile.model}
            format={(m) => m}
            parse={(text) => parseModel(profile.agent, text)}
            onCommit={(model) => save({ model })}
            placeholder={modelPlaceholder(profile.agent)}
            suggestions={modelSuggestions(profile)}
            mono
            width={220}
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
    </div>
  );
}

/** Pay with: the plan or the key, where it is read from, and its form. */
function CredentialSection({
  profile,
  planOffered,
  busy,
  onSave,
  onRemove,
  onRetry,
}: {
  profile: AgentProfile;
  planOffered: boolean;
  busy: boolean;
  onSave: (
    request: Parameters<typeof ipc.saveAgentCredential>[0],
  ) => Promise<boolean>;
  onRemove: () => Promise<boolean>;
  onRetry: () => Promise<boolean>;
}) {
  const id = useId();
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

  const payments = paymentsOffered(profile, planOffered);
  const what = editing === "claude_plan" ? "token" : "API key";
  const credentialName = (payment: AgentPayment) =>
    payment === "claude_plan" ? "Token" : "API key";
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
      profile_id: profile.id,
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
        {payments.length > 1 ? (
          <div className="flex items-center gap-2">
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
                  {paymentLabel(profile, p)}
                </button>
              ))}
            </fieldset>
          </div>
        ) : (
          <span className="text-[12.5px] text-fg-2">
            {paymentLabel(profile, payments[0])}
          </span>
        )}
        {editing === null && (
          <div className="flex items-start gap-2">
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="text-[12.5px]">
                {has
                  ? `${credentialName(profile.payment)} · ${sourceLabel(profile.credential_source)}`
                  : `No ${credentialWord(profile.payment)} yet.`}
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
                {credentialName(editing)} from
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
                    : keyLabel(profile.provider)}
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
