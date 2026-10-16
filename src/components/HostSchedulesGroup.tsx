import { useCallback, useEffect, useState } from "react";
import {
  type AgentHost,
  errorMessage,
  ipc,
  type RoutineOverview,
} from "../lib/ipc";
import { hostTime, routineState, scheduleWords } from "../lib/routines";
import { Group, Hint } from "./SettingsPanes";
import StatePill from "./StatePill";

/**
 * Schedules on a run host (docs/design/routines-ux.md): each routine the host
 * fires, with its next time and state, and Pause all. A host that cannot be
 * reached shows its schedules as overdue or unreachable, not as fine.
 */
export default function HostSchedulesGroup({ host }: { host: AgentHost }) {
  const [items, setItems] = useState<RoutineOverview[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
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
    <Group label="Schedules on this host">
      <div className="settings-group flex flex-col">
        {items?.length === 0 && (
          <div className="settings-row text-[12.5px] text-muted">
            No schedules here. A routine's Schedule tab sets one.
          </div>
        )}
        {items?.map((i) => {
          const st = routineState(i);
          return (
            <div
              key={i.routine.id}
              className="settings-row flex-wrap items-center gap-x-3 gap-y-1"
            >
              <span className="min-w-0 flex-1 truncate font-medium">
                {i.routine.name}{" "}
                <span className="font-normal text-muted">
                  {i.schedule ? scheduleWords(i.schedule.when) : ""}
                  {i.schedule?.state === "active" &&
                    ` · next ${hostTime(i.schedule.next_fire) ?? "…"}`}
                </span>
              </span>
              <StatePill tone={st.tone}>{st.word}</StatePill>
            </div>
          );
        })}
        {error && (
          <div
            className="settings-row text-[12.5px] text-conflict"
            role="alert"
          >
            {error}
          </div>
        )}
        {approved.length > 0 && (
          <div className="settings-row">
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={() => void toggleAll()}
            >
              {allPaused ? "Resume All" : "Pause All"}
            </button>
          </div>
        )}
      </div>
      <Hint>
        {host.name} fires these at their time, with this Mac off or asleep, and
        keeps what they leave until Brainiac next reaches it.
      </Hint>
    </Group>
  );
}
