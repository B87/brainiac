/**
 * What runs before the window closes: unsaved notes save, and open database
 * transactions ask first (SPEC.md, Databases: Safety). One window-level
 * handler runs every guard in turn, so no guard can close the window while
 * another one wants it to stay open.
 */
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Resolves to false to keep the window open. */
export type CloseGuard = () => Promise<boolean>;

const guards = new Set<CloseGuard>();

export function addCloseGuard(guard: CloseGuard): () => void {
  guards.add(guard);
  return () => {
    guards.delete(guard);
  };
}

/** Run every guard; true when the window may close. */
export async function runCloseGuards(): Promise<boolean> {
  for (const guard of [...guards]) {
    try {
      if (!(await guard())) return false;
    } catch {
      // A guard that fails does not keep the window open.
    }
  }
  return true;
}

/** Listen for the window's close button once, for the whole app. */
export function listenForClose(): () => void {
  const off = getCurrentWindow().onCloseRequested(async (event) => {
    if (!(await runCloseGuards())) event.preventDefault();
  });
  return () => {
    void off.then((unlisten) => unlisten());
  };
}
