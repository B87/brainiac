import { useEffect, useState } from "react";
import { durationLabel, profileName, providerName } from "../lib/agentRuns";
import { relativeTime } from "../lib/format";
import { currentStep, jobTitle, useHostJobsContext } from "../lib/hostJobs";
import {
  type AgentHost,
  type AgentProfile,
  type AgentSettings,
  errorMessage,
  type HostClock,
  type HostJob,
  type HostJobStep,
  ipc,
  type NamedSecret,
  type RoutineOverview,
  type RoutineSchedule,
  type ScheduleWhen,
} from "../lib/ipc";
import {
  hostClockWords,
  hostTime,
  scheduleWords,
  usd,
  utcOffsetWords,
} from "../lib/routines";
import { toSend, useHostCopies } from "./AccessPicker";
import Dialog from "./Dialog";
import { StepIcon } from "./HostJobView";

const WEEKDAYS = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
];

const NEW_SCHEDULE: ScheduleWhen = {
  days: "weekdays",
  weekday: 0,
  time: "07:00",
  late_start: false,
  cost_ceiling_micros: 1_000_000,
};

/**
 * A run host's clock, asked when the tab opens: a schedule's time is in the
 * host's own time zone, which need not be this Mac's. Drawn again each
 * half minute so the time shown moves.
 */
function useHostClock(hostId: string): HostClock | null {
  const [clock, setClock] = useState<HostClock | null>(null);
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!hostId) return;
    let alive = true;
    ipc
      .hostClock(hostId)
      .then((c) => alive && setClock(c))
      .catch(() => {});
    const timer = window.setInterval(() => setTick((t) => t + 1), 30_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [hostId]);
  return clock;
}

/** Whether a host's clock is in another zone than this Mac's. */
function otherZone(clock: HostClock | null): boolean {
  return (
    clock !== null &&
    clock.utc_offset_minutes !== -new Date().getTimezoneOffset()
  );
}

/**
 * A routine's Schedule tab (docs/design/routines-ux.md, Schedules): when the
 * host fires it, its limits, and where it stands. A schedule is off until the
 * user approves it, and again whenever something it depends on changes.
 */
export default function ScheduleTab({
  item,
  onChanged,
  onError,
}: {
  item: RoutineOverview;
  onChanged: () => void;
  onError: (message: string | null) => void;
}) {
  const { routine, schedule } = item;
  const [draft, setDraft] = useState<ScheduleWhen>(
    schedule?.when ?? NEW_SCHEDULE,
  );
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [approving, setApproving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The host's job while it is prepared for this schedule.
  const job = useHostJobsContext()[routine.host_id];
  const remote = routine.host_id !== "";
  const clock = useHostClock(routine.host_id);
  // What the host runs must not depend on what is checked out here.
  const moving = routine.start.trim().toUpperCase() === "HEAD";

  const act = async (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    setError(null);
    onError(null);
    try {
      await action();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
      onChanged();
    }
  };
  const dirty =
    schedule !== null &&
    JSON.stringify(draft) !== JSON.stringify(schedule.when);

  if (!remote) {
    return (
      <p className="m-0 px-5 py-4 text-muted">
        A schedule runs on a run host, because this Mac sleeps. Choose a host
        for this routine in Definition first.
      </p>
    );
  }
  if (moving) {
    return (
      <p className="m-0 px-5 py-4 text-muted">
        A schedule starts from a branch or a commit, not HEAD, so what the host
        runs does not depend on what is checked out on this Mac. Change Start
        from in Definition first.
      </p>
    );
  }
  if (!schedule && !editing) {
    return (
      <div className="flex flex-col items-start gap-3 px-5 py-4">
        <span className="text-muted">
          Not scheduled. Run Now starts it when you ask.
        </span>
        <button
          type="button"
          className="btn"
          onClick={() => {
            setDraft(NEW_SCHEDULE);
            setEditing(true);
          }}
        >
          Add a schedule…
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 px-5 py-4">
      <Section title="When">
        <Row label="Days and time">
          <div className="flex flex-wrap items-center gap-2">
            <select
              className="field w-auto"
              aria-label="Days"
              value={draft.days}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  days: e.target.value as ScheduleWhen["days"],
                })
              }
            >
              <option value="daily">Daily</option>
              <option value="weekdays">Weekdays</option>
              <option value="weekly">Weekly</option>
            </select>
            {draft.days === "weekly" && (
              <select
                className="field w-auto"
                aria-label="Day of the week"
                value={draft.weekday}
                onChange={(e) =>
                  setDraft({ ...draft, weekday: Number(e.target.value) })
                }
              >
                {WEEKDAYS.map((d, i) => (
                  <option key={d} value={i}>
                    {d}
                  </option>
                ))}
              </select>
            )}
            <input
              className="field mono w-24"
              aria-label="Time"
              value={draft.time}
              onChange={(e) => setDraft({ ...draft, time: e.target.value })}
            />
          </div>
          <Hint>
            <span className={otherZone(clock) ? "text-dirty" : undefined}>
              {clock
                ? `In ${item.host_name}'s time. ${hostClockWords(item.host_name, clock.utc_offset_minutes)}`
                : `In ${item.host_name}'s time zone, which may not be this Mac's.`}
            </span>{" "}
            The host fires it; this Mac can be asleep.
          </Hint>
        </Row>
        <Row label="If the host was down at the time">
          <select
            className="field w-auto"
            aria-label="Late start"
            value={draft.late_start ? "late" : "skip"}
            onChange={(e) =>
              setDraft({ ...draft, late_start: e.target.value === "late" })
            }
          >
            <option value="skip">Do not start late</option>
            <option value="late">Start once when it is back</option>
          </select>
          <Hint>A missed run is recorded as missed either way.</Hint>
        </Row>
      </Section>

      <Section title="Limits">
        <Row label="Time limit">
          <span>{durationLabel(routine.time_limit_minutes)}</span>
          <Hint>Set in Definition. The run is stopped when it is reached.</Hint>
        </Row>
        <Row label="Cost ceiling for one run">
          <input
            className="field w-28"
            aria-label="Cost ceiling in dollars"
            inputMode="decimal"
            value={(draft.cost_ceiling_micros / 1_000_000).toString()}
            onChange={(e) => {
              const dollars = Number(e.target.value);
              if (Number.isFinite(dollars))
                setDraft({
                  ...draft,
                  cost_ceiling_micros: Math.round(dollars * 1_000_000),
                });
            }}
          />
          <Hint>
            When a run reports a cost over this, the schedule is paused.
            Brainiac reads the cost after the run; it cannot stop one in the
            middle.
          </Hint>
        </Row>
        {schedule && (
          <Row label="Cost">
            <span>
              This month {usd(schedule.month_cost_micros)}
              {item.latest?.cost_micros != null &&
                ` · last run ${usd(item.latest.cost_micros)}`}
            </span>
          </Row>
        )}
      </Section>

      {schedule && (
        <Section title="State">
          <Row label="Approval">
            {schedule.state === "preparing" ? (
              <span>
                Approved. Preparing {item.host_name}
                {job?.state === "running"
                  ? `: ${job.steps[currentStep(job)]?.title ?? ""}`
                  : ""}
                . The schedule goes there when that is done.
              </span>
            ) : schedule.state === "needs_approval" ? (
              <span className="text-dirty">
                Needs approval: {schedule.changed.join(", ")}.
              </span>
            ) : (
              <span>
                Approved. Changing the prompt, host, secrets, tools or limits
                asks again.
              </span>
            )}
          </Row>
          <Row label="Next run">
            <span>
              {schedule.state === "paused"
                ? "Paused"
                : schedule.state === "preparing"
                  ? `When ${item.host_name} is prepared`
                  : schedule.state === "needs_approval"
                    ? "Off until approved"
                    : (hostTime(schedule.next_fire) ?? "Not known yet")}
            </span>
            <Hint>
              {schedule.synced_at
                ? `The host last answered ${relativeTime(schedule.synced_at)}.`
                : "The host has not answered yet."}
            </Hint>
          </Row>
          <Row label="Code on the host">
            <span>
              {schedule.code_commit ? (
                <>
                  <code className="mono">
                    {schedule.code_commit.slice(0, 7)}
                  </code>
                  {schedule.code_at &&
                    `, sent ${relativeTime(schedule.code_at)}`}
                </>
              ) : (
                "Not sent yet"
              )}
            </span>
            <Hint>
              The host starts runs from the code this Mac last sent, and
              Brainiac sends a newer commit when it reaches the host.
            </Hint>
          </Row>
          {schedule.notice && (
            <Row label="Note">
              <span className="text-dirty">{schedule.notice}</span>
            </Row>
          )}
          {schedule.error && (
            <Row label="Host">
              <span className="text-conflict">{schedule.error}</span>
            </Row>
          )}
        </Section>
      )}

      {error && (
        <span className="text-conflict" role="alert">
          {error}
        </span>
      )}

      <div className="flex flex-wrap items-center gap-2">
        {schedule?.state === "needs_approval" || !schedule ? (
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy !== null}
            onClick={() =>
              void act("save", async () => {
                await ipc.setRoutineSchedule({
                  routine_id: routine.id,
                  when: draft,
                });
                setEditing(false);
                setApproving(true);
              })
            }
          >
            {schedule ? "Review and approve…" : "Save schedule…"}
          </button>
        ) : (
          <button
            type="button"
            className="btn btn-primary"
            disabled={!dirty || busy !== null}
            onClick={() =>
              void act("save", async () => {
                await ipc.setRoutineSchedule({
                  routine_id: routine.id,
                  when: draft,
                });
                setApproving(true);
              })
            }
          >
            Save schedule…
          </button>
        )}
        {schedule &&
          schedule.state !== "needs_approval" &&
          schedule.state !== "preparing" && (
            <button
              type="button"
              className="btn"
              disabled={busy !== null}
              onClick={() =>
                void act("pause", () =>
                  ipc.pauseRoutine(routine.id, schedule.state !== "paused"),
                )
              }
            >
              {schedule.state === "paused" ? "Resume" : "Pause"}
            </button>
          )}
        {schedule && (
          <button
            type="button"
            className="btn text-conflict"
            disabled={busy !== null}
            onClick={() =>
              void act("remove", () => ipc.removeRoutineSchedule(routine.id))
            }
          >
            Remove Schedule
          </button>
        )}
      </div>

      {approving && (
        <ApproveDialog
          item={item}
          draft={draft}
          onClose={() => setApproving(false)}
          onApproved={() => {
            setApproving(false);
            onChanged();
          }}
        />
      )}
    </div>
  );
}

/**
 * Approve “routine” on host (docs/design/routines-ux.md): what unattended
 * runs may do, as facts, once, before the schedule is turned on. Approving
 * sends the secrets the host lacks, and when the host needs an upgrade or an
 * image, prepares it and follows that here until the schedule is on.
 */
function ApproveDialog({
  item,
  draft,
  onClose,
  onApproved,
}: {
  item: RoutineOverview;
  draft: ScheduleWhen;
  onClose: () => void;
  onApproved: () => void;
}) {
  const { routine } = item;
  const [settings, setSettings] = useState<AgentSettings | null>(null);
  const [secrets, setSecrets] = useState<NamedSecret[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [readOnly, setReadOnly] = useState(false);
  // What the approval sent before the host's job began, as done steps.
  const [sent, setSent] = useState<string[] | null>(null);
  useEffect(() => {
    let alive = true;
    Promise.all([ipc.getAgentSettings(), ipc.listNamedSecrets()])
      .then(([s, all]) => {
        if (!alive) return;
        setSettings(s);
        setSecrets(all.filter((n) => routine.secret_ids.includes(n.id)));
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [routine.secret_ids]);
  const held = useHostCopies(routine.host_id);
  const sending = toSend(routine.secret_ids, held);
  const profile = settings?.profiles.find((p) => p.id === routine.profile_id);
  const host = settings?.hosts.find((h) => h.id === routine.host_id);
  const hostName = host?.name ?? item.host_name;
  // What approving does to the host before the schedule can go there.
  const prepare = host ? preparation(host, routine.tools, profile) : [];
  const confirmed = sending.length === 0 || readOnly;
  const job = useHostJobsContext()[routine.host_id];
  const clock = useHostClock(routine.host_id);

  if (sent) {
    return (
      <PreparingDialog
        hostName={hostName}
        job={job}
        sent={sent}
        routineId={routine.id}
        profileId={routine.profile_id}
        onClose={onApproved}
      />
    );
  }

  const approve = () => {
    setBusy(true);
    setError(null);
    ipc
      .approveRoutineSchedule(routine.id)
      .then(() => {
        // A host being prepared: follow it here; else done.
        if (prepare.length > 0) {
          setSent(
            secrets
              .filter((n) => sending.includes(n.env_name))
              .map(
                (n) =>
                  `${n.env_name} sent, as a ${n.as_file ? "file" : "value"}`,
              ),
          );
          setBusy(false);
        } else onApproved();
      })
      .catch((e) => {
        setError(errorMessage(e));
        setBusy(false);
      });
  };

  return (
    <Dialog
      title={`Approve “${routine.name}” on ${hostName}`}
      width={640}
      onClose={onClose}
      footer={
        <>
          {error && (
            <span className="mr-auto text-[12px] text-conflict" role="alert">
              {error}
            </span>
          )}
          <button type="button" className="btn btn-sm" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-sm btn-primary"
            disabled={busy || !profile || !confirmed}
            onClick={approve}
          >
            {prepare.length > 0
              ? `Approve and Prepare ${hostName}`
              : "Approve and Enable"}
          </button>
        </>
      }
    >
      <p className="m-0 mb-3 text-muted">
        {scheduleWords(draft)} {hostName} time
        {clock ? ` (${utcOffsetWords(clock.utc_offset_minutes)})` : ""}, with
        your Mac asleep and no one to ask.{" "}
        {otherZone(clock) &&
          clock &&
          `${hostClockWords(hostName, clock.utc_offset_minutes)} `}
        Changing any of this turns the schedule off until you approve it again.
      </p>
      <dl className="m-0 flex flex-col rounded-lg border">
        <Fact label="Code">
          <code className="mono">{routine.start}</code> of{" "}
          {item.repository_name}, sent to{" "}
          {profile ? providerName(profile.provider) : "…"}. The host runs the
          commit this Mac last sent it.
        </Fact>
        <Fact label="Agent's key">
          {profile
            ? `${profileName(profile)} · ${profile.payment === "claude_plan" ? "Claude plan" : "API key"}`
            : "…"}
          , sealed on {hostName}, so it can start the run with no Mac. Anyone
          who administers the host can use it.
        </Fact>
        <Fact label="Access">
          {secrets.length === 0 ? (
            "None."
          ) : (
            <span className="flex flex-col gap-1">
              {secrets.map((n) => (
                <span key={n.id} className="flex items-center gap-2">
                  <code className="mono">{n.env_name}</code>
                  {sending.includes(n.env_name) ? (
                    <span className="state-pill" data-tone="amber">
                      new · {n.as_file ? "a file" : "a value"}
                    </span>
                  ) : held ? (
                    <span className="state-pill" data-tone="green">
                      already there
                    </span>
                  ) : (
                    <span className="state-pill" data-tone="dashed">
                      {n.as_file ? "a file" : "a value"}
                    </span>
                  )}
                </span>
              ))}
            </span>
          )}
          {routine.tools && (
            <span className="block text-muted">
              Python and the Google Cloud CLI in its image.
            </span>
          )}
        </Fact>
        <Fact label="Host changes">
          {prepare.length > 0
            ? `${capitalize(prepare.join(", and "))}. The schedule goes to ${hostName} when that is done; you can close this meanwhile.`
            : "None."}
        </Fact>
        <Fact label="Network">
          Not restricted: a run can send what it holds anywhere.
        </Fact>
        <Fact label="Limits">
          {durationLabel(routine.time_limit_minutes)} per run; a run over{" "}
          {usd(draft.cost_ceiling_micros)} pauses the schedule, checked after
          each run.
        </Fact>
        <Fact label="Writes">
          None outside its own container. It only proposes tasks, notes and
          issues, which wait for you.
        </Fact>
      </dl>
      {sending.length > 0 && (
        <label
          className="run-card mt-3 flex-row items-start gap-2.5"
          data-tone="amber"
        >
          <input
            type="checkbox"
            className="mt-0.5"
            checked={readOnly}
            onChange={(e) => setReadOnly(e.target.checked)}
          />
          <span>
            {sending.join(", ")} can read and nothing more. Brainiac cannot
            check this, and a run can print it.
          </span>
        </label>
      )}
    </Dialog>
  );
}

function Fact({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex gap-3 border-b px-3 py-2 last:border-b-0">
      <dt className="w-24 shrink-0 text-[12.5px] text-muted">{label}</dt>
      <dd className="m-0 min-w-0 flex-1">{children}</dd>
    </div>
  );
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

type Shown = {
  title: string;
  state: HostJobStep["state"];
  detail: string;
};

/**
 * Preparing the host, followed here: what the approval already sent, the
 * host job's steps as they run, then the schedule turned on. Whether it is
 * on is the routine's schedule's answer, read again until it leaves
 * Preparing, never the job's: the approval can still fail after the job,
 * and the job shown can be another one the host had to finish first.
 * Closing it changes nothing; the routine's Schedule tab shows the same.
 */
function PreparingDialog({
  hostName,
  job,
  sent,
  routineId,
  profileId,
  onClose,
}: {
  hostName: string;
  job: HostJob | undefined;
  sent: string[];
  routineId: string;
  profileId: string;
  onClose: () => void;
}) {
  const [schedule, setSchedule] = useState<RoutineSchedule | null>(null);
  const settled = schedule !== null && schedule.state !== "preparing";
  useEffect(() => {
    if (settled) return;
    let alive = true;
    const read = () =>
      ipc
        .listRoutines()
        .then((all) => {
          const found = all.find((i) => i.routine.id === routineId);
          if (alive && found?.schedule) setSchedule(found.schedule);
        })
        .catch(() => {});
    void read();
    const timer = setInterval(read, 2000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [routineId, settled]);
  const on =
    settled && (schedule.state === "active" || schedule.state === "paused");
  const off = settled && !on;
  // This routine's preparation, or another job the host finishes first.
  const ours = job?.kind === "prepare" && job.profile_ids.includes(profileId);
  const shown: Shown[] = [
    ...sent.map((title) => ({ title, state: "done" as const, detail: "" })),
    ...(ours
      ? job.steps.map((st) => ({
          title: st.title,
          state: st.state,
          detail:
            st.state === "skipped"
              ? st.detail
              : st.state === "failed"
                ? (job.error ?? st.detail)
                : (st.progress ?? ""),
        }))
      : job?.state === "running"
        ? [
            {
              title: `Waiting for ${hostName} to finish: ${jobTitle(job)}`,
              state: "running" as const,
              detail: "This routine's preparation starts after it.",
            },
          ]
        : []),
    {
      title: "Agent's key sealed, code sent, and schedule turned on",
      state: on ? "done" : off ? "failed" : "waiting",
      detail: off ? (schedule?.notice ?? "") : "",
    },
  ];
  const done = shown.filter(
    (st) => st.state === "done" || st.state === "skipped",
  ).length;
  return (
    <Dialog
      title={
        on
          ? `${hostName} is prepared`
          : off
            ? `Preparing ${hostName} did not finish`
            : `Preparing ${hostName} · ${done} of ${shown.length}`
      }
      width={600}
      onClose={onClose}
      footer={
        <button
          type="button"
          className="btn btn-sm btn-primary"
          onClick={onClose}
        >
          {settled ? "Done" : "Close"}
        </button>
      }
    >
      <p className="m-0 mb-3 text-muted">
        {on
          ? `The schedule is on ${hostName}.`
          : off
            ? "The schedule stays off; the last step says why. Nothing that was already sent is undone. Approve again to retry."
            : `You can close this. If ${hostName} stops answering, it picks up where it was when it is back.`}
      </p>
      <ol className="m-0 flex list-none flex-col p-0">
        {shown.map((st, i) => (
          <li
            key={st.title}
            className="flex items-start gap-2.5 border-b py-2 last:border-b-0"
          >
            {st.state === "waiting" ? (
              <span
                aria-hidden="true"
                className="flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-control text-[10px] font-semibold text-muted"
              >
                {i + 1}
              </span>
            ) : (
              <StepIcon state={st.state} />
            )}
            <span className="flex min-w-0 flex-col">
              <span className={st.state === "waiting" ? "text-fg-2" : ""}>
                {st.title}
              </span>
              {st.detail && (
                <span
                  className={`text-[12px] ${st.state === "failed" ? "text-conflict" : "text-muted"}`}
                >
                  {st.detail}
                </span>
              )}
            </span>
          </li>
        ))}
      </ol>
      {!settled && (
        <p className="m-0 mt-3 text-[12px] text-muted">
          If a step fails, the schedule stays off and the step says what to do.
          Nothing that was already sent is undone.
        </p>
      )}
    </Dialog>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="overflow-hidden rounded-lg border">
      <h3 className="m-0 border-b bg-panel px-3 py-2 text-[13px] font-semibold">
        {title}
      </h3>
      {children}
    </section>
  );
}

function Row({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1 border-b px-3 py-2.5 last:border-b-0">
      <span className="text-[12.5px] font-medium">{label}</span>
      {children}
    </div>
  );
}

function Hint({ children }: { children: React.ReactNode }) {
  return <span className="text-[12px] text-muted">{children}</span>;
}

/**
 * What preparing the host for a routine does, in words, in the order the
 * job does it; empty when the host is ready (`needs_preparing`).
 */
function preparation(
  host: AgentHost,
  tools: boolean,
  profile: AgentProfile | undefined,
): string[] {
  const steps: string[] = [];
  if (host.upgrade_available)
    steps.push(
      `upgrade its run controller${host.available_build ? ` to ${host.available_build}` : ""}`,
    );
  const built = (i: AgentHost["image"]) => !!i?.current;
  if (!built(host.image) || (tools && !built(host.tools_image)))
    steps.push(
      tools
        ? "build its image with Python and the Google Cloud CLI"
        : "build its image",
    );
  // A schedule starts runs of an agent that passed its test there.
  if (
    profile &&
    !host.tests.some((t) => t.profile_id === profile.id && t.current)
  )
    steps.push(`test ${profileName(profile)} there`);
  return steps;
}
