import { useCallback, useEffect, useState } from "react";
import { currentStep, useHostJobsContext } from "../lib/hostJobs";
import {
  type AgentHost,
  errorMessage,
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
      {items?.length === 0 && (
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
