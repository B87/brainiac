import { useCallback, useEffect, useState } from "react";
import { currentStep, useHostJobsContext } from "../lib/hostJobs";
import {
  type AgentHost,
  errorMessage,
  type HostScheduleOnly,
  ipc,
  type RoutineOverview,
} from "../lib/ipc";
import { hostTime, routineState, scheduleWords } from "../lib/routines";
import { Card, Hint } from "./SettingsPanes";
import StatePill from "./StatePill";

/**
 * Routines on a run host (docs/design/routines-ux.md): each routine the host
 * fires, with its next time and state, and Pause all. A host that cannot be
 * reached shows its schedules as overdue or unreachable, not as fine; one
 * being prepared says which step the host is at.
 */
export default function HostSchedulesGroup({ host }: { host: AgentHost }) {
  const [items, setItems] = useState<RoutineOverview[] | null>(null);
  /** What the host fires for no routine of this Mac. */
  const [only, setOnly] = useState<HostScheduleOnly[]>([]);
  /** The one whose removal is being confirmed. */
  const [removing, setRemoving] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const job = useHostJobsContext()[host.id];
  const load = useCallback(async () => {
    try {
      const all = await ipc.listRoutines();
      setItems(
        all.filter((i) => i.routine.host_id === host.id && i.schedule !== null),
      );
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [host.id]);
  useEffect(() => {
    void load();
  }, [load]);
  // Asks the host, so it comes after the routines and may take a moment.
  useEffect(() => {
    let alive = true;
    ipc
      .hostSchedulesOnly(host.id)
      .then((list) => alive && setOnly(list))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [host.id]);
  const removeOnly = async (routineId: string) => {
    setBusy(true);
    setError(null);
    try {
      setOnly(await ipc.removeHostScheduleOnly(host.id, routineId));
      setRemoving(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const approved = (items ?? []).filter(
    (i) => i.schedule && i.schedule.state !== "needs_approval",
  );
  const allPaused =
    approved.length > 0 &&
    approved.every((i) => i.schedule?.state === "paused");
  const toggleAll = async () => {
    setBusy(true);
    setError(null);
    try {
      for (const i of approved) {
        await ipc.pauseRoutine(i.routine.id, !allPaused);
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
      void load();
    }
  };

  return (
    <Card
      title="Routines on this host"
      actions={
        approved.length > 0 && (
          <button
            type="button"
            className="btn btn-sm"
            disabled={busy}
            onClick={() => void toggleAll()}
          >
            {allPaused ? "Resume All" : "Pause All"}
          </button>
        )
      }
    >
      {items?.length === 0 && only.length === 0 && (
        <div className="settings-row text-[12.5px] text-muted">
          None. A routine's Schedule tab puts one here; {host.name} then fires
          it at its time, with this Mac off or asleep, and keeps what it leaves
          until Brainiac next reaches it.
        </div>
      )}
      {items?.map((i) => {
        const st = routineState(i);
        // While the host is prepared for it, the step it is at.
        const step =
          st.key === "preparing" && job?.state === "running"
            ? job.steps[currentStep(job)]
            : null;
        const word = step
          ? `Preparing · ${step.progress ?? step.title}`
          : st.word;
        return (
          <div
            key={i.routine.id}
            className="settings-row flex-wrap items-center gap-x-3 gap-y-1"
          >
            <span className="min-w-0 flex-1 truncate font-medium">
              {i.routine.name}
            </span>
            <span className="text-[12.5px] text-muted">
              {i.schedule ? scheduleWords(i.schedule.when) : ""}
              {i.schedule?.state === "active" &&
                ` · next ${hostTime(i.schedule.next_fire) ?? "…"}`}
            </span>
            <StatePill tone={st.tone}>{word}</StatePill>
          </div>
        );
      })}
      {only.map((o) => (
        <fieldset
          key={o.routine_id}
          aria-label={`${o.name}, not a routine of this Mac`}
          className="settings-row m-0 flex-wrap items-center gap-x-3 gap-y-1 border-0"
        >
          <span className="flex min-w-55 flex-1 flex-col gap-0.5">
            <span className="font-medium">{o.name}</span>
            <span className="text-[12px] text-muted">
              Not a routine of this Mac: another Mac's, or one this Mac made
              before it was restored from an older backup.{" "}
              {o.enabled
                ? `It still fires${o.next_fire ? `, next ${hostTime(o.next_fire) ?? "…"}` : ""}.`
                : "It is off."}
              {o.results_waiting > 0 &&
                ` ${o.results_waiting} ${o.results_waiting === 1 ? "briefing waits" : "briefings wait"} there.`}
            </span>
          </span>
          {removing === o.routine_id ? (
            <span className="flex gap-2">
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => setRemoving(null)}
              >
                Keep It
              </button>
              <button
                type="button"
                className="btn btn-sm text-conflict"
                disabled={busy}
                onClick={() => void removeOnly(o.routine_id)}
              >
                Remove{o.results_waiting > 0 ? " With Its Briefings" : ""}
              </button>
            </span>
          ) : (
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={() => setRemoving(o.routine_id)}
            >
              Remove from Host…
            </button>
          )}
        </fieldset>
      ))}
      {error && (
        <div className="settings-row text-[12.5px] text-conflict" role="alert">
          {error}
        </div>
      )}
      {!items && !error && (
        <div className="settings-row">
          <Hint>Loading…</Hint>
        </div>
      )}
    </Card>
  );
}
