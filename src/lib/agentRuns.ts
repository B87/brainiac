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
