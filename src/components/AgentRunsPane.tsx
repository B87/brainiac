import { useEffect, useState } from "react";
import { profileName } from "../lib/agentRuns";
import {
  type AgentProfile,
  type AgentSettings,
  errorMessage,
  ipc,
} from "../lib/ipc";
import { sourceLabel } from "../lib/secrets";
import AgentProfilePage from "./AgentProfilePage";
import { HostPill } from "./HostJobView";
import { Hint, PageTitle } from "./SettingsPanes";

/**
 * Settings → Agents (SPEC.md, Agent runs — v0.5): the agents, each an agent
 * and a model provider with its own page (how it is paid for, new runs'
 * defaults). Run hosts have a page of their own (`RunHostsPane`). The
 * token or key is read only when a run starts; this pane never shows it.
 */
export default function AgentRunsPane({
  onHost,
  profileId,
  onProfile,
}: {
  /** Opens a run host's page (Settings › Run hosts). */
  onHost: (hostId: string | null) => void;
  /** The profile whose page is open (Agents › profile), or none. */
  profileId?: string | null;
  onProfile: (profileId: string | null) => void;
}) {
  const [settings, setSettings] = useState<AgentSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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
  useEffect(() => setError(null), [profileId]);

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

  const title = (
    <PageTitle title="Agents">
      An agent is the AI a run uses and who pays for it: an engine, the provider
      its code and prompts go to, its key, model, and defaults. Where a run goes
      and what it can reach are chosen by the run or routine.
    </PageTitle>
  );
  if (!settings) {
    return (
      <>
        {!profileId && title}
        {error ? (
          <div role="alert" className="text-[12.5px] text-conflict">
            {error}
          </div>
        ) : (
          <Hint>Loading…</Hint>
        )}
      </>
    );
  }

  const profile = profileId
    ? settings.profiles.find((p) => p.id === profileId)
    : null;
  const errorLine = error && (
    <div role="alert" className="text-[12.5px] text-conflict">
      {error}
    </div>
  );
  if (profile) {
    return (
      <>
        {errorLine}
        <AgentProfilePage
          key={profile.id}
          profile={profile}
          settings={settings}
          busy={busy}
          act={act}
          onBack={() => onProfile(null)}
          onHost={onHost}
        />
      </>
    );
  }

  return (
    <>
      {title}
      {errorLine}
      <ul aria-label="Agents" className="settings-group m-0 list-none p-0">
        {settings.profiles.map((p) => (
          <ProfileRow key={p.id} profile={p} onOpen={() => onProfile(p.id)} />
        ))}
      </ul>
    </>
  );
}

/** An agent in Agents: Ready, or the first thing it still lacks. */
function ProfileRow({
  profile,
  onOpen,
}: {
  profile: AgentProfile;
  onOpen: () => void;
}) {
  const name = profileName(profile);
  const first = profile.missing[0];
  const has = profile.credential_source.kind !== "none";
  const summary = [
    profile.payment === "claude_plan" ? "Claude plan" : "API key",
    has && sourceLabel(profile.credential_source),
    profile.model,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <li className="border-line-soft border-b last:border-b-0">
      <button
        type="button"
        className="flex w-full items-center gap-3 px-3.5 py-3 text-left hover:bg-control"
        aria-label={`${name}, ${first ?? "Ready"}`}
        onClick={onOpen}
      >
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-[13px] font-semibold">{name}</span>
          <span className="truncate text-[12px] text-muted">
            {first ?? summary}
          </span>
        </span>
        <HostPill
          label={first ? "Not ready" : "Ready"}
          tone={first ? "attention" : "ready"}
        />
        <span aria-hidden="true" className="text-[16px] text-muted">
          ›
        </span>
      </button>
    </li>
  );
}
