import { describe, expect, it } from "vitest";
import type { Check } from "./ipc";
import {
  changeTarget,
  fileTarget,
  findFiles,
  nextUnreviewed,
  orderChecks,
  resolveLanding,
  stepPast,
} from "./reviewNav";

const files = [
  { path: "a.ts", binary: false },
  { path: "b.png", binary: true },
  { path: "c.ts", binary: false },
  { path: "d.png", binary: true },
];

describe("changeTarget", () => {
  it("moves inside a file while it has more hunks", () => {
    expect(changeTarget(files, "a.ts", 0, 3, 1)).toEqual({
      path: "a.ts",
      hunk: 1,
    });
    expect(changeTarget(files, "c.ts", 2, 3, -1)).toEqual({
      path: "c.ts",
      hunk: 1,
    });
  });

  it("crosses to the next file's first hunk and the previous file's last", () => {
    expect(changeTarget(files, "a.ts", 1, 2, 1)).toEqual({
      path: "c.ts",
      hunk: "first",
    });
    expect(changeTarget(files, "c.ts", 0, 2, -1)).toEqual({
      path: "a.ts",
      hunk: "last",
    });
  });

  it("stops at the ends without wrapping, skipping files with no text", () => {
    expect(changeTarget(files, "c.ts", 0, 1, 1)).toBeNull();
    expect(changeTarget(files, "a.ts", 0, 1, -1)).toBeNull();
  });

  it("steps past a file that turned out to have no hunks", () => {
    expect(stepPast(files, "c.ts", -1)).toEqual({ path: "a.ts", hunk: "last" });
    expect(stepPast(files, "missing", 1)).toBeNull();
  });
});

describe("fileTarget", () => {
  it("is null at the ends of the active order", () => {
    expect(fileTarget(files, "a.ts", -1)).toBeNull();
    expect(fileTarget(files, "d.png", 1)).toBeNull();
    expect(fileTarget(files, "a.ts", 1)).toBe("b.png");
  });
});

describe("resolveLanding", () => {
  it("resolves first, last, and a number, and has nothing for no hunks", () => {
    expect(resolveLanding("first", 3)).toBe(0);
    expect(resolveLanding("last", 3)).toBe(2);
    expect(resolveLanding(5, 3)).toBe(2);
    expect(resolveLanding("first", 0)).toBeNull();
  });
});

describe("nextUnreviewed", () => {
  const reviewed = new Set(["a.ts", "c.ts"]);
  it("finds the next unreviewed file in the order, then wraps once", () => {
    expect(nextUnreviewed(files, "a.ts", (p) => reviewed.has(p))).toBe("b.png");
    expect(nextUnreviewed(files, "d.png", (p) => reviewed.has(p))).toBe(
      "b.png",
    );
  });
  it("is null when every other file is reviewed", () => {
    expect(nextUnreviewed(files, "a.ts", () => true)).toBeNull();
  });
});

describe("findFiles", () => {
  it("matches every word of the query anywhere in the path", () => {
    const list = [{ path: "src/a/parse.ts" }, { path: "docs/parse.md" }];
    expect(findFiles(list, "parse")).toHaveLength(2);
    expect(findFiles(list, "src parse")).toEqual([{ path: "src/a/parse.ts" }]);
    expect(findFiles(list, "")).toHaveLength(2);
  });
});

describe("orderChecks", () => {
  const check = (name: string, state: Check["state"]): Check => ({
    name,
    state,
    description: null,
    url: null,
  });
  it("lists failures first, then pending, passed, and skipped, by name", () => {
    const ordered = orderChecks([
      check("build", "success"),
      check("zeta", "failure"),
      check("lint", "neutral"),
      check("deploy", "pending"),
      check("alpha", "failure"),
    ]);
    expect(ordered.map((c) => c.name)).toEqual([
      "alpha",
      "zeta",
      "deploy",
      "build",
      "lint",
    ]);
  });
});
