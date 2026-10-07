import { useState } from "react";
import {
  type HostTone,
  jobElapsed,
  jobTitle,
  stepOf,
  stepTime,
  timeOfDay,
  useNow,
} from "../lib/hostJobs";
import { errorMessage, type HostJob, type HostJobStep, ipc } from "../lib/ipc";
import Dialog from "./Dialog";

/** A host's tone as a pill's tint (`.state-pill`). */
export const TONE: Record<HostTone, string> = {
  ready: "green",
  busy: "blue",
  attention: "amber",
  failed: "red",
  idle: "dashed",
};

export function HostPill({ label, tone }: { label: string; tone: HostTone }) {
  return (
    <span className="state-pill" data-tone={TONE[tone]}>
      {tone === "busy" ? <Spinner /> : <span className="dot" />}
      {label}
    </span>
  );
}

export function Spinner() {
  return (
    <svg
      width="11"
      height="11"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="3"
      aria-hidden="true"
      className="animate-spin"
    >
      <path d="M21 12a9 9 0 1 1-6.2-8.6" />
    </svg>
  );
}

/** A step's state as an icon: a host job's, or a run's start. */
export function StepIcon({ state }: { state: HostJobStep["state"] }) {
  const common = {
    width: 16,
    height: 16,
    viewBox: "0 0 24 24",
    fill: "none",
    strokeWidth: 2.5,
  } as const;
  switch (state) {
    case "done":
      return (
        <svg {...common} stroke="var(--clean)" role="img" aria-label="Done">
          <path d="M5 12.5 10 17 19 7" />
        </svg>
      );
    case "running":
      return (
        <svg
          {...common}
          stroke="var(--link)"
          role="img"
          aria-label="In progress"
          className="animate-spin"
        >
          <path d="M21 12a9 9 0 1 1-6.2-8.6" />
        </svg>
      );
    case "failed":
      return (
        <svg
          {...common}
          stroke="var(--conflict)"
          role="img"
          aria-label="Failed"
        >
          <path d="M6 6l12 12M18 6 6 18" />
        </svg>
      );
    case "skipped":
      return (
        <svg
          {...common}
          stroke="var(--faint)"
          strokeWidth={2}
          role="img"
          aria-label="Not run"
        >
          <path d="M6 12h12" />
        </svg>
      );
    default:
      return (
        <svg
          {...common}
          stroke="var(--faint)"
          strokeWidth={2}
          role="img"
          aria-label="Not started"
        >
          <circle cx="12" cy="12" r="7" />
        </svg>
      );
  }
}

/**
 * A host job as numbered steps (SPEC.md, Remote hosts, Host jobs): each
 * step's state and time, the running step's progress and last output lines,
 * and the whole log on request.
 */
export default function HostJobView({ job }: { job: HostJob }) {
  const running = job.state === "running";
  const now = useNow(running);
  const [log, setLog] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const builds =
    job.to_build &&
    (job.kind === "upgrade" || job.kind === "install" || job.kind === "setup")
      ? job.from_build && job.from_build !== job.to_build
        ? `Build ${job.from_build} → ${job.to_build}`
        : `Build ${job.to_build}`
      : null;
  const failedAt = job.steps.findIndex((s) => s.state === "failed");

  const showLog = () =>
    ipc
      .getAgentHostJobLog(job.host_id)
      .then((text) => setLog(text || "Nothing was printed."))
      .catch((e) => setProblem(errorMessage(e)));
  const copyLog = () =>
    ipc
      .getAgentHostJobLog(job.host_id)
      .then((text) =>
        navigator.clipboard.writeText(
          [
            `${jobTitle(job)} on ${job.host_name}`,
            ...job.steps.map(
              (s, i) => `${i + 1}. ${s.title} — ${s.state}: ${s.detail}`,
            ),
            job.error ? `\n${job.error}` : "",
            job.error_details ?? "",
            text ? `\n${text}` : "",
          ]
            .filter((l) => l !== "")
            .join("\n"),
        ),
      )
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      })
      .catch((e) => setProblem(errorMessage(e)));

  return (
    <section
      aria-label={jobTitle(job)}
      className="overflow-hidden rounded-[10px] border bg-app"
      data-state={job.state}
    >
      <div
        className={`flex flex-wrap items-baseline gap-x-3.5 gap-y-1 border-b px-4 py-3 ${running ? "bg-info-bg" : ""}`}
      >
        <h3 className="m-0 text-[13.5px] font-semibold">{jobTitle(job)}</h3>
        {builds && <span className="text-[12.5px] text-fg-2">{builds}</span>}
        <span className="flex-1" />
        <span className="tabular text-[12.5px] text-fg-2">
          {running
            ? `Started ${timeOfDay(job.started_at)} · ${jobElapsed(job, now)}`
            : `${timeOfDay(job.started_at)} – ${timeOfDay(job.ended_at ?? job.started_at)}${
                job.state === "failed" || job.state === "interrupted"
                  ? ` · stopped at ${stepOf(job)}`
                  : job.state === "cancelled"
                    ? " · cancelled"
                    : ` · ${jobElapsed(job, now)}`
              }`}
        </span>
      </div>
      <ol className="m-0 list-none p-0 py-1.5">
        {job.steps.map((step, i) => (
          <li
            // Steps are a fixed list in a fixed order for one job.
            // biome-ignore lint/suspicious/noArrayIndexKey: see above
            key={i}
            className="flex gap-3 px-4 py-2"
          >
            <span className="w-[18px] shrink-0 pt-px">
              <StepIcon state={step.state} />
            </span>
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span
                className={
                  step.state === "waiting" || step.state === "skipped"
                    ? "text-[12.5px] text-muted"
                    : step.state === "failed"
                      ? "text-[12.5px] font-semibold text-conflict"
                      : "text-[12.5px] font-medium"
                }
              >
                {step.title}
              </span>
              <span className="text-[12px] text-muted">{step.detail}</span>
              {step.state === "running" && (
                <div className="mt-2 flex flex-col gap-2">
                  {step.progress && (
                    <span className="text-[12px] text-fg-2">
                      {step.progress}
                    </span>
                  )}
                  {job.log_tail.length > 0 && (
                    <OutputTail lines={job.log_tail} />
                  )}
                </div>
              )}
              {i === failedAt && (job.error || job.error_details) && (
                <pre
                  role="note"
                  aria-label="What happened"
                  className="mono selectable m-0 mt-2 max-h-[180px] overflow-auto whitespace-pre-wrap rounded-md border bg-header px-3 py-2 text-[11.5px] leading-relaxed"
                >
                  {[job.error, job.error_details].filter(Boolean).join("\n\n")}
                </pre>
              )}
            </span>
            <span className="tabular shrink-0 text-[12px] text-muted">
              {stepTime(step, now)}
            </span>
          </li>
        ))}
      </ol>
      {failedAt < 0 && job.error && job.state !== "cancelled" && (
        <p className="m-0 border-t px-4 py-2.5 text-[12.5px] text-conflict">
          {job.error}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-3.5 border-t px-4 py-2.5 text-[12.5px]">
        <button
          type="button"
          className="text-link hover:underline"
          onClick={() => void showLog()}
        >
          Show the whole log
        </button>
        <button
          type="button"
          className="text-link hover:underline"
          onClick={() => void copyLog()}
        >
          {copied ? "Copied" : "Copy the log"}
        </button>
        {problem && (
          <span role="alert" className="text-conflict">
            {problem}
          </span>
        )}
        <span className="flex-1" />
        {running && job.kind !== "build_image" && job.kind !== "test" && (
          <span className="text-muted">
            {job.cancellable
              ? "Cancel works until the install begins: nothing on the host has changed yet."
              : "The install has begun; it finishes on its own."}
          </span>
        )}
      </div>
      {log !== null && (
        <Dialog
          title={`${jobTitle(job)} on ${job.host_name}`}
          width={760}
          onClose={() => setLog(null)}
        >
          <pre className="mono selectable m-0 max-h-[60vh] overflow-auto whitespace-pre-wrap px-4 py-3 text-[11.5px] leading-relaxed">
            {log}
          </pre>
        </Dialog>
      )}
    </section>
  );
}

function OutputTail({ lines }: { lines: string[] }) {
  return (
    <pre
      role="log"
      aria-label="Output, last lines"
      className="mono selectable m-0 max-h-[150px] overflow-auto whitespace-pre-wrap rounded-md border bg-header px-3 py-2 text-[11.5px] leading-relaxed text-fg-2"
      ref={(el) => {
        // Follow the output as it grows.
        if (el) el.scrollTop = el.scrollHeight;
      }}
    >
      {lines.slice(-12).join("\n")}
    </pre>
  );
}

/** A thin bar for a host's row: how far through its steps a job is. */
export function JobBar({ job }: { job: HostJob }) {
  const done = job.steps.filter((s) => s.state === "done").length;
  const share = job.steps.length ? (done + 0.5) / job.steps.length : 0;
  return (
    <span
      className="block h-1 overflow-hidden rounded-sm"
      style={{ background: "var(--accent-soft)" }}
      role="progressbar"
      aria-label={`${jobTitle(job)}, ${stepOf(job)}`}
      aria-valuemin={0}
      aria-valuemax={job.steps.length}
      aria-valuenow={done}
    >
      <span
        className="block h-1 bg-link"
        style={{ width: `${Math.min(100, share * 100)}%` }}
      />
    </span>
  );
}
