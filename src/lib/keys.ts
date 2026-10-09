/**
 * Window-level keyboard shortcuts (SPEC.md, Keyboard defaults). Lists react to
 * J/K and the arrow keys without being clicked first.
 *
 * One listener dispatches to every mounted `useKeys` registration in
 * registration order, and the first one that binds the key handles it. A
 * registration is made when a component mounts or re-enables its bindings,
 * so two components shown together should not bind the same key; today none
 * do (each view binds its own keys, and the diff binds N, P, [ and ]).
 *
 * A shortcut never fires while the user types in a field, while a modal
 * dialog or a menu is open, or for keys a focused control needs itself
 * (Enter and Space on a button, the arrows in a scrolled patch).
 */
import { useEffect, useRef } from "react";

/**
 * A binding: `"j"`, `"ArrowDown"`, `"["`, `"/"`, with `mod+` for ⌘ (`"mod+1"`),
 * or with `shift+` for a shifted letter (`"shift+e"`). A shifted letter that
 * nothing binds as `shift+` goes to its plain binding.
 */
export type KeyMap = Record<string, (e: KeyboardEvent) => void>;

type Registration = { map: { current: KeyMap } };

/** What a key event's target looks like, as far as routing cares. */
export type KeyTarget = {
  tagName?: string;
  isContentEditable?: boolean;
  closest?: (selector: string) => unknown;
} | null;

/** The state of the page a key press arrives in. */
export type KeyContext = {
  /** A modal dialog is open; it owns the keyboard. */
  modalOpen: boolean;
  /** A popover menu is open; plain keys belong to it. */
  menuOpen: boolean;
  /** The last click landed in a region that scrolls with the arrows. */
  arrowsOwned: boolean;
};

const registrations: Registration[] = [];

/**
 * Whether the last click landed in a region that scrolls with the arrow keys
 * (marked `data-own-arrows`). WebKit scrolls the clicked region with the
 * arrows without focusing it, so the key event's target cannot tell.
 */
let arrowsOwned = false;

const ACTIVATES =
  'button, a[href], summary, [role="button"], [role="tab"], [role="menuitem"], [role="menuitemradio"], [role="checkbox"], [role="switch"]';

function within(target: KeyTarget, selector: string): boolean {
  return !!target?.closest?.(selector);
}

/** True when the key press belongs to a text field rather than to the app. */
export function isTyping(target: KeyTarget): boolean {
  const tag = target?.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    !!target?.isContentEditable
  );
}

/** The binding name of a key event, or null when an unbound modifier is held. */
export function bindingOf(
  e: Pick<KeyboardEvent, "key" | "altKey" | "ctrlKey" | "metaKey" | "shiftKey">,
): string | null {
  if (e.altKey || e.ctrlKey) return null;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (e.metaKey) return e.shiftKey ? null : `mod+${key}`;
  return key;
}

/** `shift+x` for a shifted letter, tried before its plain binding. */
export function shiftedBindingOf(
  e: Pick<KeyboardEvent, "key" | "altKey" | "ctrlKey" | "metaKey" | "shiftKey">,
): string | null {
  if (!e.shiftKey || e.altKey || e.ctrlKey || e.metaKey) return null;
  return /^[a-z]$/i.test(e.key) ? `shift+${e.key.toLowerCase()}` : null;
}

/** Whether the app may take this key press, or it belongs to what has focus. */
export function shortcutAllowed(
  binding: string,
  target: KeyTarget,
  ctx: KeyContext,
): boolean {
  if (ctx.modalOpen) return false;
  if (binding.startsWith("mod+")) return true;
  if (ctx.menuOpen) return false;
  if (isTyping(target)) return false;
  if ((binding === "Enter" || binding === " ") && within(target, ACTIVATES))
    return false;
  if (
    binding.startsWith("Arrow") &&
    (ctx.arrowsOwned || within(target, "[data-own-arrows]"))
  )
    return false;
  return true;
}

/** The handler that takes `binding`: the first registration that binds it. */
export function pickHandler(
  binding: string,
  maps: ReadonlyArray<KeyMap>,
): ((e: KeyboardEvent) => void) | null {
  for (const map of maps) if (map[binding]) return map[binding];
  return null;
}

function dispatch(e: KeyboardEvent) {
  if (e.defaultPrevented) return;
  const binding = bindingOf(e);
  if (!binding) return;
  const ctx: KeyContext = {
    modalOpen: !!document.querySelector('[aria-modal="true"]'),
    menuOpen: !!document.querySelector('[role="menu"]'),
    arrowsOwned,
  };
  const target = e.target instanceof Element ? (e.target as HTMLElement) : null;
  if (!shortcutAllowed(binding, target, ctx)) return;
  const maps = registrations.map((r) => r.map.current);
  const shifted = shiftedBindingOf(e);
  const handler =
    (shifted && pickHandler(shifted, maps)) || pickHandler(binding, maps);
  if (handler) {
    e.preventDefault();
    handler(e);
  }
}

if (typeof window !== "undefined") {
  window.addEventListener("keydown", dispatch);
  // Arrows belong to a patch from the click that lands in it until the
  // pointer or the focus moves elsewhere.
  const track = (e: Event) => {
    arrowsOwned =
      e.target instanceof Element && !!e.target.closest("[data-own-arrows]");
  };
  window.addEventListener("mousedown", track, true);
  window.addEventListener("focusin", track, true);
}

/**
 * Register shortcuts while the component is mounted. Handlers may change on
 * every render; the latest ones run. Pass `enabled = false` to pause.
 */
export function useKeys(map: KeyMap, enabled = true) {
  const ref = useRef(map);
  ref.current = map;
  useEffect(() => {
    if (!enabled) return;
    const registration: Registration = { map: ref };
    registrations.push(registration);
    return () => {
      const i = registrations.indexOf(registration);
      if (i !== -1) registrations.splice(i, 1);
    };
  }, [enabled]);
}

/** Move a selection by `delta` within `items`, clamped; returns the item to select. */
export function step<T>(
  items: T[],
  current: T | null,
  delta: number,
): T | null {
  if (items.length === 0) return null;
  const i = current === null ? -1 : items.indexOf(current);
  if (i === -1) return items[delta > 0 ? 0 : items.length - 1];
  return items[Math.min(items.length - 1, Math.max(0, i + delta))];
}
