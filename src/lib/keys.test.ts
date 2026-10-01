import { describe, expect, it } from "vitest";
import { bindingOf, pickHandler, shortcutAllowed, step } from "./keys";

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

describe("shortcut routing", () => {
  const ctx = { modalOpen: false, menuOpen: false, arrowsOwned: false };
  /** A fake element: `inside` lists the selectors `closest` finds. */
  const el = (tagName: string, inside: string[] = []) => ({
    tagName,
    isContentEditable: false,
    closest: (selector: string) =>
      inside.some((s) => selector.includes(s)) ? {} : null,
  });

  it("leaves typing, Enter on buttons, and arrows in a patch alone", () => {
    expect(shortcutAllowed("j", el("INPUT"), ctx)).toBe(false);
    expect(shortcutAllowed("Enter", el("BUTTON", ["button"]), ctx)).toBe(false);
    expect(shortcutAllowed("Enter", el("DIV"), ctx)).toBe(true);
    expect(
      shortcutAllowed("ArrowDown", el("SPAN", ["[data-own-arrows]"]), ctx),
    ).toBe(false);
    expect(
      shortcutAllowed("ArrowDown", el("BODY"), { ...ctx, arrowsOwned: true }),
    ).toBe(false);
    expect(shortcutAllowed("j", el("BODY"), ctx)).toBe(true);
  });

  it("gives modals everything and open menus the plain keys", () => {
    expect(shortcutAllowed("mod+1", null, { ...ctx, modalOpen: true })).toBe(
      false,
    );
    expect(shortcutAllowed("j", null, { ...ctx, menuOpen: true })).toBe(false);
    expect(shortcutAllowed("mod+1", null, { ...ctx, menuOpen: true })).toBe(
      true,
    );
    // ⌘ shortcuts work even from a text field.
    expect(shortcutAllowed("mod+2", el("INPUT"), ctx)).toBe(true);
  });

  it("lets the first registration that binds a key handle it", () => {
    const calls: string[] = [];
    const first = { j: () => calls.push("first") };
    const second = { j: () => calls.push("second"), k: () => calls.push("k") };
    pickHandler("j", [first, second])?.({} as KeyboardEvent);
    pickHandler("k", [first, second])?.({} as KeyboardEvent);
    expect(calls).toEqual(["first", "k"]);
    expect(pickHandler("x", [first, second])).toBeNull();
  });
});
