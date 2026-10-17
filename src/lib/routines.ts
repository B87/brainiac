import type { BriefingResult, RoutineOverview, ScheduleWhen } from "./ipc";

/**
 * What a routine's row says (docs/design/routines-ux.md): the words come from
 * the latest briefing, and a run that left none is "No briefing", never
 * "Nothing found". Order puts what needs the user first.
 */
export type RoutineStateKey =
  | "working"
  | "failed"
  | "approval"
  | "overdue"
  | "unreachable"
  | "paused"
  | "preparing"
  | "ready"
  | "never";
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
  const schedule = item.schedule;
  if (r?.state === "working")
    return { key: "working", word: "Working", tone: "work", order: 1 };
  if (schedule?.state === "preparing")
    return { key: "preparing", word: "Preparing host", tone: "work", order: 1 };
  if (schedule?.state === "needs_approval")
    return { key: "approval", word: "Needs approval", tone: "warn", order: 0 };
  if (schedule?.state === "overdue")
    return { key: "overdue", word: "Overdue", tone: "bad", order: 0 };
  if (r?.state === "failed")
    return { key: "failed", word: "No briefing", tone: "bad", order: 0 };
  if (schedule?.state === "paused")
    return { key: "paused", word: "Paused", tone: "idle", order: 4 };
  if (schedule?.state === "unreachable")
    return {
      key: "unreachable",
      word: "Host unreachable",
      tone: "warn",
      order: 2,
    };
  if (!r) return { key: "never", word: "Not run yet", tone: "idle", order: 5 };
  return {
    key: "ready",
    word: resultWord(r),
    tone: r.status === "findings" ? "warn" : "ok",
    order: 3,
  };
}

/**
 * What needs the user: a latest briefing that failed, a schedule waiting for
 * approval, one that is overdue. The sidebar's badge counts these.
 */
export function routineNeedsYou(item: RoutineOverview): boolean {
  return ["failed", "approval", "overdue"].includes(routineState(item).key);
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
  if (routineNeedsYou(item)) return true;
  if (!r) return false;
  if (r.state === "working") return true;
  return localDay(r.started_at) === today;
}

function localDay(iso: string): string {
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

const DAYS = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
];

/** "Weekdays at 07:00", "Weekly on Friday at 16:00". The host's time. */
export function scheduleWords(when: ScheduleWhen): string {
  const day =
    when.days === "daily"
      ? "Daily"
      : when.days === "weekdays"
        ? "Weekdays"
        : `Weekly on ${DAYS[when.weekday] ?? "?"}`;
  return `${day} at ${when.time}`;
}

/**
 * A time the host sent, as the host's own clock showed it ("Wed 07:00"): the
 * text of an RFC 3339 time is already in the host's local time, so it is read
 * rather than converted to this Mac's.
 */
export function hostTime(rfc3339: string | null): string | null {
  const m = rfc3339?.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}:\d{2})/);
  if (!m) return null;
  const day = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).getDay();
  const names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  return `${names[day]} ${m[4]}`;
}

/** "UTC+2", "UTC−3:30", "UTC". */
export function utcOffsetWords(minutes: number): string {
  if (minutes === 0) return "UTC";
  const sign = minutes > 0 ? "+" : "−";
  const h = Math.floor(Math.abs(minutes) / 60);
  const m = Math.abs(minutes) % 60;
  return `UTC${sign}${h}${m ? `:${String(m).padStart(2, "0")}` : ""}`;
}

/**
 * Where a host's clock stands, for the hint beside a schedule's time: "It is
 * 17:23 on vegeta now (UTC), 2 hours behind this Mac." `now` is this Mac's
 * time; the host's is worked out from its offset, so it does not go stale.
 */
export function hostClockWords(
  hostName: string,
  offsetMinutes: number,
  now: Date = new Date(),
): string {
  const there = new Date(now.getTime() + offsetMinutes * 60_000);
  const hhmm = `${String(there.getUTCHours()).padStart(2, "0")}:${String(there.getUTCMinutes()).padStart(2, "0")}`;
  // getTimezoneOffset is UTC minus local time, in minutes.
  const diff = offsetMinutes + now.getTimezoneOffset();
  const hours = Math.abs(diff) / 60;
  const span = Number.isInteger(hours)
    ? `${hours} ${hours === 1 ? "hour" : "hours"}`
    : `${Math.floor(hours)}:${String(Math.abs(diff) % 60).padStart(2, "0")} hours`;
  const relation =
    diff === 0
      ? "the same time as this Mac"
      : `${span} ${diff > 0 ? "ahead of" : "behind"} this Mac`;
  return `It is ${hhmm} on ${hostName} now (${utcOffsetWords(offsetMinutes)}), ${relation}.`;
}

/** Millionths of a US dollar as "$0.41". */
export function usd(micros: number): string {
  return `$${(micros / 1_000_000).toFixed(2)}`;
}
