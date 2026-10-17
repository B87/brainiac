import { relativeTime } from "../lib/format";
import type { RoutineOverview } from "../lib/ipc";
import { inTodayDigest, routineState } from "../lib/routines";
import StatePill from "./StatePill";

/**
 * Routines on Today (docs/design/routines-ux.md): one line each, only for a
 * routine whose briefing began today, is working, or failed. Quiet routines
 * stay in the Routines section; with none to show, nothing is drawn.
 */
export default function RoutinesDigest({
  routines,
  today,
  onOpen,
  onAll,
}: {
  routines: RoutineOverview[];
  today: string;
  onOpen: (routineId: string) => void;
  onAll: () => void;
}) {
  const rows = routines.filter((i) => inTodayDigest(i, today));
  if (rows.length === 0) return null;
  return (
    <section
      aria-label="Routines"
      className="mx-4 mt-4 overflow-hidden rounded-lg border"
    >
      <div className="flex items-center border-b bg-panel px-3 py-1.5">
        <span className="section-label flex-1">Routines</span>
        <button
          type="button"
          className="btn btn-sm btn-ghost text-link"
          onClick={onAll}
        >
          All routines →
        </button>
      </div>
      {rows.map((i) => {
        const st = routineState(i);
        const r = i.latest;
        return (
          <button
            key={i.routine.id}
            type="button"
            className="flex w-full items-center gap-3 border-0 border-b bg-transparent px-3 py-2 text-left font-[inherit] last:border-b-0 hover:bg-panel"
            onClick={() => onOpen(i.routine.id)}
          >
            <span className="min-w-0 flex-1 truncate font-medium">
              {i.routine.name}
            </span>
            <StatePill tone={st.tone}>{st.word}</StatePill>
            {r && (
              <span className="text-[12px] text-muted">
                {relativeTime(r.finished_at ?? r.started_at)}
              </span>
            )}
          </button>
        );
      })}
    </section>
  );
}
