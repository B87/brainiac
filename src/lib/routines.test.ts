import { describe, expect, it } from "vitest";
import type { BriefingResult, RoutineOverview, RoutineSchedule } from "./ipc";
import {
  hostClockWords,
  hostTime,
  inTodayDigest,
  resultWord,
  routineNeedsYou,
  routineState,
  scheduleWords,
  sortRoutines,
  usd,
  utcOffsetWords,
} from "./routines";

function result(over: Partial<BriefingResult>): BriefingResult {
  return {
    id: "r",
    routine_id: "b",
    run_id: null,
    state: "ready",
    status: null,
    summary: "",
    findings: [],
    proposals: [],
    error: null,
    started_at: "2026-10-14T07:00:00",
    finished_at: null,
    duration_secs: null,
    cost_micros: null,
    cost_currency: null,
    ...over,
  };
}
function item(name: string, latest: BriefingResult | null): RoutineOverview {
  return {
    routine: {
      id: name,
      name,
      repository_id: "repo",
      profile_id: "claude-code",
      host_id: "",
      start: "HEAD",
      prompt: "p",
      time_limit_minutes: 30,
      secret_ids: [],
      tools: false,
      memory: "",
      memory_updated_at: null,
      previous_results: 3,
      version: 1,
    },
    repository_name: "api",
    host_name: "This Mac",
    latest,
    schedule: null,
  };
}

describe("a routine's state", () => {
  it("is worded from its latest briefing, and a failure is never 'nothing found'", () => {
    expect(resultWord(result({ status: "nothing_found" }))).toBe(
      "Nothing found",
    );
    expect(resultWord(result({ status: null }))).toBe("Report");
    expect(resultWord(result({ state: "failed", error: "x" }))).toBe(
      "No briefing",
    );
    expect(
      resultWord(
        result({
          status: "findings",
          findings: [
            {
              key: "a",
              title: "t",
              severity: null,
              detail: "",
              link: "",
              seen_before: false,
            },
          ],
        }),
      ),
    ).toBe("1 finding");
    expect(routineState(item("a", null)).word).toBe("Not run yet");
    expect(routineState(item("a", result({ state: "failed" }))).tone).toBe(
      "bad",
    );
  });

  it("puts what needs the user first and counts failures", () => {
    const list = sortRoutines([
      item("b", result({ status: "nothing_found" })),
      item("a", result({ state: "working" })),
      item("c", result({ state: "failed" })),
      item("d", null),
    ]);
    expect(list.map((i) => i.routine.name)).toEqual(["c", "a", "b", "d"]);
    expect(list.filter(routineNeedsYou)).toHaveLength(1);
  });

  it("shows on Today only when it began today, is working, or failed", () => {
    const started = (day: string) => `${day}T07:00:00`;
    expect(
      inTodayDigest(
        item("a", result({ started_at: started("2026-10-14") })),
        "2026-10-14",
      ),
    ).toBe(true);
    expect(
      inTodayDigest(
        item("a", result({ started_at: started("2026-10-13") })),
        "2026-10-14",
      ),
    ).toBe(false);
    expect(
      inTodayDigest(
        item(
          "a",
          result({ state: "failed", started_at: started("2026-10-01") }),
        ),
        "2026-10-14",
      ),
    ).toBe(true);
    expect(
      inTodayDigest(
        item(
          "a",
          result({ state: "working", started_at: started("2026-10-14") }),
        ),
        "2026-10-14",
      ),
    ).toBe(true);
    expect(inTodayDigest(item("a", null), "2026-10-14")).toBe(false);
  });
});

describe("a routine's schedule", () => {
  const schedule = (state: RoutineSchedule["state"]): RoutineSchedule => ({
    when: {
      days: "weekdays",
      weekday: 0,
      time: "07:00",
      late_start: false,
      cost_ceiling_micros: 1_000_000,
    },
    state,
    notice: null,
    changed: [],
    next_fire: null,
    synced_at: null,
    code_commit: null,
    code_at: null,
    error: null,
    month_cost_micros: 0,
  });
  const scheduled = (
    state: RoutineSchedule["state"],
    latest: BriefingResult | null,
  ) => ({
    ...item("a", latest),
    schedule: schedule(state),
  });

  it("says what needs the user before the latest briefing, and counts it", () => {
    const quiet = result({ status: "nothing_found" });
    expect(routineState(scheduled("needs_approval", quiet)).word).toBe(
      "Needs approval",
    );
    expect(routineState(scheduled("overdue", quiet)).word).toBe("Overdue");
    expect(routineState(scheduled("paused", quiet)).word).toBe("Paused");
    expect(routineState(scheduled("unreachable", quiet)).word).toBe(
      "Host unreachable",
    );
    expect(routineState(scheduled("active", quiet)).word).toBe("Nothing found");
    expect(routineNeedsYou(scheduled("needs_approval", quiet))).toBe(true);
    expect(routineNeedsYou(scheduled("overdue", null))).toBe(true);
    expect(routineNeedsYou(scheduled("paused", quiet))).toBe(false);
    // An old briefing does not keep a routine off Today when it needs approval.
    expect(
      inTodayDigest(scheduled("needs_approval", quiet), "2026-10-20"),
    ).toBe(true);
    expect(inTodayDigest(scheduled("paused", quiet), "2026-10-20")).toBe(false);
  });

  it("is worded in the host's own time", () => {
    expect(scheduleWords(schedule("active").when)).toBe("Weekdays at 07:00");
    expect(
      scheduleWords({
        ...schedule("active").when,
        days: "weekly",
        weekday: 4,
        time: "16:00",
      }),
    ).toBe("Weekly on Friday at 16:00");
    // Read from the text, not converted: 07:00 in the host's zone stays 07:00.
    expect(hostTime("2026-10-14T07:00:00+02:00")).toBe("Wed 07:00");
    expect(hostTime("2026-10-14T07:00:00-09:00")).toBe("Wed 07:00");
    expect(hostTime(null)).toBeNull();
    expect(usd(410_000)).toBe("$0.41");
  });
});

describe("a host's clock beside a schedule's time", () => {
  const now = new Date(Date.UTC(2026, 9, 17, 17, 23));
  // This Mac's offset from UTC, whatever machine runs the test.
  const mac = -now.getTimezoneOffset();

  it("names the host's time and zone", () => {
    expect(hostClockWords("vegeta", 120, now)).toMatch(
      /^It is 19:23 on vegeta now \(UTC\+2\), /,
    );
    expect(hostClockWords("vegeta", 0, now)).toMatch(/17:23 .*\(UTC\)/);
    expect(utcOffsetWords(-210)).toBe("UTC−3:30");
  });

  it("says how far it is from this Mac", () => {
    expect(hostClockWords("h", mac, now)).toMatch(
      /the same time as this Mac\.$/,
    );
    expect(hostClockWords("h", mac - 120, now)).toMatch(
      /2 hours behind this Mac\.$/,
    );
    expect(hostClockWords("h", mac + 60, now)).toMatch(
      /1 hour ahead of this Mac\.$/,
    );
    expect(hostClockWords("h", mac + 330, now)).toMatch(
      /5:30 hours ahead of this Mac\.$/,
    );
  });
});
