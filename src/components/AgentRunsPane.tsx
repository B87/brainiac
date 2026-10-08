import { useEffect, useState } from "react";
import { profileName } from "../lib/agentRuns";
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
  type AgentProfile,
  type AgentSettings,
  errorMessage,
  type HostJob,
  ipc,
} from "../lib/ipc";
import { sourceLabel } from "../lib/secrets";
import AddHostDialog from "./AddHostDialog";
import AgentProfilePage from "./AgentProfilePage";
import Dialog from "./Dialog";
import { HostPill, JobBar } from "./HostJobView";
import { LocalHostPage, RemoteHostPage } from "./RunHostPage";
import { Group, Hint, Lede } from "./SettingsPanes";

/**
 * Settings → Agents (SPEC.md, Agent runs — v0.5): the agents, each an agent
 * and a model provider with its own page (how it is paid for, the agreement
 * to send code, new runs' defaults), and the run hosts (This Mac and
 * approved Linux hosts, each with its own page, image, and tests). The
 * token or key is read only when a run starts; this pane never shows it.
 */
export default function AgentRunsPane({
  hostId,
  onHost,
  profileId,
  onProfile,
  onOpenRun,
}: {
  /** The host whose page is open (Agents › Run hosts › host), or none. */
  hostId?: string | null;
  onHost: (hostId: string | null) => void;
  /** The profile whose page is open (Agents › profile), or none. */
  profileId?: string | null;
  onProfile: (profileId: string | null) => void;
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
  useEffect(() => setError(null), [hostId, profileId]);

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

  const showDockerfile = () =>
    ipc
      .agentDockerfile()
      .then(setDockerfile)
      .catch((e) => setError(errorMessage(e)));

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
  const profile = profileId
    ? settings.profiles.find((p) => p.id === profileId)
    : null;
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
          onChooseEngine={(socket) =>
            void act(() => ipc.chooseAgentEngine(socket))
          }
          onViewDockerfile={showDockerfile}
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
        {errorLine}
        <RemoteHostPage
          host={open}
          profiles={settings.profiles}
          onBack={() => onHost(null)}
          onChanged={() => void reload()}
          onConfirmKey={(existing) => setAdding({ existing })}
          onViewDockerfile={showDockerfile}
          onOpenRun={onOpenRun}
        />
        {dialogs}
      </>
    );
  }
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
      <Lede>
        Hand a repository to a coding agent in a container. Its work comes back
        for you to review; nothing is pushed.
      </Lede>
      {errorLine}

      <Group label="Agents">
        <Hint>
          An agent and the provider its code and prompts go to, each with its
          own key, model, and defaults.
        </Hint>
        <ul aria-label="Agents" className="settings-group m-0 list-none p-0">
          {settings.profiles.map((p) => (
            <ProfileRow key={p.id} profile={p} onOpen={() => onProfile(p.id)} />
          ))}
        </ul>
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
