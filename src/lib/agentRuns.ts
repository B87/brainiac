/**
 * Settings → Agents (SPEC.md, Agent runs — v0.5): how the pane reads a
 * profile (an agent and its model provider), an engine, and an image, what
 * stops a profile's runs on a host, and the request a change saves.
 */

import type { SaveAgentSettingsRequest } from "./generated/SaveAgentSettingsRequest";
import type {
  AgentEngine,
  AgentHost,
  AgentImage,
  AgentKind,
  AgentPayment,
  AgentProfile,
  AgentProvider,
  AgentRun,
  RunEvent,
  RunFileDiff,
  RunPermissionRequest,
  RunPlanEntry,
  RunPreview,
  RunStartStep,
} from "./ipc";

/** New run's time limits offered by default, in minutes (30 minutes to 8 hours). */
export const TIME_LIMITS = [30, 60, 120, 240, 480];

/** "30 minutes", "1 hour", "8 hours". */
export function durationLabel(minutes: number): string {
  if (minutes < 60) return `${minutes} minutes`;
  const hours = minutes / 60;
  if (Number.isInteger(hours)) return hours === 1 ? "1 hour" : `${hours} hours`;
  return `${Math.floor(hours)} h ${minutes % 60} min`;
}

const AGENT_NAME: Record<AgentKind, string> = {
  claude_code: "Claude Code",
  opencode: "OpenCode",
};

const PROVIDER_NAME: Record<AgentProvider, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  openrouter: "OpenRouter",
};

/** "Claude Code", "OpenCode". */
export function agentName(agent: AgentKind): string {
  return AGENT_NAME[agent];
}

/** "Anthropic", "OpenAI", "OpenRouter". */
export function providerName(provider: AgentProvider): string {
  return PROVIDER_NAME[provider];
}

/**
 * A profile's name, as Settings and New run show it: "Claude Code", or
 * "OpenCode · OpenRouter" (Rust `settings::profile_name`).
 */
export function profileName(
  profile: Pick<AgentProfile, "agent" | "provider">,
): string {
  return profile.agent === "claude_code"
    ? AGENT_NAME.claude_code
    : `${AGENT_NAME[profile.agent]} · ${PROVIDER_NAME[profile.provider]}`;
}

/** "OpenRouter API key". */
export function keyLabel(provider: AgentProvider): string {
  return `${PROVIDER_NAME[provider]} API key`;
}

/** Pay with's choices: Claude Code's plan when offered, else the provider's key. */
export function paymentsOffered(
  profile: Pick<AgentProfile, "agent">,
  planOffered: boolean,
): AgentPayment[] {
  return profile.agent === "claude_code" && planOffered
    ? ["claude_plan", "api_key"]
    : ["api_key"];
}

/** "Claude plan" or "API key"; OpenCode names the provider's key. */
export function paymentLabel(
  profile: Pick<AgentProfile, "agent" | "provider">,
  payment: AgentPayment,
): string {
  if (payment === "claude_plan") return "Claude plan";
  return profile.agent === "claude_code"
    ? "API key"
    : keyLabel(profile.provider);
}

/** New run's Agent choice: "Claude Code · API key", "OpenCode · OpenRouter". */
export function profileChoice(profile: AgentProfile): string {
  return profile.agent === "claude_code"
    ? `${profileName(profile)} · ${paymentLabel(profile, profile.payment)}`
    : profileName(profile);
}

/** Where code and prompts go (Rust `settings::destination`). */
export function destinationLabel(
  profile: Pick<AgentProfile, "provider" | "payment">,
): string {
  return profile.payment === "claude_plan"
    ? "Anthropic, under your Claude plan"
    : `${PROVIDER_NAME[profile.provider]}, with an API key`;
}

/** "token" or "API key", for sentences about the credential. */
export function credentialWord(payment: AgentPayment): string {
  return payment === "claude_plan" ? "token" : "API key";
}

/**
 * What stops a run of this profile on this host, in the order to do it:
 * the profile's own setup, the host's, then a passed test of the profile
 * there that still matches (Rust `settings::run_missing`).
 */
export function runMissing(profile: AgentProfile, host: AgentHost): string[] {
  const missing = [...profile.missing, ...host.missing];
  const name = profileName(profile);
  const test = host.tests.find((t) => t.profile_id === profile.id);
  if (!test) missing.push(`Pass a test of ${name} on ${host.name}.`);
  else if (!test.current)
    missing.push(
      `Test ${name} on ${host.name} again: the token or key, the image, or the engine changed since.`,
    );
  return missing;
}

/** A profile's test on a host, as its page shows it. */
export type ProfileTestState =
  | { kind: "passed"; at: string }
  | { kind: "stale"; at: string }
  | { kind: "untested" }
  | { kind: "blocked"; reason: string };

export function profileTestState(
  profile: AgentProfile,
  host: AgentHost,
): ProfileTestState {
  const test = host.tests.find((t) => t.profile_id === profile.id);
  if (test?.current) return { kind: "passed", at: test.passed_at };
  const blocked = profile.missing[0];
  if (blocked) return { kind: "blocked", reason: blocked };
  return test ? { kind: "stale", at: test.passed_at } : { kind: "untested" };
}

/**
 * Models offered as suggestions, per profile: Claude Code's aliases, which
 * follow its releases, and for OpenCode the provider's model names. Any
 * other name is accepted too.
 */
export function modelSuggestions(
  profile: Pick<AgentProfile, "agent" | "provider">,
): string[] {
  if (profile.agent === "claude_code")
    return ["opus", "sonnet", "haiku", "opusplan"];
  switch (profile.provider) {
    case "anthropic":
      return ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-4-5"];
    case "openai":
      return ["gpt-6.1-sol", "gpt-6-astra", "gpt-6-sol"];
    case "openrouter":
      return ["anthropic/claude-sonnet-5-5", "openai/gpt-6.1-sol"];
  }
}

/** Whether the agent has a model of its own when none is given. */
export function modelRequired(agent: AgentKind): boolean {
  return agent === "opencode";
}

/** The model field's placeholder: Claude Code's default, or what OpenCode needs. */
export function modelPlaceholder(agent: AgentKind): string {
  return agent === "claude_code" ? "Claude Code's default" : "Required";
}

/** What the model field takes, for its hint. */
export function modelHint(
  profile: Pick<AgentProfile, "agent" | "provider">,
): string {
  if (profile.agent === "claude_code")
    return "An alias (opus, sonnet, haiku, opusplan) or a full model name; empty for Claude Code's default. The agent reports the model it opened with, so a name the plan or key cannot use shows there.";
  const example = modelSuggestions(profile)[0];
  return `Required: ${PROVIDER_NAME[profile.provider]}'s model name, such as ${example}. The test runs with it, so a name the key cannot use fails there.`;
}

/**
 * A model as typed: trimmed, with only the characters model names use
 * (OpenRouter's carry a slash), up to 96 (Rust `settings::check_model`).
 */
export function parseModel(
  agent: AgentKind,
  text: string,
): { value: string } | { error: string } {
  const model = text.trim();
  if (model.length <= 96 && /^[A-Za-z0-9._:[\]/-]*$/.test(model))
    return { value: model };
  return {
    error:
      agent === "claude_code"
        ? "An alias such as sonnet, or a full model name, with no spaces."
        : "The provider's model name, with no spaces.",
  };
}

/** What a run's model shows: the one the agent reported, else the one asked for. */
export function modelLabel(
  run: Pick<AgentRun, "agent" | "model" | "model_used">,
): string {
  if (run.model_used) return run.model_used;
  return run.model === "" ? `${agentName(run.agent)}'s default` : run.model;
}

/** The pane's request for a profile with some fields changed. */
export function settingsRequest(
  profile: AgentProfile,
  patch: Partial<SaveAgentSettingsRequest> = {},
): SaveAgentSettingsRequest {
  return {
    profile_id: profile.id,
    expected_version: profile.version,
    sends_code_agreed: profile.sends_code_agreed,
    permissions: profile.permissions,
    time_limit_minutes: profile.time_limit_minutes,
    cpus: profile.cpus,
    memory_mib: profile.memory_mib,
    workspace_gib: profile.workspace_gib,
    model: profile.model,
    ...patch,
  };
}

/** "Docker 28.3.2 · API 1.51 · 8 CPUs · 16 GB", or why it cannot be used. */
export function engineSummary(engine: AgentEngine): string {
  if (!engine.reachable) return engine.problem ?? "Not answering.";
  const parts = [
    engine.server_version && `Docker ${engine.server_version}`,
    engine.api_version && `API ${engine.api_version}`,
    engine.cpus != null && `${engine.cpus} CPUs`,
    engine.memory_bytes != null &&
      `${Math.round(engine.memory_bytes / 1024 ** 3)} GB`,
  ].filter(Boolean);
  return parts.join(" · ");
}

/** "brainiac-agents:3f9c41e1a2b0 · sha256:9f3c41e1…" */
export function imageSummary(image: AgentImage): string {
  const id = image.id.replace(/^sha256:/, "");
  return `${image.name} · sha256:${id.slice(0, 12)}…`;
}

/** "5 Oct 2026" from an RFC 3339 time. */
export function dayLabel(rfc3339: string): string {
  const date = new Date(rfc3339);
  if (Number.isNaN(date.getTime())) return rfc3339;
  return date.toLocaleDateString("en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

// --- Runs (SPEC.md, The run) --------------------------------------------------

/** How the Runs list groups runs. */
export type RunGroup = "needs_you" | "review" | "active" | "ended";

export const GROUP_LABEL: Record<RunGroup, string> = {
  needs_you: "Needs you",
  review: "Ready to review",
  active: "Active",
  ended: "Ended",
};

/**
 * Whether the run waits on the user, and otherwise where it belongs: a
 * question, an idle session, or work that waits for a decision needs you.
 */
export function groupOf(run: AgentRun): RunGroup {
  if (run.phase !== "ended") {
    return run.activity === "permission" ||
      run.activity === "idle" ||
      run.activity === "plan_limit"
      ? "needs_you"
      : "active";
  }
  if (run.collection === "failed") return "needs_you";
  if (run.kept && run.collection === "none" && run.stop_confirmed)
    return "needs_you";
  if (leftOutUndecided(run)) return "needs_you";
  if (run.collection === "ready") return "review";
  return "ended";
}

/** Left-out files that were neither added nor accepted while the work is kept. */
export function leftOutUndecided(run: AgentRun): boolean {
  return (
    run.kept &&
    !run.snapshot_accepted &&
    (run.collection === "ready" || run.collection === "no_changes") &&
    run.left_out.length + run.left_out_more > 0
  );
}

export function needsYou(run: AgentRun): boolean {
  return groupOf(run) === "needs_you";
}

const OUTCOME_LABEL = {
  finished: "Finished",
  cancelled: "Cancelled",
  expired: "Expired",
  failed: "Failed",
  interrupted: "Interrupted",
};

/** What the agent is doing, in words (SPEC.md, The run: States). */
export function activityLabel(run: AgentRun): string {
  if (run.phase === "ended" || run.activity === "ended") {
    const word = run.outcome ? OUTCOME_LABEL[run.outcome] : "Ended";
    return run.expired_asleep ? `${word} while this Mac slept` : word;
  }
  switch (run.activity) {
    case "preparing":
      return "Preparing";
    case "working":
      return `Working · turn ${run.turn}`;
    case "permission":
      return "Waiting for you · permission";
    case "idle":
      return "Ready for your prompt";
    case "plan_limit":
      return "Plan limit reached";
    case "stopping":
      return "Stopping";
    default:
      return run.activity;
  }
}

/** What the run produced, or null while nothing was collected yet. */
export function producedLabel(run: AgentRun): string | null {
  switch (run.collection) {
    case "collecting":
      return "Collecting";
    case "ready":
      return `Ready to review · ${run.changed_files ?? 0} ${
        run.changed_files === 1 ? "file" : "files"
      }`;
    case "no_changes":
      return "No changes";
    case "failed":
      return "Collection failed";
    default:
      return null;
  }
}

/** Whether Brainiac can see the run; null when connected. */
export function visibilityLabel(run: AgentRun): string | null {
  if (run.phase === "ended") {
    return run.stop_confirmed ? null : "Stop not confirmed";
  }
  if (run.cancel_requested) return "Cancel requested";
  if (!run.connected) {
    const label = activityLabel(run);
    return `Last reported: ${label.charAt(0).toLowerCase()}${label.slice(1)}`;
  }
  return null;
}

/** A state badge's colour (SPEC.md, The run: States). */
export type RunTone = "blue" | "amber" | "green" | "red" | "grey" | "dashed";

/** The tone of what the agent is doing: working, waiting on you, or ended. */
export function activityTone(run: AgentRun): RunTone {
  if (run.phase !== "ended" && !run.connected) return "dashed";
  if (run.phase === "ended" || run.activity === "ended") {
    return run.outcome === "failed" || run.outcome === "interrupted"
      ? "red"
      : "grey";
  }
  switch (run.activity) {
    case "working":
      return "blue";
    case "permission":
    case "idle":
    case "plan_limit":
      return "amber";
    default:
      return "grey";
  }
}

export function producedTone(run: AgentRun): RunTone {
  if (run.collection === "ready") return "green";
  if (run.collection === "failed") return "red";
  return "grey";
}

/** "This Mac", or the remote host's name. */
export function hostLabel(run: Pick<AgentRun, "host_id" | "host_name">) {
  return isRemote(run) ? run.host_name : "This Mac";
}

export function isRemote(run: Pick<AgentRun, "host_id">): boolean {
  return run.host_id !== "" && run.host_id !== "local";
}

/**
 * A preparing run's start as steps (SPEC.md, The run: Starting): Brainiac
 * copies the start, sends it to a remote host, and hands the run to the
 * controller; then the controller starts the container and the agent.
 */
export function startSteps(
  run: Pick<AgentRun, "host_id" | "host_name" | "starting">,
): { label: string; state: "done" | "running" | "waiting" }[] {
  const remote = isRemote(run);
  const order: RunStartStep[] = remote
    ? ["copy", "send", "start"]
    : ["copy", "start"];
  const labels = [
    "Copy the start commit and its history",
    ...(remote ? [`Send it to ${run.host_name}`] : []),
    "Hand the run to the run controller",
    "Start the container and the agent",
  ];
  const at = run.starting ? order.indexOf(run.starting) : labels.length - 1;
  return labels.map((label, i) => ({
    label,
    state: i < at ? "done" : i === at ? "running" : "waiting",
  }));
}

/** "under a minute", "31 min", "1 h 10 min". */
export function spanLabel(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return "under a minute";
  if (minutes < 60) return `${minutes} min`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

/** "just now", "40 s ago", "6 min ago", "2 h ago", then the clock. */
export function agoLabel(rfc3339: string, now = Date.now()): string {
  const ms = now - new Date(rfc3339).getTime();
  if (Number.isNaN(ms)) return "";
  if (ms < 5_000) return "just now";
  if (ms < 60_000) return `${Math.floor(ms / 1000)} s ago`;
  if (ms < 60 * 60_000) return `${Math.floor(ms / 60_000)} min ago`;
  if (ms < 12 * 60 * 60_000) return `${Math.floor(ms / (60 * 60_000))} h ago`;
  return shortClock(rfc3339, now);
}

/** The Runs list's When: what the time means for the run's group. */
export function listWhen(run: AgentRun, now = Date.now()): string {
  if (run.phase === "ended") {
    return run.ended_at ? `ended ${shortClock(run.ended_at, now)}` : "ended";
  }
  const ends = run.deadline_at
    ? `ends ${shortClock(run.deadline_at, now)}`
    : "";
  const asked = run.pending_permissions[0]?.asked_at;
  const lead =
    run.activity === "permission" && asked
      ? `asked ${agoLabel(asked, now)}`
      : run.accepted_at
        ? spanLabel(now - new Date(run.accepted_at).getTime())
        : "";
  return [lead, ends].filter(Boolean).join(" · ");
}

/** The Runs list's Result: what was collected, or that work waits uncollected. */
export function listResult(run: AgentRun): string | null {
  const produced = producedLabel(run);
  if (produced) return produced;
  if (run.phase === "ended" && run.kept && run.collection === "none")
    return "Work kept · not collected";
  return null;
}

/** "14:32" today, "Mon 14:32" this week, "5 Oct" otherwise. */
export function shortClock(rfc3339: string, now = Date.now()): string {
  const date = new Date(rfc3339);
  if (Number.isNaN(date.getTime())) return rfc3339;
  const clock = date.toLocaleTimeString("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
  });
  const sameDay = new Date(now).toDateString() === date.toDateString();
  if (sameDay) return clock;
  if (now - date.getTime() < 6 * 24 * 60 * 60 * 1000) {
    return `${date.toLocaleDateString("en-GB", { weekday: "short" })} ${clock}`;
  }
  return date.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

/** "48 min left" or "2 h 05 min left"; "Time is up" past the deadline. */
export function timeLeft(deadline: string, now = Date.now()): string {
  const ms = new Date(deadline).getTime() - now;
  if (Number.isNaN(ms)) return "";
  if (ms <= 0) return "Time is up";
  const minutes = Math.ceil(ms / 60_000);
  if (minutes < 60) return `${minutes} min left`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return `${h} h ${String(m).padStart(2, "0")} min left`;
}

/** "Uses your Claude plan", or that the provider reports no cost. */
/** The cost a run's agent last reported, from its journal. */
export function reportedCost(
  events: RunEvent[],
): { micros: number; currency: string } | null {
  for (let i = events.length - 1; i >= 0; i--) {
    const event = events[i];
    if (event.type === "usage")
      return { micros: event.cost_micros, currency: event.currency };
  }
  return null;
}

export function costLabel(
  payment: AgentPayment,
  cost: { micros: number; currency: string } | null = null,
): string {
  if (payment === "claude_plan") return "Uses your Claude plan";
  if (!cost) return "Cost unavailable";
  const amount = cost.micros / 1_000_000;
  const digits = amount < 0.1 ? 3 : 2;
  return cost.currency === "USD"
    ? `$${amount.toFixed(digits)}`
    : `${amount.toFixed(digits)} ${cost.currency}`;
}

export type ToolState = {
  id: string;
  title: string;
  kind: string | null;
  status: string | null;
  locations: string[];
  output: string | null;
  /** The edits it reports; a later update's replace an earlier one's. */
  diffs: RunFileDiff[];
};

export type PermissionState = RunPermissionRequest & {
  outcome: string | null;
  by: string | null;
};

/** One turn of the conversation, folded from its events. */
export type Turn = {
  turn: number;
  prompt: string | null;
  promptAt: string | null;
  message: string;
  thought: string;
  tools: ToolState[];
  plan: RunPlanEntry[] | null;
  permissions: PermissionState[];
  notices: string[];
  ended: { reason: string; message: string | null; at: string } | null;
  /** When the turn's latest event arrived. */
  lastAt: string | null;
};

/** Events of a run as turns, plus the notices before the first prompt. */
export function foldTurns(events: RunEvent[]): {
  turns: Turn[];
  notices: string[];
  ended: { outcome: string; message: string | null } | null;
} {
  const turns = new Map<number, Turn>();
  const notices: string[] = [];
  let ended: { outcome: string; message: string | null } | null = null;
  const turnOf = (n: number): Turn => {
    let t = turns.get(n);
    if (!t) {
      t = {
        turn: n,
        prompt: null,
        promptAt: null,
        message: "",
        thought: "",
        tools: [],
        plan: null,
        permissions: [],
        notices: [],
        ended: null,
        lastAt: null,
      };
      turns.set(n, t);
    }
    return t;
  };
  let current = 0;
  for (const e of events) {
    const b = e;
    if ("turn" in b) turnOf(b.turn).lastAt = e.at;
    switch (b.type) {
      case "prompt": {
        current = b.turn;
        const t = turnOf(b.turn);
        t.prompt = b.text;
        t.promptAt = e.at;
        break;
      }
      case "message":
        turnOf(b.turn).message += b.text;
        break;
      case "thought":
        turnOf(b.turn).thought += b.text;
        break;
      case "plan":
        turnOf(b.turn).plan = b.entries;
        break;
      case "tool": {
        const t = turnOf(b.turn);
        const existing = t.tools.find((x) => x.id === b.tool_id);
        if (existing) {
          if (b.title != null) existing.title = b.title;
          if (b.kind != null) existing.kind = b.kind;
          if (b.status != null) existing.status = b.status;
          if (b.locations.length) existing.locations = b.locations;
          if (b.output != null) existing.output = b.output;
          if (b.diffs.length) existing.diffs = b.diffs;
        } else {
          t.tools.push({
            id: b.tool_id,
            title: b.title ?? "",
            kind: b.kind,
            status: b.status,
            locations: b.locations,
            output: b.output,
            diffs: b.diffs,
          });
        }
        break;
      }
      case "permission":
        turnOf(b.turn).permissions.push({ ...b, outcome: null, by: null });
        break;
      case "permission_answered": {
        for (const t of turns.values()) {
          const p = t.permissions.find(
            (x) => x.permission_id === b.permission_id,
          );
          if (p) {
            p.outcome = b.outcome;
            p.by = b.by;
          }
        }
        break;
      }
      case "turn_ended":
        turnOf(b.turn).ended = {
          reason: b.reason,
          message: b.message,
          at: e.at,
        };
        break;
      case "notice":
        if (current === 0) notices.push(b.text);
        else turnOf(current).notices.push(b.text);
        break;
      case "ended":
        ended = { outcome: b.outcome, message: b.message };
        break;
      default:
        break;
    }
  }
  return {
    turns: [...turns.values()].sort((a, b) => a.turn - b.turn),
    notices,
    ended,
  };
}

/** "14 steps · read 6 files, ran 3 commands" for a folded turn. */
export function turnSummary(t: Turn): string {
  const counts = new Map<string, number>();
  for (const tool of t.tools) {
    const kind = tool.kind ?? "other";
    counts.set(kind, (counts.get(kind) ?? 0) + 1);
  }
  const parts: string[] = [];
  const say = (kind: string, one: string, many: string) => {
    const n = counts.get(kind);
    if (n) parts.push(n === 1 ? one : many.replace("N", String(n)));
  };
  say("read", "read 1 file", "read N files");
  say("edit", "edited 1 file", "edited N files");
  say("execute", "ran 1 command", "ran N commands");
  say("search", "searched once", "searched N times");
  say("fetch", "fetched 1 page", "fetched N pages");
  const steps = t.tools.length;
  const head = `${steps} ${steps === 1 ? "step" : "steps"}`;
  return parts.length ? `${head} · ${parts.join(", ")}` : head;
}

/** A tool row's verb: "Read", "Edited", "Running", by kind and status. */
export function toolVerb(tool: Pick<ToolState, "kind" | "status">): string {
  const running = tool.status === "pending" || tool.status === "in_progress";
  switch (tool.kind) {
    case "read":
      return running ? "Reading" : "Read";
    case "edit":
      return running ? "Editing" : "Edited";
    case "delete":
      return running ? "Deleting" : "Deleted";
    case "move":
      return running ? "Moving" : "Moved";
    case "search":
      return running ? "Searching" : "Searched";
    case "execute":
      return running ? "Running" : "Ran";
    case "fetch":
      return running ? "Fetching" : "Fetched";
    case "think":
      return "Thought";
    default:
      return running ? "Working" : "Tool";
  }
}

/** What a permission request asks for, as its card's heading. */
export function permissionHeading(
  agent: AgentKind,
  kind: string | null,
): string {
  const who = agentName(agent);
  switch (kind) {
    case "execute":
      return `${who} asks to run a command`;
    case "edit":
      return `${who} asks to edit a file`;
    case "delete":
      return `${who} asks to delete a file`;
    case "move":
      return `${who} asks to move a file`;
    case "fetch":
      return `${who} asks to fetch a page`;
    default:
      return `${who} asks for permission`;
  }
}

/**
 * The same Changes so far, read again: the window asks on every change of
 * the run, and an unchanged answer must not redraw the files or the diff.
 */
export function samePreview(a: RunPreview, b: RunPreview): boolean {
  return (
    a.commit === b.commit &&
    a.taken_at === b.taken_at &&
    a.busy === b.busy &&
    a.error === b.error
  );
}
