import { describe, expect, it } from "vitest";
import {
  durationLabel,
  engineSummary,
  imageSummary,
  settingsRequest,
} from "./agentRuns";
import type { AgentEngine, AgentProfile } from "./ipc";

const profile: AgentProfile = {
  id: "claude-code",
  engine_socket: null,
  payment: "api_key",
  credential_source: { kind: "none" },
  credential: { needs_approval: false, pending: null, revision: 1 },
  credential_saved_at: null,
  credential_ageing: false,
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
