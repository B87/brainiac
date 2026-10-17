import { type ReactNode, useCallback, useEffect, useId, useState } from "react";
import {
  activityLabel,
  dayLabel,
  engineSummary,
  imageSummary,
  profileName,
  profileTestState,
} from "../lib/agentRuns";
import {
  currentStep,
  type HostTone,
  hostState,
  useHostJobsContext,
} from "../lib/hostJobs";
import {
  type AgentEngine,
  type AgentHost,
  type AgentProfile,
  type AgentRun,
  type AgentSettings,
  errorMessage,
  type HostJobKind,
  ipc,
  type RunControllerStatus,
} from "../lib/ipc";
import HostHealthCard, { healthPill, useHostHealth } from "./HostHealthGroup";
import HostJobView, { HostPill } from "./HostJobView";
import HostSchedulesGroup from "./HostSchedulesGroup";
import HostSecretsGroup from "./HostSecretsGroup";
import { Card, Group, Hint, PageTitle } from "./SettingsPanes";

/**
 * A host page's title row: "Run hosts ›", the host's name and one state,
 * with its actions on the right and what the page is for underneath.
 */
function PageHeader({
  host,
  subtitle,
  actions,
  pill,
  onBack,
}: {
  host: AgentHost;
  subtitle: ReactNode;
  actions?: ReactNode;
  /** What the pill says; the host's setup state unless given. */
  pill?: { label: string; tone: HostTone };
  onBack: () => void;
}) {
  const jobs = useHostJobsContext();
  const state = pill ?? hostState(host, jobs[host.id]);
  return (
    <PageTitle
      title={host.name}
      crumb={{ label: "Run hosts", onBack }}
      pill={<HostPill label={state.label} tone={state.tone} />}
      actions={actions}
    >
      {subtitle}
    </PageTitle>
  );
}

function Check({ done, warn = false }: { done: boolean; warn?: boolean }) {
  if (warn)
    return (
      <span
        role="img"
        aria-label="Needs attention"
        className="flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-dirty/20 text-[11px] font-bold text-dirty"
      >
        !
      </span>
    );
  return done ? (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--clean)"
      strokeWidth="2.5"
      role="img"
      aria-label="Done"
      className="shrink-0"
    >
      <path d="M5 12.5 10 17 19 7" />
    </svg>
  ) : (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--faint)"
      strokeWidth="2"
      role="img"
      aria-label="Not yet"
      className="shrink-0"
    >
      <circle cx="12" cy="12" r="7" />
    </svg>
  );
}

function SetupRow({
  done,
  warn,
  label,
  title,
  detail,
  children,
}: {
  done: boolean;
  /** Not wrong, but to see to: an older controller, an engine that cannot run. */
  warn?: boolean;
  /** The row's name for assistive technology, when the title varies. */
  label?: string;
  title: string;
  detail: ReactNode;
  children?: ReactNode;
}) {
  return (
    <li className="settings-row" aria-label={label}>
      <Check done={done} warn={warn} />
      <span className="flex min-w-55 flex-1 flex-col gap-0.5">
        <span className="font-medium">{title}</span>
        <Hint>{detail}</Hint>
      </span>
      {children}
    </li>
  );
}

/** The runs on one host that are not over, refreshed while the page is open. */
function useLiveRuns(hostId: string): AgentRun[] {
  const [runs, setRuns] = useState<AgentRun[]>([]);
  useEffect(() => {
    let alive = true;
    const load = () =>
      ipc
        .listAgentRuns()
        .then(
          (list) =>
            alive &&
            setRuns(
              list.runs.filter(
                (r) => r.host_id === hostId && r.phase !== "ended",
              ),
            ),
        )
        .catch(() => {});
    void load();
    const timer = setInterval(load, 10_000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [hostId]);
  return runs;
}

/** What the host still lacks before any run, in the order to do it. */
function HostMissing({ host }: { host: AgentHost }) {
  if (host.missing.length === 0) return null;
  return (
    <Group label="Before the first run">
      <ul className="m-0 flex flex-col gap-1 pl-5 text-[12.5px] text-fg-2">
        {host.missing.map((m) => (
          <li key={m}>{m}</li>
        ))}
      </ul>
    </Group>
  );
}

/** The image's row: what it holds, when it was built, and its Dockerfile. */
function ImageRow({
  host,
  building,
  onViewDockerfile,
  children,
}: {
  host: AgentHost;
  building?: boolean;
  onViewDockerfile: () => void;
  children: ReactNode;
}) {
  const recipe = "Claude Code, its ACP adapter, and OpenCode";
  const tools = host.tools_image;
  return (
    <SetupRow
      done={!!host.image?.current && (!tools || tools.current)}
      warn={
        !!host.image && (!host.image.current || (!!tools && !tools.current))
      }
      title={
        building
          ? "Building the image"
          : host.image
            ? tools?.current
              ? "Image with Python and the Google Cloud CLI"
              : "Image"
            : "No image yet"
      }
      detail={
        building ? (
          "Downloads the base image and packages; it takes a few minutes."
        ) : host.image ? (
          <>
            <span className="mono">{imageSummary(host.image)}</span> · built{" "}
            {dayLabel(host.image.built_at)}
            {!host.image.current &&
              " · this Brainiac changed the Dockerfile: rebuild it"}
            {tools &&
              !tools.current &&
              " · its Python and Google Cloud CLI layer is out of date: rebuild it"}
          </>
        ) : (
          `${recipe}, each at a pinned version.`
        )
      }
    >
      <button
        type="button"
        className="btn btn-sm btn-ghost text-link"
        title={`${recipe}, from a Dockerfile you can read`}
        onClick={onViewDockerfile}
      >
        View Dockerfile
      </button>
      {children}
    </SetupRow>
  );
}

/**
 * Which agents passed their test here (SPEC.md, Settings → Agents, Test):
 * one line, each name opening that agent's page, where Test is. A test is
 * of an agent, so it is run from the agent; the host only says what it can
 * take.
 */
function TestedAgents({
  host,
  profiles,
  onAgent,
}: {
  host: AgentHost;
  profiles: AgentProfile[];
  onAgent?: (profileId: string) => void;
}) {
  const passed = profiles.filter(
    (p) => profileTestState(p, host).kind === "passed",
  );
  const rest = profiles.length - passed.length;
  return (
    <SetupRow
      done={passed.length > 0}
      label="Agents tested here"
      title="Agents tested here"
      detail={
        <>
          {passed.length === 0 && "None yet. "}
          {passed.map((p, i) => (
            <span key={p.id}>
              {i > 0 && ", "}
              {onAgent ? (
                <button
                  type="button"
                  className="text-link hover:underline"
                  onClick={() => onAgent(p.id)}
                >
                  {profileName(p)}
                </button>
              ) : (
                profileName(p)
              )}
            </span>
          ))}
          {passed.length > 0 && rest > 0 && " · "}
          {rest > 0 &&
            `${rest} ${rest === 1 ? "agent" : "agents"} not tested here. `}
          A run of an agent starts here once its test passes; Test is on the
          agent's page, in Agents.
        </>
      }
    />
  );
}

const CANCEL_LABEL: Partial<Record<HostJobKind, string>> = {
  install: "Cancel install",
  upgrade: "Cancel upgrade",
  setup: "Cancel setup",
  prepare: "Cancel preparation",
};

/**
 * A remote host's page (SPEC.md, Remote hosts, A host's page): the machine
 * only. Its header says whether it answers; The machine lists what it has
 * and lacks, Health its numbers, Routines on this host what it fires, Held
 * here what it keeps. What runs here, with what access, is set on each
 * routine or run, never here.
 */
export function RemoteHostPage({
  host,
  profiles,
  onBack,
  onChanged,
  onConfirmKey,
  onViewDockerfile,
  onOpenRun,
  onSecrets,
  onAgent,
}: {
  host: AgentHost;
  profiles: AgentProfile[];
  onBack: () => void;
  onChanged: () => void;
  /** Opens Add host for this host, to approve its key again. */
  onConfirmKey: (host: AgentHost) => void;
  onViewDockerfile: () => void;
  onOpenRun?: (runId: string) => void;
  onSecrets?: () => void;
  /** Opens an agent's page, where its test on this host is run. */
  onAgent?: (profileId: string) => void;
}) {
  const jobs = useHostJobsContext();
  const job = jobs[host.id];
  const running = job?.state === "running";
  const live = useLiveRuns(host.id);
  const settled = host.installed && host.approved && !running;
  const health = useHostHealth(host.id, settled);
  const [error, setError] = useState<string | null>(null);
  const [showKey, setShowKey] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);

  // A job that ends changes the host's row: read it again.
  const jobState = job?.state;
  // biome-ignore lint/correctness/useExhaustiveDependencies: reload when the job's state changes
  useEffect(() => {
    if (jobState && jobState !== "running") onChanged();
  }, [jobState]);

  /** A job; Test is of one profile. */
  const start = async (kind: HostJobKind, profileId?: string) => {
    setError(null);
    setBusy(true);
    try {
      await ipc.startAgentHostJob(host.id, kind, profileId);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const cancel = async () => {
    setError(null);
    try {
      await ipc.cancelAgentHostJob(host.id);
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  const remove = async () => {
    setError(null);
    setBusy(true);
    try {
      await ipc.removeAgentHost(host.id);
      onChanged();
      onBack();
    } catch (e) {
      setError(errorMessage(e));
      setRemoving(false);
    } finally {
      setBusy(false);
    }
  };

  const blocked = busy || running || !host.approved;
  const failed =
    job && (job.state === "failed" || job.state === "interrupted") && !running;
  const address = `${host.ssh_user}@${host.ssh_host}:${host.ssh_port ?? 22}`;
  // The header's pill: what the host lacks, or whether it answers.
  const setup = hostState(host, job);
  const pill =
    setup.tone === "ready" && health.health ? healthPill(health.health) : setup;
  const reachable = health.health?.reachable;
  const buildingImage =
    running &&
    job?.steps[currentStep(job)]?.title.startsWith("Build the image");

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        host={host}
        onBack={onBack}
        pill={pill}
        subtitle="The machine only. What runs here, with what access, is set on each routine or run."
        actions={
          <>
            {settled && (
              <button
                type="button"
                className="btn btn-sm"
                disabled={health.busy}
                onClick={() => void health.check()}
              >
                {health.busy ? "Checking…" : "Check Now"}
              </button>
            )}
            {running && job && CANCEL_LABEL[job.kind] && (
              <button
                type="button"
                className="btn btn-sm"
                disabled={!job.cancellable}
                title={
                  job.cancellable
                    ? undefined
                    : "The install has begun; it finishes on its own."
                }
                onClick={() => void cancel()}
              >
                {CANCEL_LABEL[job.kind]}
              </button>
            )}
            {failed && job && (
              <button
                type="button"
                className="btn btn-sm btn-primary"
                disabled={busy}
                onClick={() =>
                  // A test, or a routine's preparation, is again of its agent.
                  void start(
                    job.kind,
                    job.kind === "test" || job.kind === "prepare"
                      ? job.profile_ids[0]
                      : undefined,
                  )
                }
              >
                Try Again
              </button>
            )}
          </>
        }
      />
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}

      {!host.approved && (
        <Banner tone="amber" title="Confirm this host's key">
          <p className="m-0">
            Restored from a backup, or its key changed: Brainiac installs
            nothing and starts no run on {host.name} until you confirm the key
            it presents.
          </p>
          <div>
            <button
              type="button"
              className="btn btn-sm btn-primary"
              onClick={() => onConfirmKey(host)}
            >
              Confirm Host Key…
            </button>
          </div>
        </Banner>
      )}

      {host.approved && !host.installed && !running && !failed && (
        <Banner tone="blue" title={`${host.name} is not set up`}>
          <p className="m-0">
            Install puts Brainiac's run controller on it, then the image is
            built and a test runs, one after another. You can leave Settings
            meanwhile.
          </p>
          <div>
            <button
              type="button"
              className="btn btn-sm btn-primary"
              disabled={busy}
              onClick={() => void start("setup")}
            >
              Install, Build Image, and Test
            </button>
          </div>
        </Banner>
      )}

      {/* A job that succeeded says so once, in the window; the rows below
          show what it changed. One that failed or stopped stays, with its log. */}
      {job && job.state !== "succeeded" && <HostJobView job={job} />}

      {host.approved && host.installed && <HostMissing host={host} />}

      {running &&
        job &&
        (job.kind === "upgrade" ||
          job.kind === "install" ||
          job.kind === "setup" ||
          job.kind === "prepare") && (
          <Group label="Meanwhile">
            <ul className="m-0 flex list-disc flex-col gap-1.5 rounded-[10px] border bg-app py-3 pr-4 pl-8 text-[12.5px] leading-relaxed text-fg-2">
              <li>
                {live.length > 0
                  ? `${live.length === 1 ? "A run is" : `${live.length} runs are`} live on ${host.name}; the restart waits for ${live.length === 1 ? "it" : "them"}.`
                  : `No run is live on ${host.name}.`}{" "}
                New runs there wait until this ends; This Mac still takes them.
              </li>
              <li>
                You can leave Settings or close this window. The job goes on,
                and Brainiac tells you when it ends.
              </li>
              <li>
                {host.controller_from_release
                  ? "The run controller is the one this version of Brainiac was released with, downloaded once from GitHub and checked against the SHA-256 the app carries."
                  : "The first build after a Brainiac update compiles the Linux run controller on this Mac, in Docker, imitating the host's processor when it differs. Brainiac keeps that build's cache, so later builds recompile only what changed."}
              </li>
            </ul>
          </Group>
        )}

      <div className="flex flex-wrap items-start gap-4">
        <Card title="The machine" className="min-w-[420px] flex-1">
          <ol className="m-0 flex list-none flex-col p-0">
            <SetupRow
              done={host.approved && reachable !== false}
              title={
                reachable === true
                  ? "Connected"
                  : reachable === false
                    ? "Not answering"
                    : host.approved
                      ? "Key trusted"
                      : "Key to confirm"
              }
              detail={
                <>
                  <span className="mono">{address}</span>
                  {host.fingerprint &&
                    ` · ${host.approved ? "key trusted" : "key to confirm"}${
                      showKey ? `: ${host.fingerprint}` : ""
                    }`}
                </>
              }
            >
              {host.fingerprint && (
                <button
                  type="button"
                  className="btn btn-sm btn-ghost text-link"
                  onClick={() => setShowKey(!showKey)}
                >
                  {showKey ? "Hide Key" : "Show Key"}
                </button>
              )}
            </SetupRow>
            <SetupRow
              done={host.installed && !host.upgrade_available}
              warn={host.installed && host.upgrade_available}
              label="Run controller"
              title={
                host.installed
                  ? `Run controller ${host.controller_build ?? "(an earlier build)"}`
                  : "Run controller not installed"
              }
              detail={
                host.upgrade_available
                  ? `Older than this Brainiac, which ${host.controller_from_release ? "brings" : "builds"} ${host.available_build ?? "a newer one"}. The next approval of a routine here upgrades it, or do it now; upgrading restarts the controller, so it waits while a run is live${
                      live.length > 0
                        ? `: ${live.length === 1 ? `“${live[0].title}” is live` : `${live.length} runs are live`} there now`
                        : ""
                    }.`
                  : host.installed
                    ? [
                        host.protocol != null && `protocol ${host.protocol}`,
                        "systemd service brainiac-runner",
                        host.controller_installed_at &&
                          `installed ${dayLabel(host.controller_installed_at)}`,
                      ]
                        .filter(Boolean)
                        .join(" · ")
                    : "Install puts it on the host as a systemd service, with the sudo actions you approved."
              }
            >
              {host.upgrade_available && host.approved && !running && (
                <button
                  type="button"
                  className="btn btn-sm btn-primary"
                  disabled={busy}
                  onClick={() => void start("upgrade")}
                >
                  {live.length > 0 ? "Upgrade When Runs End" : "Upgrade"}
                </button>
              )}
              <button
                type="button"
                className="btn btn-sm btn-ghost text-link"
                disabled={blocked}
                onClick={() => void start("install")}
              >
                {host.installed ? "Reinstall" : "Install"}
              </button>
            </SetupRow>
            <SetupRow
              done={!!host.engine_name && host.loop_devices}
              warn={!!host.engine_name && !host.loop_devices}
              title={host.engine_name ?? "Docker"}
              detail={
                host.engine_name
                  ? host.loop_devices
                    ? "Runs start in containers on it."
                    : "It cannot attach the workspace's loop devices, so no run can start."
                  : "Known once the image is built there."
              }
            />
            <ImageRow
              host={host}
              building={buildingImage}
              onViewDockerfile={onViewDockerfile}
            >
              <button
                type="button"
                className="btn btn-sm btn-ghost text-link"
                disabled={blocked || !host.installed}
                onClick={() => void start("build_image")}
              >
                {host.image ? "Rebuild" : "Build Image"}
              </button>
            </ImageRow>
            <TestedAgents host={host} profiles={profiles} onAgent={onAgent} />
          </ol>
        </Card>
        {settled && (
          <HostHealthCard
            key={`health-${host.id}`}
            state={health}
            className="w-[300px] flex-none"
          />
        )}
      </div>

      {settled && (
        <HostSchedulesGroup key={`schedules-${host.id}`} host={host} />
      )}

      {settled && (
        <HostSecretsGroup
          key={`secrets-${host.id}`}
          host={host}
          onSecrets={onSecrets}
        />
      )}

      {live.length > 0 && (
        <Card title={`Runs on ${host.name}`}>
          {live.map((run) => (
            <div key={run.id} className="settings-row">
              <span className="min-w-0 flex-1 truncate">{run.title}</span>
              <Hint>{activityLabel(run)}</Hint>
              {onOpenRun && (
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => onOpenRun(run.id)}
                >
                  Open
                </button>
              )}
            </div>
          ))}
        </Card>
      )}

      {host.emergency_stop && (
        <details className="rounded-[10px] border bg-app px-4 py-3">
          <summary className="cursor-pointer font-medium">
            Emergency stop, without this Mac
          </summary>
          <div className="mt-3 flex flex-col gap-2.5">
            <p className="m-0 text-[12.5px] text-fg-2">
              Run this on any machine that can reach {host.name}. It stops every
              run's container there and deletes nothing.
            </p>
            <div className="flex flex-wrap items-start gap-2.5">
              <code className="mono selectable min-w-0 flex-1 break-all rounded-md border bg-header px-3 py-2 text-[11.5px]">
                {host.emergency_stop}
              </code>
              <button
                type="button"
                className="btn btn-sm"
                onClick={() =>
                  void navigator.clipboard
                    .writeText(host.emergency_stop ?? "")
                    .then(() => {
                      setCopied(true);
                      setTimeout(() => setCopied(false), 2000);
                    })
                }
              >
                {copied ? "Copied" : "Copy"}
              </button>
            </div>
          </div>
        </details>
      )}

      <section
        aria-label={`Remove ${host.name}`}
        className="flex flex-wrap items-center gap-3 text-[12.5px]"
      >
        <Hint>
          Removing the host uninstalls the run controller and deletes what it
          holds there: its sealed secrets and its schedules. Possible once no
          run is live; work still waiting for a decision keeps its files.
          {host.state_kept && " Files from earlier runs remain on the host."}
        </Hint>
        <span className="flex-1" />
        {removing ? (
          <span className="flex gap-2">
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => setRemoving(false)}
            >
              Keep It
            </button>
            <button
              type="button"
              className="btn btn-sm text-conflict"
              disabled={busy}
              onClick={() => void remove()}
            >
              Remove {host.name}
            </button>
          </span>
        ) : (
          <button
            type="button"
            className="btn btn-sm btn-ghost text-conflict"
            disabled={busy || running || live.length > 0}
            onClick={() => setRemoving(true)}
          >
            Remove host…
          </button>
        )}
      </section>
    </div>
  );
}

function Banner({
  tone,
  title,
  children,
}: {
  tone: "blue" | "amber";
  title: string;
  children: ReactNode;
}) {
  return (
    <section
      aria-label={title}
      className="run-card text-[12.5px] leading-relaxed"
      data-tone={tone}
    >
      <h3 className="m-0 text-[13.5px] font-semibold">{title}</h3>
      {children}
    </section>
  );
}

/**
 * This Mac's page: its Docker engine, whether the run controller runs, and
 * this Mac's image and test (SPEC.md, Settings → Agents).
 */
export function LocalHostPage({
  host,
  settings,
  busy,
  onBack,
  onChooseEngine,
  onViewDockerfile,
  onSettings,
  onError,
  onAgent,
}: {
  host: AgentHost;
  settings: AgentSettings;
  busy: boolean;
  onBack: () => void;
  onChooseEngine: (socket: string) => void;
  onViewDockerfile: () => void;
  onSettings: (settings: AgentSettings) => void;
  onError: (message: string | null) => void;
  /** Opens an agent's page, where its test on this Mac is run. */
  onAgent?: (profileId: string) => void;
}) {
  const [engines, setEngines] = useState<AgentEngine[] | null>(null);
  const [controller, setController] = useState<RunControllerStatus | null>(
    null,
  );
  const [building, setBuilding] = useState(false);
  const loadEngines = useCallback(() => {
    setEngines(null);
    ipc
      .listAgentEngines()
      .then(setEngines)
      .catch((e) => onError(errorMessage(e)));
  }, [onError]);
  useEffect(loadEngines, [loadEngines]);

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

  // A build takes minutes: the rest of the page stays usable meanwhile.
  const build = async () => {
    setBuilding(true);
    onError(null);
    try {
      onSettings(await ipc.buildAgentImage());
    } catch (e) {
      onError(errorMessage(e));
      ipc.getAgentSettings().then(onSettings, () => {});
    } finally {
      setBuilding(false);
    }
  };
  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        host={host}
        onBack={onBack}
        subtitle="Runs pause while this Mac sleeps; when it wakes, a run past its time limit is stopped."
      />

      <Group label="Docker engine">
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
                chosen={settings.engine_socket === engine.socket}
                disabled={busy}
                onChoose={() => onChooseEngine(engine.socket)}
              />
            ))
          )}
        </div>
        <div className="flex items-start gap-2">
          <Hint>
            Brainiac starts and stops its own containers on this engine.
            Choosing another engine means building the image there.{" "}
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

      <HostMissing host={host} />

      <Group label="Setup">
        <ol className="settings-group m-0 list-none p-0">
          <ImageRow
            host={host}
            building={building}
            onViewDockerfile={onViewDockerfile}
          >
            <button
              type="button"
              className="btn btn-sm"
              disabled={building || busy || !settings.engine_socket}
              onClick={() => void build()}
            >
              {building ? "Building…" : host.image ? "Rebuild" : "Build Image"}
            </button>
          </ImageRow>
          <TestedAgents
            host={host}
            profiles={settings.profiles}
            onAgent={onAgent}
          />
        </ol>
      </Group>
    </div>
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
