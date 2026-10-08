import { createContext, useContext, useEffect, useRef, useState } from "react";
import { dayLabel } from "./agentRuns";
import {
  type AgentHost,
  type HostJob,
  type HostJobKind,
  type HostJobStep,
  ipc,
  onAgentHostJob,
} from "./ipc";

/**
 * Host jobs (SPEC.md, Remote hosts, Host jobs): a remote host's install,
 * upgrade, image build, or test, followed from Settings, New run, and the
 * status bar while it runs in the background.
 */

/** Each host's last job, by host ID. */
export type HostJobs = Record<string, HostJob>;

export const HostJobsContext = createContext<HostJobs>({});

export function useHostJobsContext(): HostJobs {
  return useContext(HostJobsContext);
}

/**
 * Loads each host's last job and follows changes. `onEnded` hears about a
 * job that ended while the window watched it, once.
 */
export function useHostJobs(onEnded: (job: HostJob) => void): HostJobs {
  const [jobs, setJobs] = useState<HostJobs>({});
  const ended = useRef(onEnded);
  ended.current = onEnded;
  useEffect(() => {
    let alive = true;
    const seen = new Map<string, HostJob>();
    const apply = (job: HostJob) => {
      const before = seen.get(job.host_id);
      // An event can overtake the list it raced with; an older job loses.
      if (before && before.id !== job.id && before.started_at > job.started_at)
        return;
      seen.set(job.host_id, job);
      if (
        before?.id === job.id &&
        before.state === "running" &&
        job.state !== "running"
      )
        ended.current(job);
      setJobs((all) => ({ ...all, [job.host_id]: job }));
    };
    const off = onAgentHostJob((job) => alive && apply(job));
    ipc
      .listAgentHostJobs()
      .then((list) => {
        if (!alive) return;
        for (const job of list) if (!seen.has(job.host_id)) apply(job);
      })
      .catch(() => {});
    return () => {
      alive = false;
      void off.then((unlisten) => unlisten());
    };
  }, []);
  return jobs;
}

/** The current time, ticking once a second while `active`. */
export function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [active]);
  return now;
}

const RUNNING: Record<HostJobKind, string> = {
  install: "Installing the run controller",
  upgrade: "Upgrading the run controller",
  build_image: "Building the image",
  test: "Testing",
  setup: "Setting up",
};

const ENDED: Record<HostJobKind, string> = {
  install: "Install of the run controller",
  upgrade: "Upgrade of the run controller",
  build_image: "Image build",
  test: "Test",
  setup: "Setup",
};

/** What the status bar and a host's row say: "upgrading", "testing". */
const VERB: Record<HostJobKind, string> = {
  install: "installing",
  upgrade: "upgrading",
  build_image: "building its image",
  test: "testing",
  setup: "setting up",
};

export function jobTitle(job: HostJob): string {
  return job.state === "running" ? RUNNING[job.kind] : ENDED[job.kind];
}

export function jobVerb(job: HostJob): string {
  return VERB[job.kind];
}

/** The step running now, or the one that failed. */
export function currentStep(job: HostJob): number {
  const running = job.steps.findIndex((s) => s.state === "running");
  if (running >= 0) return running;
  const failed = job.steps.findIndex((s) => s.state === "failed");
  if (failed >= 0) return failed;
  const done = job.steps.map((s) => s.state).lastIndexOf("done");
  return Math.max(0, done);
}

/** "step 2 of 6". */
export function stepOf(job: HostJob): string {
  return `step ${currentStep(job) + 1} of ${job.steps.length}`;
}

function millis(rfc3339: string | null | undefined): number | null {
  if (!rfc3339) return null;
  const t = new Date(rfc3339).getTime();
  return Number.isNaN(t) ? null : t;
}

/** "7:41", "1:02:05": a running clock. */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}

/** Seconds between two times; a running job counts up to `now`. */
function span(
  start: string | null | undefined,
  end: string | null | undefined,
  now: number,
): number | null {
  const from = millis(start);
  if (from === null) return null;
  const to = millis(end) ?? now;
  return Math.max(0, (to - from) / 1000);
}

export function jobElapsed(job: HostJob, now: number): string {
  return clock(span(job.started_at, job.ended_at, now) ?? 0);
}

/** "3 s" for a short step, a clock for a long one, nothing before it ran. */
export function stepTime(step: HostJobStep, now: number): string {
  if (step.state === "waiting" || step.state === "skipped") return "";
  const seconds = span(step.started_at, step.ended_at, now);
  if (seconds === null) return "";
  return seconds < 60 ? `${Math.round(seconds)} s` : clock(seconds);
}

/** "19:02". */
export function timeOfDay(rfc3339: string): string {
  const t = millis(rfc3339);
  if (t === null) return "";
  return new Date(t).toLocaleTimeString("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** "19 minutes", "40 seconds". */
export function durationWords(seconds: number): string {
  const s = Math.round(seconds);
  if (s < 60) return `${s} ${s === 1 ? "second" : "seconds"}`;
  const m = Math.round(s / 60);
  return `${m} ${m === 1 ? "minute" : "minutes"}`;
}

/**
 * What a job's end says in the window; the same words as the notification
 * (`host_jobs::outcome`). A cancelled job says nothing.
 */
export function jobOutcome(
  job: HostJob,
): { title: string; body: string; failed: boolean } | null {
  const name = job.host_name;
  const took = durationWords(span(job.started_at, job.ended_at, 0) ?? 0);
  if (job.state === "succeeded") {
    switch (job.kind) {
      case "upgrade":
        return {
          title: `${name} is upgraded`,
          body: `${job.to_build ? `Build ${job.to_build}, ` : ""}in ${took}. Runs can start there again.`,
          failed: false,
        };
      case "install":
        return {
          title: `The run controller is installed on ${name}`,
          body: `In ${took}. Build the image there next.`,
          failed: false,
        };
      case "build_image":
        return {
          title: `The image is built on ${name}`,
          body: `In ${took}. Test the host next.`,
          failed: false,
        };
      case "test":
        return {
          title: `${name} passed its test`,
          body: "Runs can start there.",
          failed: false,
        };
      case "setup": {
        const last = job.steps.at(-1);
        return last?.state === "skipped"
          ? {
              title: `${name} is installed, and its image built`,
              body: `Its test waits. ${last.detail}`,
              failed: false,
            }
          : {
              title: `${name} is ready`,
              body: `Set up in ${took}. Runs can start there.`,
              failed: false,
            };
      }
    }
  }
  if (job.state === "failed" || job.state === "interrupted") {
    const at = job.steps.findIndex((s) => s.state === "failed");
    const where = at >= 0 ? ` at step ${at + 1} of ${job.steps.length}` : "";
    const what: Record<HostJobKind, string> = {
      upgrade: `${name}'s upgrade failed`,
      install: `The install on ${name} failed`,
      build_image: `The image build on ${name} failed`,
      test: `${name}'s test did not pass`,
      setup: `Setting up ${name} failed`,
    };
    return {
      title: `${what[job.kind]}${where}`,
      body: job.error ?? "",
      failed: true,
    };
  }
  return null;
}

export type HostTone = "ready" | "busy" | "attention" | "failed" | "idle";

/** One state for a host's row and page header (SPEC.md, Run hosts). */
export function hostState(
  host: AgentHost,
  job: HostJob | undefined,
): { label: string; tone: HostTone } {
  if (job?.state === "running") {
    const verb = jobVerb(job);
    return {
      label: `${verb[0].toUpperCase()}${verb.slice(1)} · ${stepOf(job)}`,
      tone: "busy",
    };
  }
  if (job && (job.state === "failed" || job.state === "interrupted")) {
    const label =
      job.kind === "upgrade" && host.installed
        ? "Upgrade failed · still on its previous controller"
        : `${ENDED[job.kind]} ${job.state === "interrupted" ? "interrupted" : "failed"}`;
    return { label, tone: "failed" };
  }
  if (host.kind === "ssh" && !host.approved)
    return { label: "Waiting for confirmation", tone: "attention" };
  if (host.kind === "ssh" && !host.installed)
    return { label: "Not set up", tone: "idle" };
  if (!host.image) return { label: "No image yet", tone: "attention" };
  if (!host.image.current)
    return { label: "Image to rebuild", tone: "attention" };
  if (host.missing.length > 0) return { label: "Not ready", tone: "attention" };
  // Ready once any profile's test there still matches (SPEC.md, Settings → Agents, Test).
  if (!testedProfiles(host)) return { label: "Test needed", tone: "attention" };
  if (host.upgrade_available)
    return { label: "Ready · upgrade available", tone: "ready" };
  return { label: "Ready", tone: "ready" };
}

/** How many profiles passed a test on the host that still matches. */
export function testedProfiles(host: AgentHost): number {
  return host.tests.filter((t) => t.current).length;
}

/** The address and what a host has, for its row. */
export function hostSummary(host: AgentHost): string {
  const parts: string[] = [];
  if (host.kind === "ssh")
    parts.push(`${host.ssh_user}@${host.ssh_host}:${host.ssh_port ?? 22}`);
  if (host.engine_name) parts.push(host.engine_name);
  if (host.image) parts.push(`image built ${dayLabel(host.image.built_at)}`);
  const tested = testedProfiles(host);
  if (tested)
    parts.push(`${tested} ${tested === 1 ? "agent" : "agents"} tested`);
  if (host.kind === "local") parts.push("pauses while this Mac sleeps");
  else if (host.installed) parts.push("keeps working while this Mac sleeps");
  return parts.join(" · ");
}

/** A short fingerprint for a header: start and end. */
export function shortFingerprint(fingerprint: string): string {
  const [kind, value] = fingerprint.includes(":")
    ? [
        fingerprint.slice(0, fingerprint.indexOf(":") + 1),
        fingerprint.slice(fingerprint.indexOf(":") + 1),
      ]
    : ["", fingerprint];
  return value.length > 20
    ? `${kind}${value.slice(0, 8)}…${value.slice(-8)}`
    : fingerprint;
}
