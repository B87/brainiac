import { describe, expect, it } from "vitest";
import { type ReviewFooterState, reviewHints } from "./footer";

const base: ReviewFooterState = {
  tab: "files",
  zone: "diff",
  hasNextChange: true,
  hasPrevChange: true,
  hasNextFile: true,
  hasPrevFile: true,
  canComment: true,
  commentWhy: "Select a line first",
  onHelp: () => {},
};

const labels = (s: Partial<ReviewFooterState>) =>
  reviewHints({ ...base, ...s }).map((h) => `${h.keys} ${h.label}`.trim());

describe("reviewHints", () => {
  it("shows the diff's keys in Files Changed, ending with the way to all of them", () => {
    const hints = reviewHints(base);
    expect(hints.map((h) => h.keys)).toEqual([
      "J",
      "K",
      "⇧J/K",
      "⌘P",
      "C",
      "T",
      "?",
    ]);
    expect(hints.at(-1)).toMatchObject({ essential: true });
    expect(hints.at(-1)?.onActivate).toBeTypeOf("function");
  });

  it("dims what cannot act and says why, instead of hiding it", () => {
    const hints = reviewHints({
      ...base,
      hasNextChange: false,
      canComment: false,
    });
    const next = hints.find((h) => h.label === "Next change");
    const comment = hints.find((h) => h.label === "Comment");
    expect(next).toMatchObject({ disabled: true });
    expect(next?.why).toMatch(/last change/);
    expect(comment).toMatchObject({
      disabled: true,
      why: "Select a line first",
    });
    expect(hints.find((h) => h.label === "Previous")?.disabled).toBeFalsy();
  });

  it("follows focus into the tree, a comment box, a dialog, and a text field", () => {
    expect(labels({ zone: "tree" })).toContain("↵ Open file");
    expect(labels({ zone: "comment" })).toEqual([
      "⌘↩ Save draft",
      "Esc Cancel",
    ]);
    expect(labels({ zone: "review" })).toContain("Esc Cancel");
    expect(labels({ zone: "text" })[0]).toMatch(/shortcuts are paused/);
  });

  it("offers no letter shortcuts on the other tabs, and never the repository list's", () => {
    const all = labels({ tab: "checks" }).join(" | ");
    expect(all).toContain("Tabs");
    expect(all).not.toContain("rows");
    expect(all).not.toContain("Next change");
  });
});
