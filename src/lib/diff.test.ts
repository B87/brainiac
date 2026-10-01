import { describe, expect, it } from "vitest";
import {
  hunkAtRow,
  hunkRowIndexes,
  pairLines,
  splitRows,
  tokenize,
  wordDiff,
  wordHighlighter,
} from "./diff";
import type { DiffLine, Hunk } from "./ipc";

const ctx = (n: number, text: string): DiffLine => ({
  kind: "context",
  old_no: n,
  new_no: n,
  text,
});
const del = (n: number, text: string): DiffLine => ({
  kind: "delete",
  old_no: n,
  new_no: null,
  text,
});
const add = (n: number, text: string): DiffLine => ({
  kind: "add",
  old_no: null,
  new_no: n,
  text,
});
const hunk = (lines: DiffLine[]): Hunk => ({
  header: "",
  old_start: 1,
  old_lines: 0,
  new_start: 1,
  new_lines: 0,
  lines,
});

describe("pairLines", () => {
  it("pairs removed with added lines in order and pads the shorter side", () => {
    const pairs = pairLines([
      ctx(1, "a"),
      del(2, "b"),
      del(3, "c"),
      add(2, "B"),
      ctx(4, "d"),
      add(4, "e"),
    ]);
    expect(
      pairs.map((p) => [p.left?.text ?? null, p.right?.text ?? null]),
    ).toEqual([
      ["a", "a"],
      ["b", "B"],
      ["c", null],
      ["d", "d"],
      [null, "e"],
    ]);
  });
});

describe("splitRows and hunk positions", () => {
  it("puts a header row before each hunk", () => {
    const rows = splitRows([
      hunk([ctx(1, "a")]),
      hunk([del(5, "x"), add(5, "y")]),
    ]);
    expect(rows.map((r) => r.kind)).toEqual(["hunk", "pair", "hunk", "pair"]);
    expect(hunkRowIndexes(rows)).toEqual([0, 2]);
    expect(hunkAtRow(rows, 3)).toBe(1);
    expect(hunkAtRow(rows, 1)).toBe(0);
  });
});

describe("wordDiff", () => {
  it("tokenizes words, spaces and punctuation", () => {
    expect(tokenize("let x = f(1);")).toEqual([
      "let",
      " ",
      "x",
      " ",
      "=",
      " ",
      "f",
      "(",
      "1",
      ")",
      ";",
    ]);
  });
  it("marks only the changed words", () => {
    const d = wordDiff("const limit = 10;", "const limit = 25;");
    expect(d?.old).toEqual([
      { text: "const limit = ", changed: false },
      { text: "10", changed: true },
      { text: ";", changed: false },
    ]);
    expect(d?.new.find((s) => s.changed)?.text).toBe("25");
  });
  it("gives up on lines that share almost nothing", () => {
    expect(wordDiff("alpha beta gamma", "one two three four five")).toBeNull();
  });
  it("maps highlights to the paired lines", () => {
    const h = hunk([del(1, "return a + b;"), add(1, "return a - b;")]);
    const words = wordHighlighter([h]);
    expect(words(h.lines[0])?.some((s) => s.changed && s.text === "+")).toBe(
      true,
    );
    expect(words(h.lines[1])?.some((s) => s.changed && s.text === "-")).toBe(
      true,
    );
  });
  it("compares only the rows asked for, so long patches stay fast", () => {
    const line = Array.from({ length: 190 }, (_, i) => `t${i}`).join(" ");
    const lines: DiffLine[] = [];
    for (let i = 0; i < 2500; i++)
      lines.push(del(i, line), add(i, `${line} x`));
    const started = performance.now();
    const words = wordHighlighter([hunk(lines)]);
    for (const l of lines.slice(0, 80)) words(l);
    expect(performance.now() - started).toBeLessThan(500);
  });
});
