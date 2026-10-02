import { describe, expect, it } from "vitest";
import {
  editedLabel,
  insideVault,
  normalizePath,
  vaultImageUrl,
} from "./notes";
import { diffLines } from "./textDiff";

describe("vault paths", () => {
  it("draws vault images through the vault scheme and never web images", () => {
    expect(vaultImageUrl("Projects/Plan.md", "img/a b.png")).toBe(
      "vault://localhost/Projects/img/a%20b.png",
    );
    expect(vaultImageUrl("Projects/Plan.md", "../img/x%20y.png")).toBe(
      "vault://localhost/img/x%20y.png",
    );
    expect(vaultImageUrl("Plan.md", "/top.png")).toBe(
      "vault://localhost/top.png",
    );
    expect(vaultImageUrl("Plan.md", "https://example.com/a.png")).toBeNull();
    expect(vaultImageUrl("Plan.md", "//cdn.example.com/a.png")).toBeNull();
    expect(vaultImageUrl("Plan.md", "../../outside.png")).toBeNull();
  });

  it("normalizes and keeps paths inside the vault", () => {
    expect(normalizePath("a/./b/../c.md")).toBe("a/c.md");
    expect(normalizePath("../x")).toBeNull();
    expect(insideVault("/v/Notes", "/v/Notes/a/b.md")).toBe("a/b.md");
    expect(insideVault("/v/Notes", "/v/Other/b.md")).toBeNull();
  });

  it("reads edit times as relative for a week, then as dates", () => {
    const now = Date.parse("2026-10-02T12:00:00Z");
    expect(editedLabel("2026-10-02T10:00:00Z", now)).toBe("edited 2 hours ago");
    expect(editedLabel("2026-09-14T10:00:00Z", now)).toBe("edited 14 Sep");
  });
});

describe("diffLines", () => {
  it("shows changed lines between shared ones", () => {
    expect(diffLines("a\nb\nc", "a\nB\nc\nd")).toEqual([
      { kind: "same", text: "a" },
      { kind: "added", text: "B" },
      { kind: "removed", text: "b" },
      { kind: "same", text: "c" },
      { kind: "added", text: "d" },
    ]);
  });
});
