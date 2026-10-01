/**
 * Window-level keyboard shortcuts (SPEC §4, Keyboard defaults). Lists react to
 * J/K and the arrow keys without being clicked first.
 *
 * One listener dispatches to every mounted `useKeys` registration in
 * registration order, and the first one that binds the key handles it. Since
 * React mounts children before parents, a component's own bindings win over
 * its container's. A shortcut never fires while the user types in a field,
 * while a modal dialog is open, or for keys a focused control needs itself
 * (Enter and Space on a button, the arrows in a scrolled patch).
 */
import { useEffect, useRef } from "react";

/** A binding: `"j"`, `"ArrowDown"`, `"["`, `"/"`, or with `mod+` for ⌘ (`"mod+1"`). */
export type KeyMap = Record<string, (e: KeyboardEvent) => void>;

type Registration = { map: { current: KeyMap } };

const registrations: Registration[] = [];

/**
 * Whether the last click landed in a region that scrolls with the arrow keys
 * (marked `data-own-arrows`). WebKit scrolls the clicked region with the
 * arrows without focusing it, so the key event's target cannot tell.
 */
let arrowsOwned = false;

/** True when the key press belongs to a text field rather than to the app. */
export function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    target.isContentEditable
  );
}

const ACTIVATES =
  'button, a[href], summary, [role="button"], [role="tab"], [role="menuitem"], [role="menuitemradio"], [role="checkbox"], [role="switch"]';

/** True when Enter or Space would activate the focused element itself. */
export function activatesTarget(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest(ACTIVATES);
}

/** The binding name of a key event, or null when an unbound modifier is held. */
export function bindingOf(e: KeyboardEvent): string | null {
  if (e.altKey || e.ctrlKey) return null;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (e.metaKey) return e.shiftKey ? null : `mod+${key}`;
  return key;
}

/** Whether the app may take this key press, or it belongs to what has focus. */
export function shortcutAllowed(e: KeyboardEvent, binding: string): boolean {
  if (document.querySelector('[aria-modal="true"]')) return false;
  if (binding.startsWith("mod+")) return true;
  if (isTyping(e.target)) return false;
  if ((binding === "Enter" || binding === " ") && activatesTarget(e.target))
    return false;
  if (
    binding.startsWith("Arrow") &&
    (arrowsOwned ||
      (e.target instanceof Element && !!e.target.closest("[data-own-arrows]")))
  )
    return false;
  return true;
}

function dispatch(e: KeyboardEvent) {
  if (e.defaultPrevented) return;
  const binding = bindingOf(e);
  if (!binding || !shortcutAllowed(e, binding)) return;
  for (const r of registrations) {
    const handler = r.map.current[binding];
    if (handler) {
      e.preventDefault();
      handler(e);
      return;
    }
  }
}

if (typeof window !== "undefined") {
  window.addEventListener("keydown", dispatch);
  window.addEventListener(
    "mousedown",
    (e) => {
      arrowsOwned =
        e.target instanceof Element && !!e.target.closest("[data-own-arrows]");
    },
    true,
  );
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
