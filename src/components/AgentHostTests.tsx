import { useEffect, useRef, useState } from "react";
import { dayLabel, profileName, profileTestState } from "../lib/agentRuns";
import { currentStep, useHostJobsContext } from "../lib/hostJobs";
import {
  type AgentHost,
  type AgentProfile,
  type AgentTestResult,
  errorMessage,
  ipc,
} from "../lib/ipc";
import { HostPill, StepIcon } from "./HostJobView";
import { Group, Hint } from "./SettingsPanes";

/**
 * An agent's page, Where it can run (SPEC.md, Settings → Agents, Test): each
 * host with this agent's test there and Test. A test checks the pair: this
 * agent's key with that host's image and engine. On a run host it is a host
 * job, followed here and on the host's page; on this Mac it runs while this
 * page waits, and its steps show under the row.
 */
export default function AgentHostTests({
  profile,
  hosts,
  onHost,
  onChanged,
}: {
  profile: AgentProfile;
  hosts: AgentHost[];
  onHost: (hostId: string) => void;
  /** Reads the settings again: a test ended. */
  onChanged: () => void;
}) {
  const jobs = useHostJobsContext();
  const name = profileName(profile);
  const [error, setError] = useState<string | null>(null);
  /** This Mac's test while it runs, and its last result. */
  const [localTesting, setLocalTesting] = useState(false);
  const [localResult, setLocalResult] = useState<AgentTestResult | null>(null);

  // A host's test of this agent that ends changes what the hosts report.
  const before = useRef<Record<string, string>>({});
  useEffect(() => {
    let ended = false;
    for (const host of hosts) {
      const job = jobs[host.id];
      if (job?.kind !== "test") continue;
      const key = `${job.id}:${job.state}`;
      const was = before.current[host.id];
      if (was?.startsWith(`${job.id}:running`) && job.state !== "running")
        ended = true;
      before.current[host.id] = key;
    }
    if (ended) onChanged();
  }, [jobs, hosts, onChanged]);

  const testRemote = async (host: AgentHost) => {
    setError(null);
    try {
      await ipc.startAgentHostJob(host.id, "test", profile.id);
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  // A wrong key can take minutes to be refused: the page stays usable.
  const testLocal = async () => {
    setError(null);
    setLocalResult(null);
    setLocalTesting(true);
    try {
      setLocalResult(await ipc.testAgentSetup(profile.id));
      onChanged();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLocalTesting(false);
    }
  };

  return (
    <Group label="Where it can run">
      <ul className="settings-group m-0 list-none p-0">
        {hosts.map((host) => {
          const local = host.kind === "local";
          const job = jobs[host.id];
          const busy = !local && job?.state === "running";
          const mine =
            job?.kind === "test" && job.profile_ids[0] === profile.id;
          const testing = local ? localTesting : busy && mine;
          const failed =
            !local &&
            mine &&
            (job.state === "failed" || job.state === "interrupted");
          const state = profileTestState(profile, host);
          // What stops a test here, in the order to fix it.
          const blocked =
            profile.missing.length > 0
              ? "Not ready: see above"
              : !local && !host.approved
                ? `Confirm ${host.name}'s key first`
                : !local && !host.installed
                  ? `Set up ${host.name} first`
                  : !host.image?.current
                    ? host.image
                      ? `Rebuild the image on ${host.name} first`
                      : `Build the image on ${host.name} first`
                    : (host.missing[0] ?? null);
          const line = testing
            ? local
              ? "Testing: a short run that sends a prompt, cancels, and collects…"
              : `Testing · ${job?.steps[currentStep(job)]?.title ?? ""}`
            : busy
              ? `${host.name} is busy with another job`
              : state.kind === "passed"
                ? `Test passed ${dayLabel(state.at)}, with this key, image, and engine`
                : blocked
                  ? blocked
                  : state.kind === "stale"
                    ? `Passed ${dayLabel(state.at)}, before the key, the image, or the engine changed: test again`
                    : "Not tested here: runs of it cannot start here until it passes";
          const tone =
            state.kind === "passed" && !testing
              ? "text-muted"
              : blocked || state.kind !== "passed"
                ? "text-dirty"
                : "text-muted";
          const shown = local ? localResult : null;
          return (
            <li
              key={host.id}
              aria-label={host.name}
              className="settings-row flex-col items-stretch gap-2"
            >
              <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                <span className="flex min-w-55 flex-1 flex-col gap-0.5">
                  <span className="font-medium">{host.name}</span>
                  <span
                    className={`text-[12px] ${testing ? "text-fg-2" : tone}`}
                  >
                    {line}
                  </span>
                </span>
                {state.kind === "passed" && !testing && (
                  <HostPill label="Tested" tone="ready" />
                )}
                <button
                  type="button"
                  className="btn btn-sm"
                  aria-label={`Test ${name} on ${host.name}`}
                  disabled={!!blocked || busy || localTesting}
                  onClick={() => void (local ? testLocal() : testRemote(host))}
                >
                  {testing
                    ? "Testing…"
                    : state.kind === "passed" || state.kind === "stale"
                      ? "Test Again"
                      : "Test"}
                </button>
                <button
                  type="button"
                  className="btn btn-sm btn-ghost text-link"
                  onClick={() => onHost(host.id)}
                >
                  Open Host
                </button>
              </div>
              {failed && (
                <span className="text-[12px] text-conflict">
                  The last test did not pass
                  {job.error ? `: ${job.error}` : "."} Its log is on the host's
                  page.
                </span>
              )}
              {shown && (
                <div className="flex flex-col gap-1 pl-1">
                  {shown.steps.map((step) => (
                    <div key={step.name} className="flex gap-2 text-[12.5px]">
                      <StepIcon state={step.passed ? "done" : "failed"} />
                      <span>{step.name}</span>
                      {step.detail && (
                        <span className="truncate text-muted">
                          {step.detail}
                        </span>
                      )}
                    </div>
                  ))}
                  <Hint>
                    {shown.passed
                      ? `Passed at ${dayLabel(shown.tested_at)}.`
                      : `The test did not pass; runs of ${name} cannot start on ${host.name} yet.`}
                  </Hint>
                </div>
              )}
            </li>
          );
        })}
      </ul>
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}
      <Hint>
        A test starts a short run with this agent, sends a prompt, cancels, and
        collects. On a run host it is the first time the key goes there. A wrong
        key can take a few minutes to be refused: the agent retries it first.
        What a host lacks is fixed on its page; approving a routine there does
        most of it for you.
      </Hint>
    </Group>
  );
}
