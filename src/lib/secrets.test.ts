import { describe, expect, it } from "vitest";
import type { SecretEntry } from "./ipc";
import {
  commandPreview,
  draftOf,
  sameSource,
  sourceLabel,
  sourceOf,
  stateLabel,
  validVariable,
} from "./secrets";

const entry = (over: Partial<SecretEntry> = {}): SecretEntry => ({
  owner: { kind: "db_connection", id: "c1" },
  label: "Billing",
  destination: "db.example.com:5432/app as app",
  source: { kind: "store" },
  state: { needs_approval: false, pending: null, revision: 1 },
  input_required: false,
  last_test: null,
  ...over,
});

describe("source drafts", () => {
  it("keep every kind's fields and build only the chosen one", () => {
    const draft = draftOf(
      {
        kind: "command",
        program: "/opt/homebrew/bin/op",
        args: ["read", "op://Work/db"],
      },
      "store",
    );
    expect(draft.kind).toBe("command");
    expect(sourceOf({ ...draft, kind: "store" })).toEqual({ kind: "store" });
    expect(
      sourceOf({ ...draft, kind: "environment", name: " TOKEN " }),
    ).toEqual({
      kind: "environment",
      name: "TOKEN",
    });
    expect(sourceOf(draft)).toEqual({
      kind: "command",
      program: "/opt/homebrew/bin/op",
      args: ["read", "op://Work/db"],
    });
    expect(draftOf(null, "ask").kind).toBe("ask");
    expect(sameSource({ kind: "store" }, { kind: "store" })).toBe(true);
    expect(sameSource({ kind: "store" }, { kind: "ask" })).toBe(false);
  });

  it("preview the exact argument array, spaces and quotes included", () => {
    expect(commandPreview("/usr/bin/gh", ["auth", "token", "a b", 'q"'])).toBe(
      '["/usr/bin/gh","auth","token","a b","q\\""]',
    );
  });

  it("describe sources and states without values", () => {
    expect(
      sourceLabel({
        kind: "command",
        program: "/opt/homebrew/bin/gh",
        args: ["auth", "token"],
      }),
    ).toBe("Command gh auth token");
    expect(sourceLabel({ kind: "environment", name: "PGPASSWORD" })).toBe(
      "Environment variable PGPASSWORD",
    );
    expect(stateLabel(entry())).toBe("Not tested.");
    expect(
      stateLabel(
        entry({ state: { needs_approval: true, pending: null, revision: 1 } }),
      ),
    ).toMatch(/not read until you allow it/);
    expect(
      stateLabel(
        entry({
          state: { needs_approval: false, pending: "cleanup", revision: 1 },
        }),
      ),
    ).toMatch(/new source is in use/);
    const now = Date.parse("2026-10-04T10:05:00Z");
    expect(
      stateLabel(
        entry({
          last_test: {
            at: "2026-10-04T10:00:00Z",
            ok: true,
            message: "Connected: PostgreSQL 16.4.",
          },
        }),
        now,
      ),
    ).toBe("Tested 5 min ago: Connected: PostgreSQL 16.4.");
    expect(validVariable("GITHUB_TOKEN")).toBe(true);
    expect(validVariable("1TOKEN")).toBe(false);
  });
});
