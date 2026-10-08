import { describe, expect, it } from "vitest";
import {
  clock,
  hostState,
  hostSummary,
  jobOutcome,
  shortFingerprint,
  stepOf,
  stepTime,
} from "./hostJobs";
import type { AgentHost, HostJob, HostJobStep } from "./ipc";

const host: AgentHost = {
  id: "h1",
  kind: "ssh",
  name: "build-01",
  ssh_user: "ci",
  ssh_host: "build-01.lan",
  ssh_port: 22,
  identity_path: null,
  fingerprint: "SHA256:68+fphWeYWHsoVD/OJfo/L7I1SiHfwv0N/Ln4q0s4eQ",
  approved: true,
  installed: true,
  engine_name: "Docker 27.3",
  loop_devices: true,
  image: {
    name: "brainiac-agents:4d1e",
    id: "sha256:4d1e",
    built_at: "2026-10-06T10:00:00Z",
    current: true,
  },
  missing: [],
  tests: [
    {
      profile_id: "claude-code",
      passed_at: "2026-10-06T11:00:00Z",
      current: true,
    },
  ],
  emergency_stop: null,
  state_kept: false,
  controller_build: "a1f3c9e",
  protocol: 2,
  controller_installed_at: "2026-10-04T10:00:00Z",
  upgrade_available: false,
  available_build: "a1f3c9e",
  version: 1,
};

const step = (
  state: HostJobStep["state"],
  started_at: string | null = null,
  ended_at: string | null = null,
): HostJobStep => ({
  title: "A step",
  detail: "",
  state,
  started_at,
  ended_at,
  progress: null,
});

const job = (patch: Partial<HostJob>): HostJob => ({
  id: "j1",
  host_id: "h1",
  host_name: "build-01",
  kind: "upgrade",
  state: "running",
  started_at: "2026-10-07T19:02:00Z",
  ended_at: null,
  steps: [step("done"), step("running"), step("waiting")],
  log_tail: [],
  error: null,
  error_details: null,
  cancellable: true,
  from_build: "a1f3c9e",
  to_build: "7c2e51a",
  profile_ids: [],
  ...patch,
});

describe("host jobs", () => {
  it("counts time as a clock", () => {
    expect(clock(461)).toBe("7:41");
    expect(clock(3725)).toBe("1:02:05");
  });

  it("names the running step out of all of them", () => {
    expect(stepOf(job({}))).toBe("step 2 of 3");
  });

  it("gives a short step seconds and a long one a clock", () => {
    const start = "2026-10-07T19:02:00Z";
    expect(stepTime(step("done", start, "2026-10-07T19:02:03Z"), 0)).toBe(
      "3 s",
    );
    expect(stepTime(step("done", start, "2026-10-07T19:20:12Z"), 0)).toBe(
      "18:12",
    );
    expect(stepTime(step("waiting"), 0)).toBe("");
  });

  it("puts a running job before the host's own state", () => {
    expect(hostState(host, job({}))).toEqual({
      label: "Upgrading · step 2 of 3",
      tone: "busy",
    });
    expect(hostState(host, undefined)).toEqual({
      label: "Ready",
      tone: "ready",
    });
    expect(
      hostState(
        host,
        job({ state: "failed", steps: [step("done"), step("failed")] }),
      ).label,
    ).toBe("Upgrade failed · still on its previous controller");
    expect(hostState({ ...host, approved: false }, undefined).label).toBe(
      "Waiting for confirmation",
    );
    // Ready once one profile's test there still matches.
    const stale = [{ ...host.tests[0], current: false }];
    expect(hostState({ ...host, tests: stale }, undefined).label).toBe(
      "Test needed",
    );
    expect(
      hostState(
        {
          ...host,
          missing: ["This host's Docker engine cannot attach loop devices."],
        },
        undefined,
      ).label,
    ).toBe("Not ready");
    expect(hostSummary(host)).toContain("1 agent tested");
    expect(hostState({ ...host, upgrade_available: true }, undefined)).toEqual({
      label: "Ready · upgrade available",
      tone: "ready",
    });
  });

  it("says how a job ended, and nothing for a cancelled one", () => {
    expect(
      jobOutcome(
        job({
          state: "succeeded",
          ended_at: "2026-10-07T19:21:00Z",
          steps: [step("done")],
        }),
      ),
    ).toEqual({
      title: "build-01 is upgraded",
      body: "Build 7c2e51a, in 19 minutes. Runs can start there again.",
      failed: false,
    });
    expect(
      jobOutcome(
        job({
          state: "failed",
          error: "The copy did not match.",
          steps: [step("done"), step("done"), step("failed"), step("skipped")],
        }),
      )?.title,
    ).toBe("build-01's upgrade failed at step 3 of 4");
    expect(jobOutcome(job({ state: "cancelled" }))).toBeNull();
  });

  it("shortens a fingerprint to its ends", () => {
    expect(shortFingerprint(host.fingerprint ?? "")).toBe(
      "SHA256:68+fphWe…n4q0s4eQ",
    );
  });
});
