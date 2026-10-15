import { describe, expect, it } from "vitest";
import type { HostHealth } from "./generated/HostHealth";
import {
  freeLabel,
  healthSummary,
  healthTone,
  loadLabel,
  sizeLabel,
  uptimeLabel,
} from "./hostHealth";

const health = (over: Partial<HostHealth> = {}): HostHealth => ({
  reachable: true,
  problem: null,
  checked_at: "2026-10-15T10:00:00Z",
  controller_version: "0.6.4",
  controller_uptime_secs: 60,
  live_runs: 0,
  host_uptime_secs: 100,
  load: [0.1, 0.2, 0.3],
  memory_total_kib: 1024,
  memory_available_kib: 512,
  disk_total_kib: 1024,
  disk_free_kib: 512,
  engine_name: "Docker",
  engine_supported: true,
  engine_problem: null,
  warnings: [],
  ...over,
});

describe("host health labels", () => {
  it("shortens durations to two units", () => {
    expect(uptimeLabel(40)).toBe("40 s");
    expect(uptimeLabel(7 * 60 + 5)).toBe("7 min");
    expect(uptimeLabel(2 * 3600 + 5 * 60)).toBe("2 h 5 min");
    expect(uptimeLabel(3 * 86400 + 4 * 3600 + 59)).toBe("3 d 4 h");
    expect(uptimeLabel(null)).toBe("—");
  });

  it("writes sizes and shares", () => {
    expect(sizeLabel(512 * 1024)).toBe("512 MiB");
    expect(sizeLabel(3.4 * 1024 * 1024)).toBe("3.4 GiB");
    expect(sizeLabel(undefined)).toBe("—");
    expect(freeLabel(512, 1024)).toBe("1 MiB free of 1 MiB (50%)");
    // Rounded down, so 9.6% free never reads as the 10% a warning is below.
    expect(freeLabel(96, 1000)).toBe("0 MiB free of 1 MiB (9%)");
    expect(freeLabel(1, 0)).toBe("—");
    expect(freeLabel(null, 5)).toBe("—");
  });

  it("writes load, or a dash", () => {
    expect(loadLabel([0.1, 0.2, 0.3])).toBe("0.10 · 0.20 · 0.30");
    expect(loadLabel(null)).toBe("—");
    expect(loadLabel([1])).toBe("—");
  });

  it("calls a host down, needing attention, or fine", () => {
    expect(healthTone(health())).toBe("ok");
    expect(healthTone(health({ warnings: ["The disk is almost full"] }))).toBe(
      "warn",
    );
    expect(healthTone(health({ reachable: false }))).toBe("bad");
    expect(healthSummary(health())).toMatch(/nothing needs attention/);
    expect(healthSummary(health({ warnings: ["a", "b"] }))).toBe(
      "2 things need attention",
    );
    expect(
      healthSummary(health({ reachable: false, problem: "It is off." })),
    ).toBe("It is off.");
  });
});
