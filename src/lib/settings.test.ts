import { describe, expect, it } from "vitest";
import { argsText, parseArgs, parseInRange, sectionLabel } from "./settings";

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
