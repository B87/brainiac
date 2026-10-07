import { describe, expect, it } from "vitest";
import {
  activityLabel,
  activityTone,
  durationLabel,
  engineSummary,
  foldTurns,
  groupOf,
  imageSummary,
  listResult,
  listWhen,
  producedLabel,
  samePreview,
  settingsRequest,
  spanLabel,
  startSteps,
  timeLeft,
  turnSummary,
  visibilityLabel,
} from "./agentRuns";
import type { AgentEngine, AgentProfile, AgentRun, RunEvent } from "./ipc";

const profile: AgentProfile = {
  id: "claude-code",
  engine_socket: null,
  payment: "api_key",
  credential_source: { kind: "none" },
  credential: { needs_approval: false, pending: null, revision: 1 },
  credential_saved_at: null,
  credential_ageing: false,
  test_passed_at: null,
  test_current: false,
  sends_code_agreed: false,
  permissions: "ask",
  time_limit_minutes: 60,
  cpus: 4,
  memory_mib: 8192,
  workspace_gib: 20,
  model: "",
  image: null,
  version: 3,
};

describe("Settings → Agents", () => {
  it("names time limits", () => {
    expect(durationLabel(30)).toBe("30 minutes");
    expect(durationLabel(60)).toBe("1 hour");
    expect(durationLabel(480)).toBe("8 hours");
    expect(durationLabel(90)).toBe("1 h 30 min");
  });

  it("saves the shown version with the change", () => {
    const request = settingsRequest(profile, { permissions: "act" });
    expect(request.expected_version).toBe(3);
    expect(request.permissions).toBe("act");
    expect(request.time_limit_minutes).toBe(60);
  });

  it("summarizes an engine, or says why it cannot be used", () => {
    const engine: AgentEngine = {
      socket: "/Users/someone/.orbstack/run/docker.sock",
      name: "OrbStack",
      reachable: true,
      supported: true,
      problem: null,
      server_version: "28.3.2",
      api_version: "1.51",
      cpus: 8,
      memory_bytes: 16 * 1024 ** 3,
    };
    expect(engineSummary(engine)).toBe(
      "Docker 28.3.2 · API 1.51 · 8 CPUs · 16 GB",
    );
    expect(
      engineSummary({ ...engine, reachable: false, problem: "Not running." }),
    ).toBe("Not running.");
  });

  it("shortens an image ID", () => {
    expect(
      imageSummary({
        name: "brainiac-claude:3f9c41e1a2b0",
        id: "sha256:9f3c41e1aaaaaaaaaaaaaaaa",
        built_at: "2026-10-05T12:00:00Z",
        current: true,
      }),
    ).toBe("brainiac-claude:3f9c41e1a2b0 · sha256:9f3c41e1aaaa…");
  });
});

describe("Runs", () => {
  const run: AgentRun = {
    id: "run-1",
    repository_id: "repo-1",
    repository_name: "example",
    title: "Fix the build",
    start_commit: "0123456789abcdef0123456789abcdef01234567",
    start_subject: "Start",
    payment: "api_key",
    credential_source: "the Keychain",
    host_id: "local",
    host_name: "This Mac",
    engine_name: "OrbStack",
    image_name: "brainiac-claude:abc",
    permissions: "ask",
    time_limit_minutes: 60,
    cpus: 4,
    memory_mib: 8192,
    workspace_gib: 20,
    model: "",
    model_used: null,
    phase: "running",
    activity: "working",
    turn: 2,
    outcome: null,
    stop_confirmed: false,
    kept: true,
    accepted_at: "2026-10-05T10:00:00Z",
    deadline_at: "2026-10-05T11:00:00Z",
    ended_at: null,
    expired_asleep: false,
    error: null,
    pending_permissions: [],
    connected: true,
    reported_at: "2026-10-05T10:30:00Z",
    cancel_requested: false,
    collection: "none",
    collection_error: null,
    result_commit: null,
    changed_files: null,
    left_out: [],
    left_out_more: 0,
    snapshot_accepted: false,
    cleanup_pending: null,
    cursor: 12,
    created_at: "2026-10-05T10:00:00Z",
    updated_at: "2026-10-05T10:30:00Z",
    version: 4,
    starting: null,
  };

  it("groups runs by what they wait for", () => {
    expect(groupOf(run)).toBe("active");
    expect(groupOf({ ...run, activity: "preparing" })).toBe("active");
    expect(groupOf({ ...run, activity: "permission" })).toBe("needs_you");
    expect(groupOf({ ...run, activity: "idle" })).toBe("needs_you");
    expect(groupOf({ ...run, activity: "plan_limit" })).toBe("needs_you");
    const ended: AgentRun = {
      ...run,
      phase: "ended",
      activity: "ended",
      outcome: "finished",
      stop_confirmed: true,
    };
    expect(groupOf({ ...ended, collection: "collecting" })).toBe("ended");
    expect(groupOf({ ...ended, collection: "ready" })).toBe("review");
    expect(groupOf({ ...ended, collection: "no_changes" })).toBe("ended");
    const leftOut = [{ path: "out.log", reason: "ignored by .gitignore" }];
    expect(groupOf({ ...ended, collection: "ready", left_out: leftOut })).toBe(
      "needs_you",
    );
    expect(
      groupOf({
        ...ended,
        collection: "ready",
        left_out: leftOut,
        snapshot_accepted: true,
      }),
    ).toBe("review");
    expect(groupOf({ ...ended, collection: "failed" })).toBe("needs_you");
    expect(groupOf({ ...ended, outcome: "interrupted" })).toBe("needs_you");
    expect(groupOf({ ...ended, outcome: "interrupted", kept: false })).toBe(
      "ended",
    );
  });

  it("says what the agent is doing and what the run produced", () => {
    expect(activityLabel(run)).toBe("Working · turn 2");
    expect(activityTone(run)).toBe("blue");
    expect(activityTone({ ...run, activity: "idle" })).toBe("amber");
    expect(activityTone({ ...run, connected: false })).toBe("dashed");
    expect(activityLabel({ ...run, activity: "idle" })).toBe(
      "Ready for your prompt",
    );
    expect(
      activityLabel({
        ...run,
        phase: "ended",
        outcome: "expired",
        expired_asleep: true,
      }),
    ).toBe("Expired while this Mac slept");
    expect(producedLabel(run)).toBeNull();
    expect(
      producedLabel({ ...run, collection: "ready", changed_files: 1 }),
    ).toBe("Ready to review · 1 file");
    expect(visibilityLabel(run)).toBeNull();
    expect(visibilityLabel({ ...run, cancel_requested: true })).toBe(
      "Cancel requested",
    );
    expect(visibilityLabel({ ...run, connected: false })).toBe(
      "Last reported: working · turn 2",
    );
    const now = new Date("2026-10-05T10:40:00Z").getTime();
    expect(listWhen(run, now)).toMatch(/^40 min · ends /);
    expect(
      listWhen(
        {
          ...run,
          activity: "permission",
          pending_permissions: [
            {
              permission_id: "p",
              turn: 2,
              title: "npm install",
              kind: "execute",
              detail: null,
              asked_at: "2026-10-05T10:36:00Z",
              diffs: [],
            },
          ],
        },
        now,
      ),
    ).toMatch(/^asked 4 min ago · ends /);
    expect(listResult({ ...run, phase: "ended", stop_confirmed: true })).toBe(
      "Work kept · not collected",
    );
    expect(spanLabel(70 * 60_000)).toBe("1 h 10 min");
    expect(
      timeLeft(
        "2026-10-05T11:00:00Z",
        new Date("2026-10-05T10:12:00Z").getTime(),
      ),
    ).toBe("48 min left");
    expect(
      timeLeft(
        "2026-10-05T13:05:00Z",
        new Date("2026-10-05T10:00:00Z").getTime(),
      ),
    ).toBe("3 h 05 min left");
  });

  it("folds events into turns and summarizes a finished one", () => {
    const at = "2026-10-05T10:00:00Z";
    const events: RunEvent[] = [
      { seq: 1, at, type: "accepted", deadline_at: at, permissions: "act" },
      {
        seq: 2,
        at,
        type: "ready",
        session_id: "s",
        agent: "a",
        version: "1",
        model: null,
      },
      { seq: 3, at, type: "prompt", turn: 1, command_id: "c1", text: "go" },
      { seq: 4, at, type: "message", turn: 1, text: "Sure, " },
      {
        seq: 5,
        at,
        type: "tool",
        turn: 1,
        tool_id: "t1",
        title: "cat a.txt",
        kind: "read",
        status: "pending",
        locations: ["a.txt"],
        output: null,
        diffs: [],
      },
      {
        seq: 6,
        at,
        type: "tool",
        turn: 1,
        tool_id: "t1",
        title: null,
        kind: null,
        status: "completed",
        locations: [],
        output: "hi",
        diffs: [],
      },
      {
        seq: 7,
        at,
        type: "tool",
        turn: 1,
        tool_id: "t2",
        title: "npm test",
        kind: "execute",
        status: "completed",
        locations: [],
        output: null,
        diffs: [],
      },
      { seq: 8, at, type: "message", turn: 1, text: "done." },
      {
        seq: 9,
        at,
        type: "turn_ended",
        turn: 1,
        reason: "end_turn",
        message: null,
      },
      { seq: 10, at, type: "prompt", turn: 2, command_id: "c2", text: "more" },
      { seq: 11, at, type: "notice", text: "A request was refused." },
    ];
    const { turns, notices } = foldTurns(events);
    expect(turns.map((t) => t.turn)).toEqual([1, 2]);
    expect(turns[0].message).toBe("Sure, done.");
    expect(turns[0].tools).toHaveLength(2);
    expect(turns[0].tools[0]).toMatchObject({
      title: "cat a.txt",
      kind: "read",
      status: "completed",
      locations: ["a.txt"],
      output: "hi",
    });
    expect(turns[0].ended?.reason).toBe("end_turn");
    expect(turnSummary(turns[0])).toBe("2 steps · read 1 file, ran 1 command");
    expect(turns[1].notices).toEqual(["A request was refused."]);
    expect(notices).toEqual([]);
  });
});

describe("reported edits", () => {
  it("keep the latest diffs an update carried", () => {
    const at = "2026-10-07T10:00:00Z";
    const edit = (seq: number, newText: string | null, status: string) => ({
      seq,
      at,
      type: "tool" as const,
      turn: 1,
      tool_id: "e1",
      title: seq === 1 ? "Edit a.txt" : null,
      kind: seq === 1 ? "edit" : null,
      status,
      locations: [],
      output: null,
      diffs:
        newText === null
          ? []
          : [
              {
                path: "/workspace/a.txt",
                old_text: "one",
                new_text: newText,
                truncated: false,
              },
            ],
    });
    const { turns } = foldTurns([
      edit(1, "two", "pending"),
      edit(2, "TWO", "in_progress"),
      edit(3, null, "completed"),
    ]);
    expect(turns[0].tools[0].status).toBe("completed");
    expect(turns[0].tools[0].diffs.map((d) => d.new_text)).toEqual(["TWO"]);
  });
});

describe("samePreview", () => {
  it("tells a new preview from the same one read again", () => {
    const preview = {
      run_id: "r",
      start_commit: "a".repeat(40),
      commit: "b".repeat(40),
      taken_at: "2026-10-07T10:00:00Z",
      turn: 1,
      files: [],
      left_out: 0,
      busy: false,
      error: null,
    };
    expect(samePreview(preview, { ...preview, files: [] })).toBe(true);
    expect(samePreview(preview, { ...preview, busy: true })).toBe(false);
    expect(
      samePreview(preview, { ...preview, taken_at: "2026-10-07T10:05:00Z" }),
    ).toBe(false);
    expect(samePreview(preview, { ...preview, error: "failed" })).toBe(false);
  });

  it("shows a remote start as steps up to the session", () => {
    const remote = { host_id: "host-1", host_name: "build-01" };
    const states = (starting: AgentRun["starting"]) =>
      startSteps({ ...remote, starting }).map((s) => s.state);
    expect(startSteps({ ...remote, starting: "send" })[1].label).toBe(
      "Send it to build-01",
    );
    expect(states("copy")).toEqual([
      "running",
      "waiting",
      "waiting",
      "waiting",
    ]);
    expect(states("send")).toEqual(["done", "running", "waiting", "waiting"]);
    expect(states("start")).toEqual(["done", "done", "running", "waiting"]);
    // The controller has the run: only its own preparing is left.
    expect(states(null)).toEqual(["done", "done", "done", "running"]);
    // This Mac sends nothing anywhere.
    expect(
      startSteps({
        host_id: "local",
        host_name: "This Mac",
        starting: "start",
      }),
    ).toHaveLength(3);
  });
});
