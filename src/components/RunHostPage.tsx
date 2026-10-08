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
  hostState,
  shortFingerprint,
  useHostJobsContext,
} from "../lib/hostJobs";
import {
  type AgentEngine,
  type AgentHost,
  type AgentProfile,
  type AgentRun,
  type AgentSettings,
  type AgentTestResult,
  errorMessage,
  type HostJobKind,
  ipc,
  type RunControllerStatus,
} from "../lib/ipc";
import HostJobView, { HostPill } from "./HostJobView";
import { Group, Hint } from "./SettingsPanes";

/**
 * Agents › Run hosts › a host, or Agents › a profile: where the breadcrumb
 * goes back to.
 */
export function Breadcrumb({
  name,
  via,
  onBack,
}: {
  name: string;
  /** The list between Agents and the page, such as "Run hosts". */
  via?: string;
  onBack: () => void;
}) {
  return (
    <nav aria-label="Breadcrumb" className="text-[12px] text-muted">
      <button
        type="button"
        className="text-link hover:underline"
        onClick={onBack}
      >
        Agents
      </button>{" "}
      ›{" "}
      {via && (
        <>
          <button
            type="button"
            className="text-link hover:underline"
            onClick={onBack}
          >
            {via}
          </button>{" "}
          ›{" "}
        </>
      )}
      {name}
    </nav>
  );
}

function PageHeader({
  host,
  subtitle,
  actions,
}: {
  host: AgentHost;
  subtitle: ReactNode;
  actions?: ReactNode;
}) {
  const jobs = useHostJobsContext();
  const state = hostState(host, jobs[host.id]);
  return (
    <header className="flex flex-wrap items-start justify-between gap-4">
      <div className="flex min-w-0 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2.5">
          <h2 className="m-0 text-[19px] font-semibold">{host.name}</h2>
          <HostPill label={state.label} tone={state.tone} />
        </div>
        <span className="text-[12.5px] text-muted">{subtitle}</span>
      </div>
      {actions && <div className="flex flex-wrap gap-2">{actions}</div>}
    </header>
  );
}

function Check({ done }: { done: boolean }) {
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
  title,
  detail,
  children,
}: {
  done: boolean;
  title: string;
  detail: ReactNode;
  children?: ReactNode;
}) {
  return (
    <li className="settings-row">
      <Check done={done} />
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
  return (
    <SetupRow
      done={!!host.image?.current}
      title={host.image ? "Image built" : "Image not built"}
      detail={
        host.image ? (
          <>
            <span className="mono">{imageSummary(host.image)}</span> ·{" "}
            {dayLabel(host.image.built_at)}
            {!host.image.current &&
              " · this Brainiac changed the Dockerfile: rebuild it"}
            {host.kind === "ssh" &&
              host.engine_name &&
              ` · ${host.engine_name}`}
            {building &&
              " · building downloads the base image and packages; it takes a few minutes"}
          </>
        ) : building ? (
          "Building downloads the base image and packages; it takes a few minutes."
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
 * One test line per profile on a host: passed, to run again, not run, or
 * what the profile lacks first (SPEC.md, Settings → Agents, Test).
 */
function ProfileTests({
  host,
  profiles,
  disabled,
  testing,
  onTest,
  result,
}: {
  host: AgentHost;
  profiles: AgentProfile[];
  /** Nothing can be tested now: the host is busy or not ready. */
  disabled: boolean;
  /** The profile being tested now, if any. */
  testing?: string | null;
  onTest: (profileId: string) => void;
  /** This Mac's last test, under its profile's line. */
  result?: { profileId: string; result: AgentTestResult } | null;
}) {
  return (
    <>
      {profiles.map((profile) => {
        const name = profileName(profile);
        const state = profileTestState(profile, host);
        const title =
          state.kind === "passed"
            ? `${name} · test passed ${dayLabel(state.at)}`
            : state.kind === "stale"
              ? `${name} · test to run again`
              : `${name} · not tested`;
        const detail =
          state.kind === "passed"
            ? "With this token or key, image, and engine."
            : state.kind === "stale"
              ? `Passed ${dayLabel(state.at)}, before the token or key, the image, or the engine changed.`
              : state.kind === "blocked"
                ? state.reason
                : "Runs of it start here once it passes.";
        const shown = result?.profileId === profile.id ? result.result : null;
        return (
          <li
            key={profile.id}
            className="settings-row flex-col items-stretch gap-2"
          >
            <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
              <Check done={state.kind === "passed"} />
              <span className="flex min-w-55 flex-1 flex-col gap-0.5">
                <span className="font-medium">{title}</span>
                <Hint>{detail}</Hint>
              </span>
              <button
                type="button"
                className="btn btn-sm"
                aria-label={`Test ${name}`}
                disabled={disabled || state.kind === "blocked" || !!testing}
                onClick={() => onTest(profile.id)}
              >
                {testing === profile.id
                  ? "Testing…"
                  : state.kind === "passed" || state.kind === "stale"
                    ? "Test Again"
                    : "Test"}
              </button>
            </div>
            {shown && (
              <div className="flex flex-col gap-1 pl-8">
                {shown.steps.map((step) => (
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
                  {shown.passed
                    ? `Passed at ${dayLabel(shown.tested_at)}.`
                    : `The test did not pass; runs of ${name} cannot start here yet.`}
                </Hint>
              </div>
            )}
          </li>
        );
      })}
    </>
  );
}

const CANCEL_LABEL: Partial<Record<HostJobKind, string>> = {
  install: "Cancel install",
  upgrade: "Cancel upgrade",
  setup: "Cancel setup",
};

/**
 * A remote host's page (SPEC.md, Remote hosts): its state, an upgrade when
 * one is available, its last job, its setup in order, its live runs,
 * Emergency stop, and Remove.
 */
export function RemoteHostPage({
  host,
  profiles,
  onBack,
  onChanged,
  onConfirmKey,
  onViewDockerfile,
  onOpenRun,
}: {
  host: AgentHost;
  profiles: AgentProfile[];
  onBack: () => void;
  onChanged: () => void;
  /** Opens Add host for this host, to approve its key again. */
  onConfirmKey: (host: AgentHost) => void;
  onViewDockerfile: () => void;
  onOpenRun?: (runId: string) => void;
}) {
  const jobs = useHostJobsContext();
  const job = jobs[host.id];
  const running = job?.state === "running";
  const live = useLiveRuns(host.id);
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

  return (
    <div className="flex flex-col gap-5">
      <Breadcrumb name={host.name} via="Run hosts" onBack={onBack} />
      <PageHeader
        host={host}
        subtitle={
          <>
            {address}
            {host.fingerprint && ` · key ${shortFingerprint(host.fingerprint)}`}
            {host.installed &&
              !running &&
              " · keeps working while this Mac sleeps"}
          </>
        }
        actions={
          <>
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
                  void start(
                    job.kind,
                    job.kind === "test" ? job.profile_ids[0] : undefined,
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

      {host.upgrade_available && host.approved && !running && (
        <Banner
          tone="blue"
          title="An upgrade of the run controller is available"
        >
          <p className="m-0">
            {host.name} runs{" "}
            {host.controller_build
              ? `build ${host.controller_build}`
              : "an earlier build"}
            ; this Brainiac builds {host.available_build}. Upgrading restarts
            the controller, so it waits while a run is live
            {live.length > 0
              ? `: ${live.length === 1 ? `“${live[0].title}” is live` : `${live.length} runs are live`} there now.`
              : "."}
          </p>
          <div className="flex flex-wrap items-center gap-2.5">
            <button
              type="button"
              className="btn btn-sm btn-primary"
              disabled={busy}
              onClick={() => void start("upgrade")}
            >
              {live.length > 0 ? "Upgrade When Runs End" : "Upgrade"}
            </button>
            <span className="text-[12px]">
              Builds the new controller now, so the restart itself takes
              seconds. New runs on {host.name} wait from then until it is done.
            </span>
          </div>
        </Banner>
      )}

      {job && <HostJobView job={job} />}

      {host.approved && host.installed && <HostMissing host={host} />}

      {running &&
        job &&
        (job.kind === "upgrade" ||
          job.kind === "install" ||
          job.kind === "setup") && (
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
                The first build after a Brainiac update compiles the Linux run
                controller on this Mac, in Docker, imitating the host's
                processor when it differs. Brainiac keeps that build's cache, so
                later builds recompile only what changed.
              </li>
            </ul>
          </Group>
        )}

      <Group label="Setup">
        <ol className="settings-group m-0 list-none p-0">
          <SetupRow
            done={host.approved}
            title={host.approved ? "Host key confirmed" : "Host key to confirm"}
            detail={
              showKey || !host.fingerprint
                ? (host.fingerprint ?? "No key saved")
                : shortFingerprint(host.fingerprint)
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
            done={host.installed}
            title={
              host.installed
                ? "Run controller installed"
                : "Run controller not installed"
            }
            detail={
              host.installed
                ? [
                    host.controller_build
                      ? `Build ${host.controller_build}`
                      : "An earlier build",
                    host.protocol != null && `protocol ${host.protocol}`,
                    "systemd service brainiac-runner",
                    host.controller_installed_at &&
                      dayLabel(host.controller_installed_at),
                  ]
                    .filter(Boolean)
                    .join(" · ")
                : "Install puts it on the host as a systemd service, with the sudo actions you approved."
            }
          >
            <button
              type="button"
              className="btn btn-sm btn-ghost text-link"
              disabled={blocked}
              onClick={() => void start("install")}
            >
              {host.installed ? "Reinstall" : "Install"}
            </button>
          </SetupRow>
          <ImageRow host={host} onViewDockerfile={onViewDockerfile}>
            <button
              type="button"
              className="btn btn-sm btn-ghost text-link"
              disabled={blocked || !host.installed}
              onClick={() => void start("build_image")}
            >
              {host.image ? "Rebuild" : "Build Image"}
            </button>
          </ImageRow>
          <ProfileTests
            host={host}
            profiles={profiles}
            disabled={
              blocked || !host.image?.current || host.missing.length > 0
            }
            testing={
              running && job?.kind === "test" ? job.profile_ids[0] : null
            }
            onTest={(profileId) => void start("test", profileId)}
          />
        </ol>
        <Hint>
          A test starts a short run, sends a prompt, cancels, and collects. It
          is the first time that token or key goes to {host.name}.
        </Hint>
      </Group>

      {live.length > 0 && (
        <Group label={`Runs on ${host.name}`}>
          <ul className="settings-group m-0 list-none p-0">
            {live.map((run) => (
              <li key={run.id} className="settings-row">
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
              </li>
            ))}
          </ul>
        </Group>
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
        className="flex flex-wrap items-center gap-3 border-t pt-3"
      >
        <span className="flex min-w-55 flex-1 flex-col gap-0.5">
          <span className="font-medium">Remove {host.name}</span>
          <Hint>
            Uninstalls the run controller. Possible once no run is live there;
            work still waiting for a decision keeps its files on the host.
            {host.state_kept && " Files from earlier runs remain on the host."}
          </Hint>
        </span>
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
            className="btn btn-sm text-conflict"
            disabled={busy || running || live.length > 0}
            onClick={() => setRemoving(true)}
          >
            Remove…
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
}: {
  host: AgentHost;
  settings: AgentSettings;
  busy: boolean;
  onBack: () => void;
  onChooseEngine: (socket: string) => void;
  onViewDockerfile: () => void;
  onSettings: (settings: AgentSettings) => void;
  onError: (message: string | null) => void;
}) {
  const [engines, setEngines] = useState<AgentEngine[] | null>(null);
  const [controller, setController] = useState<RunControllerStatus | null>(
    null,
  );
  const [building, setBuilding] = useState(false);
  /** The profile being tested, and the last test's result under its line. */
  const [testing, setTesting] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<{
    profileId: string;
    result: AgentTestResult;
  } | null>(null);

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
  /** Test: minutes when a wrong token is retried; the page stays usable. */
  const runTest = async (profileId: string) => {
    setTesting(profileId);
    onError(null);
    setTestResult(null);
    try {
      const result = await ipc.testAgentSetup(profileId);
      setTestResult({ profileId, result });
      onSettings(await ipc.getAgentSettings());
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setTesting(null);
    }
  };

  return (
    <div className="flex flex-col gap-5">
      <Breadcrumb name="This Mac" via="Run hosts" onBack={onBack} />
      <PageHeader
        host={host}
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
          <ProfileTests
            host={host}
            profiles={settings.profiles}
            disabled={busy || building || host.missing.length > 0}
            testing={testing}
            onTest={(profileId) => void runTest(profileId)}
            result={testResult}
          />
        </ol>
        <Hint>
          A test starts a short run, sends a prompt, cancels, and collects. A
          run of an agent cannot start until its test passes for this token or
          key, image, and engine. A wrong token or key can take a few minutes to
          be refused: the agent retries it first.
        </Hint>
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
