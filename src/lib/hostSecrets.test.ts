import { describe, expect, it } from "vitest";
import {
  removable,
  shareLabel,
  stateHelp,
  stateText,
  stateTone,
} from "./hostSecrets";
import type { HostSecretRow, HostSecretState } from "./ipc";

const row = (
  state: HostSecretState,
  failure: string | null = null,
): HostSecretRow => ({
  secret_id: "s1",
  label: "Sentry",
  env_name: "SENTRY_TOKEN",
  state,
  shared_at: null,
  failure,
});

describe("secrets on a host", () => {
  it("names each state and how bad it is", () => {
    expect(stateText("shared")).toBe("On the host");
    expect(stateTone("shared")).toBe("ok");
    expect(stateTone("out_of_date")).toBe("warn");
    expect(stateTone("missing_on_host")).toBe("warn");
    expect(stateTone("failed")).toBe("bad");
    expect(stateTone("not_shared")).toBe("idle");
  });

  it("says what to do, and shows the reason a share failed", () => {
    expect(stateHelp(row("shared"))).toContain("SENTRY_TOKEN");
    expect(stateHelp(row("out_of_date"))).toContain("Share it again");
    expect(stateHelp(row("failed", "The host did not answer."))).toBe(
      "The host did not answer.",
    );
    expect(stateHelp(row("failed"))).toContain("did not finish");
  });

  it("offers Share, Retry or Share Again, and Remove only when a copy may exist", () => {
    expect(shareLabel("not_shared")).toBe("Share");
    expect(shareLabel("failed")).toBe("Retry");
    expect(shareLabel("out_of_date")).toBe("Share Again");
    expect(shareLabel("shared")).toBe("Share Again");
    expect(removable("not_shared")).toBe(false);
    expect(removable("failed")).toBe(true);
    expect(removable("shared")).toBe(true);
  });
});
