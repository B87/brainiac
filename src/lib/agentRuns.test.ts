import { describe, expect, it } from "vitest";
import {
  activityLabel,
  durationLabel,
  engineSummary,
  foldTurns,
  groupOf,
  imageSummary,
  producedLabel,
  settingsRequest,
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
    engine_name: "OrbStack",
    image_name: "brainiac-claude:abc",
    permissions: "ask",
    time_limit_minutes: 60,
    cpus: 4,
    memory_mib: 8192,
    workspace_gib: 20,
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
  };

  it("groups runs by what they wait for", () => {
    expect(groupOf(run)).toBe("active");
    expect(groupOf({ ...run, activity: "permission" })).toBe("needs_you");
    expect(groupOf({ ...run, activity: "plan_limit" })).toBe("needs_you");
    const ended: AgentRun = {
      ...run,
      phase: "ended",
      activity: "ended",
      outcome: "finished",
      stop_confirmed: true,
    };
    expect(groupOf({ ...ended, collection: "collecting" })).toBe("ended");
    expect(groupOf({ ...ended, collection: "ready" })).toBe("needs_you");
    expect(
      groupOf({ ...ended, collection: "ready", snapshot_accepted: true }),
    ).toBe("review");
    expect(groupOf({ ...ended, collection: "failed" })).toBe("needs_you");
    expect(groupOf({ ...ended, outcome: "interrupted" })).toBe("needs_you");
    expect(groupOf({ ...ended, outcome: "interrupted", kept: false })).toBe(
      "ended",
    );
  });

  it("says what the agent is doing and what the run produced", () => {
    expect(activityLabel(run)).toBe("Working, turn 2");
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
    ).toBe("Ready to review, 1 file");
    expect(visibilityLabel(run)).toBeNull();
    expect(visibilityLabel({ ...run, cancel_requested: true })).toBe(
      "Cancel requested",
    );
    expect(
      visibilityLabel(
        { ...run, connected: false },
        new Date("2026-10-05T10:40:00Z").getTime(),
      ),
    ).toMatch(/^Last reported: /);
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
      { seq: 2, at, type: "ready", session_id: "s", agent: "a", version: "1" },
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
