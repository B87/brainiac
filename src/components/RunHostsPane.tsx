import { useEffect, useState } from "react";
import {
  type HostTone,
  hostNextStep,
  hostState,
  useHostJobsContext,
} from "../lib/hostJobs";
import {
  type AgentHost,
  type AgentSettings,
  errorMessage,
  ipc,
  type RoutineOverview,
} from "../lib/ipc";
import AddHostPage from "./AddHostPage";
import Dialog from "./Dialog";
import { HostPill } from "./HostJobView";
import { LocalHostPage, RemoteHostPage } from "./RunHostPage";
import { Hint, Lede } from "./SettingsPanes";

/** What needs the user sorts first, then what is busy, then the rest. */
const ORDER: Record<HostTone, number> = {
  failed: 0,
  attention: 1,
  busy: 2,
  idle: 3,
  ready: 4,
};

/**
 * Settings → Run hosts (docs/design/routines-ux.md): where runs and
 * scheduled routines execute, as a table that says what each host needs,
 * and each host's page. What runs there, with what access, is set on each
 * run or routine, never here.
 */
export default function RunHostsPane({
  hostId,
  onHost,
  onOpenRun,
  onSecrets,
}: {
  /** The host whose page is open, or none. */
  hostId: string | null;
  onHost: (hostId: string | null) => void;
  onOpenRun?: (runId: string) => void;
  /** Opens Settings › Secrets, where each copy on a host is listed. */
  onSecrets?: () => void;
}) {
  const [settings, setSettings] = useState<AgentSettings | null>(null);
  const [routines, setRoutines] = useState<RoutineOverview[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [dockerfile, setDockerfile] = useState<string | null>(null);
  /** Add host…, or confirming a saved host's key again. */
  const [adding, setAdding] = useState<{ existing?: AgentHost } | null>(null);
  const jobs = useHostJobsContext();

  const reload = () =>
    ipc.getAgentSettings().then(setSettings, (e) => setError(errorMessage(e)));
  useEffect(() => {
    let alive = true;
    ipc
      .getAgentSettings()
      .then((s) => alive && setSettings(s))
      .catch((e) => alive && setError(errorMessage(e)));
    ipc
      .listRoutines()
      .then((r) => alive && setRoutines(r))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  // A page opened from elsewhere starts without the last error.
  // biome-ignore lint/correctness/useExhaustiveDependencies: clear on navigation only
  useEffect(() => setError(null), [hostId]);

  const act = async (action: () => Promise<AgentSettings>) => {
    setBusy(true);
    setError(null);
    try {
      setSettings(await action());
    } catch (e) {
      setError(errorMessage(e));
      ipc.getAgentSettings().then(setSettings, () => {});
    } finally {
      setBusy(false);
    }
  };
  const showDockerfile = () =>
    ipc
      .agentDockerfile()
      .then(setDockerfile)
      .catch((e) => setError(errorMessage(e)));

  if (!settings) {
    return error ? (
      <div role="alert" className="text-[12.5px] text-conflict">
        {error}
      </div>
    ) : (
      <Hint>Loading…</Hint>
    );
  }

  const dialogs = dockerfile !== null && (
    <Dialog title="Dockerfile" width={720} onClose={() => setDockerfile(null)}>
      <pre className="mono selectable m-0 overflow-auto px-4 py-3 text-[11.5px] leading-relaxed">
        {dockerfile}
      </pre>
    </Dialog>
  );
  const errorLine = error && (
    <div role="alert" className="text-[12.5px] text-conflict">
      {error}
    </div>
  );

  // Add a host, or confirm a saved host's key again: a page of its own.
  if (adding) {
    return (
      <AddHostPage
        existing={adding.existing}
        onBack={() => setAdding(null)}
        onAdded={(id) => {
          setAdding(null);
          void reload();
          onHost(id);
        }}
      />
    );
  }

  const open = hostId ? settings.hosts.find((h) => h.id === hostId) : null;
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
          onSecrets={onSecrets}
        />
        {dialogs}
      </>
    );
  }

  const remote = settings.hosts.filter((h) => h.kind === "ssh");
  const scheduled = (host: AgentHost) =>
    routines.filter(
      (r) =>
        (host.kind === "local" ? "" : host.id) === r.routine.host_id &&
        r.schedule,
    ).length;
  const rows = settings.hosts
    .map((host) => {
      const job = jobs[host.id];
      return {
        host,
        state: hostState(host, job),
        next: hostNextStep(host, job),
      };
    })
    .sort(
      (a, b) =>
        ORDER[a.state.tone] - ORDER[b.state.tone] ||
        a.host.name.localeCompare(b.host.name),
    );

  return (
    <>
      <div className="flex flex-wrap items-end justify-between gap-3">
        <Lede>
          Where runs execute: this Mac's Docker engine, and Linux machines that
          stay on, so routines run while this Mac sleeps. Sorted with what needs
          you first.
        </Lede>
        <button
          type="button"
          className="btn btn-sm btn-primary"
          onClick={() => setAdding({})}
        >
          Add Host…
        </button>
      </div>
      {errorLine}
      <div className="overflow-x-auto rounded-[10px] border bg-app">
        <table aria-label="Run hosts" className="w-full border-collapse">
          <thead>
            <tr className="text-left text-[11px] uppercase tracking-wide text-muted">
              <th className="px-3.5 py-2 font-medium">Host</th>
              <th className="px-3 py-2 font-medium">State</th>
              <th className="px-3 py-2 font-medium">Run controller</th>
              <th className="px-3 py-2 font-medium">Routines</th>
              <th className="px-3 py-2 font-medium">Next step</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(({ host, state, next }) => (
              <tr
                key={host.id}
                className="border-line-soft border-t hover:bg-control"
              >
                <td className="px-3.5 py-2.5">
                  <button
                    type="button"
                    className="flex flex-col items-start text-left"
                    aria-label={`${host.name}, ${state.label}`}
                    onClick={() => onHost(host.id)}
                  >
                    <span className="text-[13px] font-semibold text-link">
                      {host.name}
                    </span>
                    <span className="mono text-[11.5px] text-muted">
                      {host.kind === "ssh"
                        ? `${host.ssh_user}@${host.ssh_host}`
                        : (host.engine_name ?? "Docker on this Mac")}
                    </span>
                  </button>
                </td>
                <td className="px-3 py-2.5">
                  <HostPill label={state.label} tone={state.tone} />
                </td>
                <td className="px-3 py-2.5 text-[12.5px] text-fg-2">
                  {host.kind === "local" ? (
                    "Built in"
                  ) : !host.installed ? (
                    "Not installed"
                  ) : host.upgrade_available ? (
                    <HostPill
                      label={`Older${host.controller_build ? `: ${host.controller_build}` : ""}`}
                      tone="attention"
                    />
                  ) : (
                    <HostPill
                      label={`Current${host.controller_build ? `: ${host.controller_build}` : ""}`}
                      tone="ready"
                    />
                  )}
                </td>
                <td className="px-3 py-2.5 text-[12.5px] text-fg-2">
                  {host.kind === "local"
                    ? "Run Now only"
                    : scheduled(host) || "None"}
                </td>
                <td className="px-3 py-2.5 text-[12.5px]">
                  {next ? (
                    <span className="text-dirty">{next}</span>
                  ) : (
                    <span className="text-muted">Nothing</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {remote.length === 0 && (
        <section
          aria-label="Run routines while your Mac sleeps"
          className="flex flex-col gap-2 rounded-[10px] border bg-app px-4 py-3.5"
        >
          <h3 className="m-0 text-[14px] font-semibold">
            Run routines while your Mac sleeps
          </h3>
          <p className="m-0 text-[12.5px] text-fg-2">
            A run host is a Linux machine that stays on. A routine on it fires
            at its time with this Mac closed, and Brainiac collects the briefing
            when it next opens. You will need:
          </p>
          <ul className="m-0 flex list-disc flex-col gap-1 pl-5 text-[12.5px] text-fg-2">
            <li>A Linux machine you can reach with ssh</li>
            <li>Docker running on it</li>
            <li>systemd 256 or newer, to keep secrets sealed</li>
          </ul>
          <Hint>
            No root login: Brainiac installs its run controller as a service
            with the sudo actions you approve, and keeps no private key.
          </Hint>
          <div>
            <button
              type="button"
              className="btn btn-sm btn-primary"
              onClick={() => setAdding({})}
            >
              Add Host…
            </button>
          </div>
        </section>
      )}
      <Hint>
        A remote host's administrator can see the repository and the credential.
        What runs there, with which secrets, is chosen on each run or routine.
      </Hint>
      {dialogs}
    </>
  );
}
