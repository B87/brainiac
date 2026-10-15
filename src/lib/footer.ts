/**
 * The shortcuts the bottom bar shows (SPEC.md, Keyboard defaults). A view
 * that has contextual shortcuts, such as a pull request, publishes them
 * here; the bar shows them while the view is on screen.
 */
import { useSyncExternalStore } from "react";

export type FooterHint = {
  keys: string;
  label: string;
  /** Shown dimmed, with `why` as its explanation, instead of hidden. */
  disabled?: boolean;
  why?: string;
  /** Kept visible in a narrow window; the rest are dropped there. */
  essential?: boolean;
  /** A hint that is also a button. */
  onActivate?: () => void;
};

let current: FooterHint[] | null = null;
const listeners = new Set<() => void>();

/** Publish the hints (or null to clear them); returns nothing to undo. */
export function setFooterHints(hints: FooterHint[] | null): void {
  if (sameHints(current, hints)) return;
  current = hints;
  for (const l of listeners) l();
}

function sameHints(a: FooterHint[] | null, b: FooterHint[] | null): boolean {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  return a.every((h, i) => {
    const o = b[i];
    return (
      h.keys === o.keys &&
      h.label === o.label &&
      !!h.disabled === !!o.disabled &&
      h.why === o.why &&
      !!h.essential === !!o.essential &&
      !!h.onActivate === !!o.onActivate
    );
  });
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

export function useFooterHints(): FooterHint[] | null {
  return useSyncExternalStore(
    subscribe,
    () => current,
    () => null,
  );
}

/** Where keyboard focus is inside a pull request. */
export type ReviewZone =
  | "diff"
  | "tree"
  | "comment"
  | "reply"
  | "text"
  | "review"
  | "finder"
  | "help"
  | "merge";

export type ReviewFooterState = {
  tab: "overview" | "files" | "checks";
  zone: ReviewZone;
  /** Another change, or a file, exists in that direction. */
  hasNextChange: boolean;
  hasPrevChange: boolean;
  hasNextFile: boolean;
  hasPrevFile: boolean;
  /** A line is selected and commenting is on; otherwise why not. */
  canComment: boolean;
  commentWhy: string;
  onHelp: () => void;
};

/** The hints for where the reviewer is: the tab, and what has focus. */
export function reviewHints(s: ReviewFooterState): FooterHint[] {
  const help: FooterHint = {
    keys: "?",
    label: "All shortcuts",
    essential: true,
    onActivate: s.onHelp,
  };
  if (s.zone === "text")
    return [
      {
        keys: "",
        label: "Typing: letter shortcuts are paused",
        essential: true,
      },
    ];
  if (s.zone === "reply")
    return [{ keys: "⌘↩", label: "Send", essential: true }];
  if (s.zone === "help" || s.zone === "merge")
    return [
      {
        keys: "Esc",
        label: s.zone === "help" ? "Close" : "Cancel",
        essential: true,
      },
    ];
  if (s.zone === "comment")
    return [
      { keys: "⌘↩", label: "Save draft", essential: true },
      { keys: "Esc", label: "Cancel", essential: true },
    ];
  if (s.zone === "review")
    return [
      { keys: "⌘↩", label: "Submit from the summary", essential: true },
      { keys: "Esc", label: "Cancel", essential: true },
    ];
  if (s.zone === "finder")
    return [
      { keys: "↑ ↓", label: "Choose", essential: true },
      { keys: "↵", label: "Open", essential: true },
      { keys: "Esc", label: "Close", essential: true },
    ];
  if (s.tab !== "files")
    return [
      { keys: "⌘1 ⌘2 ⌘3", label: "Tabs" },
      { keys: "Esc", label: "Back" },
      help,
    ];
  if (s.zone === "tree")
    return [
      { keys: "↑ ↓", label: "Move", essential: true },
      { keys: "↵", label: "Open file", essential: true },
      { keys: "T", label: "Hide tree" },
      { keys: "Esc", label: "Back to the diff" },
      help,
    ];
  return [
    {
      keys: "J",
      label: "Next change",
      disabled: !s.hasNextChange,
      why: "This is the last change in the pull request",
    },
    {
      keys: "K",
      label: "Previous",
      disabled: !s.hasPrevChange,
      why: "This is the first change in the pull request",
    },
    {
      keys: "⇧J/K",
      label: "Files",
      disabled: !s.hasNextFile && !s.hasPrevFile,
      why: "There is no other file in this list",
    },
    { keys: "⌘P", label: "Find file" },
    {
      keys: "C",
      label: "Comment",
      disabled: !s.canComment,
      why: s.commentWhy,
    },
    { keys: "T", label: "File tree" },
    help,
  ];
}
