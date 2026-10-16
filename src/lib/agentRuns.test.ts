import { describe, expect, it } from "vitest";
import {
  activityLabel,
  activityTone,
  destinationLabel,
  durationLabel,
  engineSummary,
  explainedLabel,
  foldTurns,
  groupOf,
  imageSummary,
  listResult,
  listWhen,
  modelLabel,
  modelSuggestions,
  needsYou,
  parseModel,
  paymentLabel,
  paymentsOffered,
  permissionHeading,
  producedLabel,
  profileChoice,
  profileName,
  profileTestState,
  resultTone,
  runMissing,
  runState,
  runTitle,
  samePreview,
  settingsRequest,
  spanLabel,
  startSteps,
  timeLeft,
  turnSummary,
  visibilityLabel,
} from "./agentRuns";
import type {
  AgentEngine,
  AgentHost,
  AgentProfile,
  AgentRun,
  RunEvent,
  RunExplanation,
} from "./ipc";

const profile: AgentProfile = {
  id: "claude-code",
  agent: "claude_code",
  provider: "anthropic",
  payment: "api_key",
  credential_source: { kind: "none" },
  credential: { needs_approval: false, pending: null, revision: 1 },
  credential_saved_at: null,
  credential_ageing: false,
  permissions: "ask",
  time_limit_minutes: 60,
  cpus: 4,
  memory_mib: 8192,
  workspace_gib: 20,
  model: "",
  tools: false,
  missing: [],
  version: 3,
};

const openrouter: AgentProfile = {
  ...profile,
  id: "opencode-openrouter",
  agent: "opencode",
  provider: "openrouter",
  model: "anthropic/claude-sonnet-5-5",
};

const thisMac: AgentHost = {
  id: "local",
  kind: "local",
  name: "This Mac",
  ssh_user: null,
  ssh_host: null,
  ssh_port: null,
  identity_path: null,
  fingerprint: null,
  approved: true,
  installed: true,
  engine_name: "OrbStack",
  loop_devices: false,
  image: null,
  tools_image: null,
  missing: [],
  tests: [],
  emergency_stop: null,
  state_kept: false,
  controller_build: null,
  protocol: null,
  controller_installed_at: null,
  upgrade_available: false,
  available_build: null,
  version: 1,
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
    expect(request.profile_id).toBe("claude-code");
    expect(request.expected_version).toBe(3);
    expect(request.permissions).toBe("act");
    expect(request.time_limit_minutes).toBe(60);
  });

  it("names a profile, where it sends code, and how it is paid", () => {
    expect(profileName(profile)).toBe("Claude Code");
    expect(profileName(openrouter)).toBe("OpenCode · OpenRouter");
    expect(destinationLabel({ ...profile, payment: "claude_plan" })).toBe(
      "Anthropic, under your Claude plan",
    );
    expect(destinationLabel(openrouter)).toBe("OpenRouter, with an API key");
    expect(paymentLabel(profile, "api_key")).toBe("API key");
    expect(paymentLabel(openrouter, "api_key")).toBe("OpenRouter API key");
    expect(paymentsOffered(profile, true)).toEqual(["claude_plan", "api_key"]);
    expect(paymentsOffered(profile, false)).toEqual(["api_key"]);
    expect(paymentsOffered(openrouter, true)).toEqual(["api_key"]);
    expect(profileChoice(profile)).toBe("Claude Code · API key");
    expect(profileChoice(openrouter)).toBe("OpenCode · OpenRouter");
  });

  it("says what stops a profile's run on a host, the profile first", () => {
    const lacking = { ...openrouter, missing: ["Add an OpenRouter API key."] };
    const host = { ...thisMac, missing: ["Build the image."] };
    expect(runMissing(lacking, host)).toEqual([
      "Add an OpenRouter API key.",
      "Build the image.",
      "Pass a test of OpenCode · OpenRouter on This Mac.",
    ]);
    const tested = (current: boolean): AgentHost => ({
      ...thisMac,
      tests: [
        {
          profile_id: openrouter.id,
          passed_at: "2026-10-05T10:00:00Z",
          current,
        },
      ],
    });
    expect(runMissing(openrouter, tested(true))).toEqual([]);
    expect(runMissing(openrouter, tested(false))).toEqual([
      "Test OpenCode · OpenRouter on This Mac again: the token or key, the image, or the engine changed since.",
    ]);
    // Another profile's test does not count.
    expect(runMissing(profile, tested(true))).toHaveLength(1);
  });

  it("asks for the tools layer on a host that lacks it or has it out of date", () => {
    const wants = { ...openrouter, tools: true };
    const host = (tools_image: AgentHost["tools_image"]): AgentHost => ({
      ...thisMac,
      tools_image,
      tests: [
        {
          profile_id: wants.id,
          passed_at: "2026-10-05T10:00:00Z",
          current: true,
        },
      ],
    });
    expect(runMissing(wants, host(null))).toEqual([
      "Build the image on This Mac to add Python and the Google Cloud CLI.",
    ]);
    const layer = {
      name: "brainiac-agents:aaaaaaaaaaaa",
      id: "sha256:a",
      built_at: "2026-10-05T10:00:00Z",
    };
    expect(runMissing(wants, host({ ...layer, current: false }))).toEqual([
      "Rebuild the image on This Mac: the Python and Google Cloud CLI layer is out of date.",
    ]);
    expect(runMissing(wants, host({ ...layer, current: true }))).toEqual([]);
    // A profile that does not ask for it is not held up by it.
    expect(runMissing(openrouter, host(null))).toEqual([]);
  });

  it("gives each profile's test on a host one state", () => {
    expect(profileTestState(openrouter, thisMac)).toEqual({ kind: "untested" });
    expect(
      profileTestState({ ...openrouter, missing: ["Add a key."] }, thisMac),
    ).toEqual({ kind: "blocked", reason: "Add a key." });
    const at = "2026-10-05T10:00:00Z";
    const host = (current: boolean): AgentHost => ({
      ...thisMac,
      tests: [{ profile_id: openrouter.id, passed_at: at, current }],
    });
    expect(profileTestState(openrouter, host(true))).toEqual({
      kind: "passed",
      at,
    });
    expect(profileTestState(openrouter, host(false))).toEqual({
      kind: "stale",
      at,
    });
  });

  it("suggests and checks models per agent", () => {
    expect(modelSuggestions(profile)).toContain("opusplan");
    expect(modelSuggestions({ ...openrouter, provider: "openai" })).toContain(
      "gpt-6.1-sol",
    );
    expect(modelSuggestions(openrouter)).toContain(
      "anthropic/claude-sonnet-5-5",
    );
    expect(parseModel("opencode", " anthropic/claude-sonnet-5-5 ")).toEqual({
      value: "anthropic/claude-sonnet-5-5",
    });
    expect(parseModel("claude_code", "opus[1m]")).toEqual({
      value: "opus[1m]",
    });
    expect("error" in parseModel("opencode", "gpt 6")).toBe(true);
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
        name: "brainiac-agents:3f9c41e1a2b0",
        id: "sha256:9f3c41e1aaaaaaaaaaaaaaaa",
        built_at: "2026-10-05T12:00:00Z",
        current: true,
      }),
    ).toBe("brainiac-agents:3f9c41e1a2b0 · sha256:9f3c41e1aaaa…");
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
    profile_id: "claude-code",
    agent: "claude_code",
    provider: "anthropic",
    payment: "api_key",
    credential_source: "the Keychain",
    host_id: "local",
    host_name: "This Mac",
    engine_name: "OrbStack",
    image_name: "brainiac-agents:abc",
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
    explain: false,
    host_secrets: [],
    cursor: 12,
    created_at: "2026-10-05T10:00:00Z",
    updated_at: "2026-10-05T10:30:00Z",
    version: 4,
    starting: null,
  };

  it("names the run's agent in its model and its questions", () => {
    expect(modelLabel(run)).toBe("Claude Code's default");
    expect(
      modelLabel({ ...run, agent: "opencode", model: "gpt-6.1-sol" }),
    ).toBe("gpt-6.1-sol");
    expect(modelLabel({ ...run, model_used: "claude-sonnet-5-5" })).toBe(
      "claude-sonnet-5-5",
    );
    expect(permissionHeading("opencode", "execute")).toBe(
      "OpenCode asks to run a command",
    );
    expect(permissionHeading("claude_code", null)).toBe(
      "Claude Code asks for permission",
    );
  });

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

  it("never asks the user about an explain run", () => {
    const explain: AgentRun = { ...run, explain: true };
    expect(groupOf({ ...explain, activity: "idle" })).toBe("active");
    const ended: AgentRun = {
      ...explain,
      phase: "ended",
      activity: "ended",
      outcome: "interrupted",
      stop_confirmed: true,
      kept: true,
    };
    expect(groupOf(ended)).toBe("ended");
    expect(needsYou(ended)).toBe(false);
    expect(listResult(ended)).toBeNull();
  });

  it("shows an explain run's explanation as its outcome and result", () => {
    // Brainiac stops an explain run with Cancel when its explanation failed.
    const ended: AgentRun = {
      ...run,
      explain: true,
      phase: "ended",
      activity: "ended",
      outcome: "cancelled",
      stop_confirmed: true,
      kept: false,
    };
    const link: RunExplanation = {
      run_id: ended.id,
      id: "e1",
      repository_id: ended.repository_id,
      subject: { kind: "commit", reference: "a".repeat(40) },
      depth: "teach_me",
      state: "ready",
      error: null,
    };
    expect(runState(ended, link)).toEqual({ label: "Finished", tone: "grey" });
    expect(listResult(ended, link)).toBe("Explanation ready");
    expect(resultTone(ended, link)).toBe("green");
    const failed: RunExplanation = { ...link, state: "failed", error: "x" };
    expect(runState(ended, failed)).toEqual({ label: "Failed", tone: "red" });
    expect(listResult(ended, failed)).toBe("Explanation failed");
    expect(resultTone(ended, failed)).toBe("red");
    const cancelled: RunExplanation = { ...link, state: "cancelled" };
    expect(runState(ended, cancelled).label).toBe("Cancelled");
    expect(listResult(ended, cancelled)).toBeNull();
    // Without its explanation, the run's own state; ordinary runs as before.
    expect(runState(ended, null).label).toBe("Cancelled");
    const finished: AgentRun = {
      ...ended,
      explain: false,
      outcome: "finished",
    };
    expect(runState(finished, null)).toEqual({
      label: "Finished",
      tone: "grey",
    });
    expect(resultTone({ ...finished, collection: "ready" })).toBe("green");
  });

  it("titles an explain run by its subject and depth, not the verb", () => {
    const explain: AgentRun = {
      ...run,
      explain: true,
      title: "Explain Fix it",
    };
    expect(runTitle(explain)).toBe("Fix it");
    expect(runTitle({ ...explain, explain: false })).toBe("Explain Fix it");
    const link: RunExplanation = {
      run_id: run.id,
      id: "e1",
      repository_id: run.repository_id,
      subject: { kind: "commit", reference: "a1b2c3d4".repeat(5) },
      depth: "teach_me",
      state: "ready",
      error: null,
    };
    expect(explainedLabel(link)).toBe("Commit a1b2c3d · Teach me");
    expect(
      explainedLabel({
        ...link,
        subject: { kind: "branch", reference: "refs/heads/feature/x" },
        depth: "brief",
      }),
    ).toBe("Branch feature/x · Brief");
    expect(
      explainedLabel({
        ...link,
        subject: { kind: "pull_request", reference: "acme/app#42" },
        depth: "deep",
      }),
    ).toBe("Pull request #42 · Deep");
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
