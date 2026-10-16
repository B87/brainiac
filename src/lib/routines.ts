import type { BriefingResult, RoutineOverview } from "./ipc";

/**
 * What a routine's row says (docs/design/routines-ux.md): the words come from
 * the latest briefing, and a run that left none is "No briefing", never
 * "Nothing found". Order puts what needs the user first.
 */
export type RoutineStateKey = "working" | "failed" | "ready" | "never";
export type Tone = "ok" | "warn" | "bad" | "work" | "idle";

export type RoutineState = {
  key: RoutineStateKey;
  word: string;
  tone: Tone;
  /** Lower sorts first. */
  order: number;
};

/** "2 findings", "Nothing found", "Report", or "No briefing". */
export function resultWord(r: BriefingResult): string {
  if (r.state === "working") return "Working";
  if (r.state === "failed") return "No briefing";
  if (r.status === "findings") {
    const n = r.findings.length;
    return n === 1 ? "1 finding" : `${n} findings`;
  }
  if (r.status === "nothing_found") return "Nothing found";
  return "Report";
}

export function routineState(item: RoutineOverview): RoutineState {
  const r = item.latest;
  if (!r) return { key: "never", word: "Not run yet", tone: "idle", order: 5 };
  if (r.state === "working")
    return { key: "working", word: "Working", tone: "work", order: 1 };
  if (r.state === "failed")
    return { key: "failed", word: "No briefing", tone: "bad", order: 0 };
  return {
    key: "ready",
    word: resultWord(r),
    tone: r.status === "findings" ? "warn" : "ok",
    order: 3,
  };
}

/** A failed latest briefing needs the user; the badge counts these. */
export function routineNeedsYou(item: RoutineOverview): boolean {
  return routineState(item).key === "failed";
}

/** Needs-you first, then working, then by name. */
export function sortRoutines(items: RoutineOverview[]): RoutineOverview[] {
  return [...items].sort(
    (a, b) =>
      routineState(a).order - routineState(b).order ||
      a.routine.name.localeCompare(b.routine.name),
  );
}

/**
 * Today's digest: a routine whose latest briefing began today, is working, or
 * failed. Quiet routines stay in the Routines section. `today` is a local
 * date, "YYYY-MM-DD".
 */
export function inTodayDigest(item: RoutineOverview, today: string): boolean {
  const r = item.latest;
  if (!r) return false;
  if (r.state === "working" || r.state === "failed") return true;
  return localDay(r.started_at) === today;
}

function localDay(iso: string): string {
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}
