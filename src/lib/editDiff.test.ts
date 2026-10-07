import { describe, expect, it } from "vitest";
import { editCounts, editLines, workspacePath } from "./editDiff";

describe("editLines", () => {
  it("shows a replaced line between unchanged ones", () => {
    const rows = editLines("a\nb\nc\n", "a\nB\nc\n");
    expect(rows).toEqual([
      { kind: "context", text: "a" },
      { kind: "del", text: "b" },
      { kind: "add", text: "B" },
      { kind: "context", text: "c" },
    ]);
    expect(editCounts(rows)).toEqual({ added: 1, removed: 1 });
  });

  it("treats a new file as all added", () => {
    expect(editLines(null, "one\ntwo")).toEqual([
      { kind: "add", text: "one" },
      { kind: "add", text: "two" },
    ]);
  });

  it("keeps lines common to both in the middle", () => {
    const rows = editLines("x\nkeep\ny", "keep\nz");
    expect(rows.filter((r) => r.kind === "context")).toEqual([
      { kind: "context", text: "keep" },
    ]);
    expect(editCounts(rows)).toEqual({ added: 1, removed: 2 });
  });

  it("gives up on pairing very long texts but keeps every line", () => {
    const a = Array.from({ length: 1500 }, (_, i) => `a${i}`).join("\n");
    const b = Array.from({ length: 1500 }, (_, i) => `b${i}`).join("\n");
    const rows = editLines(a, b);
    expect(editCounts(rows)).toEqual({ added: 1500, removed: 1500 });
  });
});

describe("workspacePath", () => {
  it("names a file as the repository does", () => {
    expect(workspacePath("/workspace/src/a.ts")).toBe("src/a.ts");
    expect(workspacePath("/tmp/x")).toBe("/tmp/x");
  });
});
