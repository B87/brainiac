import { describe, expect, it } from "vitest";
import type { BriefingResult, RoutineOverview } from "./ipc";
import {
  inTodayDigest,
  resultWord,
  routineNeedsYou,
  routineState,
  sortRoutines,
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
      memory: "",
      memory_updated_at: null,
      previous_results: 3,
      version: 1,
    },
    repository_name: "api",
    host_name: "This Mac",
    latest,
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
