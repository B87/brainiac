import { useEffect, useState } from "react";
import { durationLabel, providerName } from "../lib/agentRuns";
import { relativeTime } from "../lib/format";
import { currentStep, useHostJobsContext } from "../lib/hostJobs";
import {
  type AgentHost,
  type AgentSettings,
  errorMessage,
  ipc,
  type NamedSecret,
  type RoutineOverview,
  type ScheduleWhen,
} from "../lib/ipc";
import { hostTime, scheduleWords, usd } from "../lib/routines";
import { toSend, useHostCopies } from "./AccessPicker";
import Dialog from "./Dialog";

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
            In the host's time. The host fires it; this Mac can be asleep.
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
 * What unattended runs may do, once, before the schedule is turned on. The
 * same list as New run's "Before you start", for a run with nobody there.
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
  // What approving does to the host before the schedule can go there.
  const prepare = host ? preparation(host, routine.tools) : [];
  const confirmed = sending.length === 0 || readOnly;
  return (
    <Dialog
      title="Approve schedule"
      width={600}
      onClose={onClose}
      footer={
        <>
          {error && (
            <span className="mr-auto text-[12px] text-conflict" role="alert">
              {error}
            </span>
          )}
          <button type="button" className="btn btn-sm" onClick={onClose}>
            Later
          </button>
          <button
            type="button"
            className="btn btn-sm btn-primary"
            disabled={busy || !profile || !confirmed}
            onClick={() => {
              setBusy(true);
              setError(null);
              ipc
                .approveRoutineSchedule(routine.id)
                .then(onApproved)
                .catch((e) => {
                  setError(errorMessage(e));
                  setBusy(false);
                });
            }}
          >
            {prepare.length > 0
              ? `Approve and Prepare ${host?.name ?? "the Host"}`
              : "Approve and Enable"}
          </button>
        </>
      }
    >
      <p className="m-0 mb-2 text-muted">
        {routine.name} will run on its own, with nobody to ask. It may do the
        following. Changing any of it turns the schedule off until you approve
        it again.
      </p>
      <ul className="m-0 flex flex-col gap-1.5 pl-5">
        <li>
          <strong>Code:</strong> {item.repository_name}, the {routine.start}{" "}
          commit, sent to{" "}
          <strong>{profile ? providerName(profile.provider) : "…"}</strong>. The
          host runs the commit this Mac last sent it.
        </li>
        <li>
          <strong>Prompt:</strong> "{routine.prompt.slice(0, 120)}
          {routine.prompt.length > 120 ? "…" : ""}"
        </li>
        <li>
          <strong>Host:</strong> {host?.name ?? item.host_name},{" "}
          {scheduleWords(draft)} in its time.
        </li>
        <li>
          <strong>The agent's key:</strong> a copy is kept sealed on the host,
          so it can start the run with no Mac. Anyone who administers the host
          can use it.
        </li>
        <li>
          <strong>Network:</strong> unrestricted. A run can reach any site.
        </li>
        <li>
          <strong>Access:</strong>{" "}
          {secrets.length === 0
            ? "none."
            : secrets.map((n, i) => (
                <span key={n.id}>
                  {i > 0 && ", "}
                  <code className="mono">{n.env_name}</code>{" "}
                  <span className="text-muted">
                    ({n.as_file ? "a file" : "a value"}
                    {sending.includes(n.env_name)
                      ? `, sent to ${host?.name ?? "the host"} now`
                      : held
                        ? ", already there"
                        : ""}
                    )
                  </span>
                </span>
              ))}
          {routine.tools &&
            " Python and the Google Cloud CLI are in its image."}
        </li>
        {prepare.length > 0 && (
          <li>
            <strong>Host changes:</strong> {prepare.join(", and ")}. The
            schedule goes to {host?.name} when that is done; you can close this
            meanwhile.
          </li>
        )}
        <li>
          <strong>Limits:</strong> {durationLabel(routine.time_limit_minutes)}{" "}
          and a {usd(draft.cost_ceiling_micros)} cost ceiling, checked after
          each run.
        </li>
        <li>
          <strong>Writes:</strong> none outside its own container. It only
          proposes tasks, notes and issues, which wait for you.
        </li>
      </ul>
      {sending.length > 0 && (
        <label className="mt-3 flex items-start gap-2 rounded-md bg-panel px-3 py-2">
          <input
            type="checkbox"
            checked={readOnly}
            onChange={(e) => setReadOnly(e.target.checked)}
          />
          <span>
            {sending.join(", ")} can read and nothing more. Brainiac cannot
            check what a token allows, and a run can print it.
          </span>
        </label>
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

/** What preparing the host for a routine does, in words; empty when ready. */
function preparation(host: AgentHost, tools: boolean): string[] {
  const steps: string[] = [];
  if (host.upgrade_available)
    steps.push(
      `upgrade its Brainiac helper${host.available_build ? ` to ${host.available_build}` : ""}`,
    );
  const built = (i: AgentHost["image"]) => !!i?.current;
  if (!built(host.image) || (tools && !built(host.tools_image)))
    steps.push("build its image with Python and the Google Cloud CLI");
  return steps;
}
