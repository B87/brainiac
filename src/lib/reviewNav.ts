/**
 * Moving through a pull request's files and changes (SPEC.md, Files Changed).
 * A "change" is a diff hunk. These are the pure rules; the view owns the
 * state and the scrolling.
 */
import type { Check, CheckState } from "./ipc";

/** What navigation needs to know of a file. */
export type NavFile = { path: string; binary: boolean };

/** Which hunk of a file to land on: a fixed one, or whichever end is nearest. */
export type Landing = "first" | "last" | number;

export type ChangeTarget = { path: string; hunk: Landing };

/** The file `delta` steps from `path` in the active order, or null at an end (no wrap). */
export function fileTarget(
  files: ReadonlyArray<NavFile>,
  path: string | null,
  delta: 1 | -1,
): string | null {
  const i = files.findIndex((f) => f.path === path);
  if (i === -1) return null;
  return files[i + delta]?.path ?? null;
}

/**
 * Where the next (`delta` 1) or previous (-1) change is, from hunk `hunk` of
 * `path`, which has `hunkCount` hunks. Past the last hunk it is the next
 * file's first; before the first, the previous file's last. A file with no
 * text to show (binary) is skipped; a text file with no hunks is only found
 * out once it is read, so the view asks again from it (`stepPast`). Null at
 * the pull request's ends.
 */
export function changeTarget(
  files: ReadonlyArray<NavFile>,
  path: string,
  hunk: number,
  hunkCount: number,
  delta: 1 | -1,
): ChangeTarget | null {
  const inFile = hunk + delta;
  if (inFile >= 0 && inFile < hunkCount) return { path, hunk: inFile };
  return stepPast(files, path, delta);
}

/** The first or last change of the nearest file past `path` that can have one. */
export function stepPast(
  files: ReadonlyArray<NavFile>,
  path: string,
  delta: 1 | -1,
): ChangeTarget | null {
  const i = files.findIndex((f) => f.path === path);
  if (i === -1) return null;
  for (let j = i + delta; j >= 0 && j < files.length; j += delta) {
    if (!files[j].binary)
      return { path: files[j].path, hunk: delta > 0 ? "first" : "last" };
  }
  return null;
}

/** The hunk index a landing means in a file of `hunkCount` hunks, or null when it has none. */
export function resolveLanding(
  landing: Landing,
  hunkCount: number,
): number | null {
  if (hunkCount <= 0) return null;
  if (landing === "first") return 0;
  if (landing === "last") return hunkCount - 1;
  return Math.min(Math.max(0, landing), hunkCount - 1);
}

/**
 * The next file after `path` (wrapping once to the start) that is not
 * reviewed, in the active order; null when every file is.
 */
export function nextUnreviewed(
  files: ReadonlyArray<{ path: string }>,
  path: string | null,
  isReviewed: (path: string) => boolean,
): string | null {
  const start = files.findIndex((f) => f.path === path);
  for (let k = 1; k <= files.length; k++) {
    const f = files[(start + k + files.length) % files.length];
    if (f && f.path !== path && !isReviewed(f.path)) return f.path;
  }
  return null;
}

/** "3 of 42 reviewed", counted over the files of the whole pull request. */
export function progressLabel(reviewed: number, total: number): string {
  return `${reviewed} of ${total} reviewed`;
}

/** Files whose name contains every word of `query`, in their order; all when it is empty. */
export function findFiles<T extends { path: string }>(
  files: ReadonlyArray<T>,
  query: string,
): T[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [...files];
  return files.filter((f) => {
    const p = f.path.toLowerCase();
    return words.every((w) => p.includes(w));
  });
}

const CHECK_ORDER: Record<CheckState, number> = {
  failure: 0,
  pending: 1,
  success: 2,
  neutral: 3,
};

/** Failed checks first, then pending, passed, and skipped; by name within each. */
export function orderChecks(checks: ReadonlyArray<Check>): Check[] {
  return [...checks].sort(
    (a, b) =>
      CHECK_ORDER[a.state] - CHECK_ORDER[b.state] ||
      a.name.localeCompare(b.name),
  );
}
