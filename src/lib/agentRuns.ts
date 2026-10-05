/**
 * Settings → Agents (SPEC.md, Agent runs — v0.5): how the pane reads a
 * profile, an engine, and an image, and the request a change saves.
 */

import type { SaveAgentSettingsRequest } from "./generated/SaveAgentSettingsRequest";
import type {
  AgentEngine,
  AgentImage,
  AgentPayment,
  AgentProfile,
  AgentRun,
  RunEvent,
  RunPermissionRequest,
  RunPlanEntry,
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

/**
 * Models offered as suggestions: Claude Code's aliases, which follow its
 * releases; a full model name is accepted too. Empty is its default.
 */
export const MODEL_SUGGESTIONS = ["opus", "sonnet", "haiku", "opusplan"];

/** A model as typed: trimmed; an alias or a name without spaces, up to 64. */
export function parseModel(
  text: string,
): { value: string } | { error: string } {
  const model = text.trim();
  return model.length <= 64 && /^[A-Za-z0-9._:[\]-]*$/.test(model)
    ? { value: model }
    : {
        error: "An alias such as sonnet, or a full model name, with no spaces.",
      };
}

/** What a run's model shows: the one the agent reported, else the one asked for. */
export function modelLabel(
  run: Pick<AgentRun, "model" | "model_used">,
): string {
  if (run.model_used) return run.model_used;
  return run.model === "" ? "Claude Code's default" : run.model;
}

/** The pane's request for a profile with some fields changed. */
export function settingsRequest(
  profile: AgentProfile,
  patch: Partial<SaveAgentSettingsRequest> = {},
): SaveAgentSettingsRequest {
  return {
    expected_version: profile.version,
    engine_socket: profile.engine_socket,
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

/** Where code and prompts go, by payment. */
export function destinationLabel(payment: AgentPayment): string {
  return payment === "claude_plan"
    ? "Anthropic, under your Claude plan"
    : "Anthropic, with an API key";
}

export function paymentLabel(payment: AgentPayment): string {
  return payment === "claude_plan" ? "Claude plan" : "API key";
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

/** "brainiac-claude:3f9c41e1a2b0 · sha256:9f3c41e1…" */
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

/** Whether the run waits on the user, and otherwise where it belongs. */
export function groupOf(run: AgentRun): RunGroup {
  if (run.phase !== "ended") {
    return run.activity === "permission" || run.activity === "plan_limit"
      ? "needs_you"
      : "active";
  }
  const collected =
    run.collection === "ready" || run.collection === "no_changes";
  if (run.collection === "failed") return "needs_you";
  if (run.outcome === "interrupted" && run.kept && run.collection === "none")
    return "needs_you";
  if (collected && !run.snapshot_accepted) return "needs_you";
  if (collected) return "review";
  return "ended";
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
      return `Working, turn ${run.turn}`;
    case "permission":
      return "Waiting for you, permission";
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
      return `Ready to review, ${run.changed_files ?? 0} ${
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
export function visibilityLabel(
  run: AgentRun,
  now = Date.now(),
): string | null {
  if (run.phase === "ended") {
    return run.stop_confirmed ? null : "Stop not confirmed";
  }
  if (run.cancel_requested) return "Cancel requested";
  if (!run.connected) {
    return `Last reported: ${
      run.reported_at ? shortClock(run.reported_at, now) : "never"
    }`;
  }
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
export function costLabel(payment: AgentPayment): string {
  return payment === "claude_plan"
    ? "Uses your Claude plan"
    : "Cost unavailable";
}

export type ToolState = {
  id: string;
  title: string;
  kind: string | null;
  status: string | null;
  locations: string[];
  output: string | null;
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
      };
      turns.set(n, t);
    }
    return t;
  };
  let current = 0;
  for (const e of events) {
    const b = e;
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
        } else {
          t.tools.push({
            id: b.tool_id,
            title: b.title ?? "",
            kind: b.kind,
            status: b.status,
            locations: b.locations,
            output: b.output,
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
