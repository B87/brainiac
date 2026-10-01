import { describe, expect, it } from "vitest";
import { bindingOf, step } from "./keys";

const key = (k: string, mods: Partial<KeyboardEvent> = {}) =>
  ({
    key: k,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    ...mods,
  }) as KeyboardEvent;

describe("bindingOf", () => {
  it("names plain keys, ⌘ combinations, and refuses other modifiers", () => {
    expect(bindingOf(key("J", { shiftKey: true }))).toBe("j");
    expect(bindingOf(key("ArrowDown"))).toBe("ArrowDown");
    expect(bindingOf(key("1", { metaKey: true }))).toBe("mod+1");
    expect(bindingOf(key("1", { metaKey: true, shiftKey: true }))).toBeNull();
    expect(bindingOf(key("j", { ctrlKey: true }))).toBeNull();
  });
});

describe("step", () => {
  it("moves within bounds and starts at an end", () => {
    const items = ["a", "b", "c"];
    expect(step(items, "b", 1)).toBe("c");
    expect(step(items, "c", 1)).toBe("c");
    expect(step(items, null, 1)).toBe("a");
    expect(step(items, null, -1)).toBe("c");
    expect(step([], null, 1)).toBeNull();
  });
});
