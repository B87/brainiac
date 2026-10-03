import { describe, expect, it } from "vitest";
import {
  argsText,
  EDITOR_PRESETS,
  editorPreset,
  hasPathArgument,
  parseArgs,
  parseInRange,
  sectionLabel,
} from "./settings";

describe("editor arguments", () => {
  it("round-trips through the text field", () => {
    expect(argsText(["-g", "{path}:{line}"])).toBe("-g {path}:{line}");
    expect(parseArgs("  -g   {path}:{line} ")).toEqual(["-g", "{path}:{line}"]);
    expect(parseArgs("")).toEqual([]);
  });
});

describe("parseInRange", () => {
  it("accepts whole numbers within the limits", () => {
    expect(parseInRange("10", 10, 60)).toEqual({ value: 10 });
    expect(parseInRange(" 10,000 ", 1000, 20000)).toEqual({ value: 10000 });
    expect(parseInRange("060", 10, 60)).toEqual({ value: 60 });
  });

  it("says why anything else cannot be saved", () => {
    expect(parseInRange("9", 10, 60)).toEqual({ error: "At least 10." });
    expect(parseInRange("999", 1000, 5000)).toEqual({
      error: "At least 1,000.",
    });
    expect(parseInRange("99999999999999999999", 1, 86400)).toEqual({
      error: "At most 86,400.",
    });
    expect(parseInRange("1.5", 1, 9)).toEqual({
      error: "Enter a whole number.",
    });
    expect(parseInRange("", 1, 9)).toEqual({ error: "Enter a whole number." });
  });
});

it("labels sections", () => {
  expect(sectionLabel("agents")).toBe("Agent Access");
});

describe("editorPreset", () => {
  it("recognizes each preset, and Brainiac's default as VS Code", () => {
    for (const p of EDITOR_PRESETS) expect(editorPreset(p.editor)).toBe(p.id);
    expect(
      editorPreset({
        executable: "code",
        repo_args: ["{path}"],
        file_args: ["-g", "{path}:{line}"],
      }),
    ).toBe("vscode");
  });

  it("calls anything else Custom", () => {
    expect(
      editorPreset({
        executable: "/usr/bin/open",
        repo_args: ["-a", "Zed", "{path}"],
        file_args: ["-a", "Zed", "{path}"],
      }),
    ).toBeNull();
  });
});

it("needs the path in some form", () => {
  expect(hasPathArgument(["warp://x?path={path_url}"])).toBe(true);
  expect(hasPathArgument(["-g", "{path}:{line}"])).toBe(true);
  expect(hasPathArgument(["-g"])).toBe(false);
});
