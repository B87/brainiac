/**
 * The line diff of an edit the agent reports (SPEC.md, The run:
 * Conversation): its text before and after, or only the part it replaces.
 * Git's diffs come from the backend; these texts never reach Git.
 */

export type EditLine = { kind: "context" | "add" | "del"; text: string };

/** Past this many line pairs the texts are shown as removed, then added. */
const MAX_CELLS = 1_000_000;

function lines(text: string): string[] {
  if (text === "") return [];
  const split = text.split("\n");
  // A final newline ends the last line; it does not start another.
  if (split[split.length - 1] === "") split.pop();
  return split;
}

/**
 * Removed, added, and unchanged lines, in order: a longest common
 * subsequence of lines, with removals before additions in each change.
 */
export function editLines(oldText: string | null, newText: string): EditLine[] {
  const a = lines(oldText ?? "");
  const b = lines(newText);
  // Lines both texts start and end with need no table.
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const head: EditLine[] = a
    .slice(0, start)
    .map((text) => ({ kind: "context", text }));
  const tail: EditLine[] = a
    .slice(endA)
    .map((text) => ({ kind: "context", text }));
  const midA = a.slice(start, endA);
  const midB = b.slice(start, endB);
  const n = midA.length;
  const m = midB.length;
  if (n * m > MAX_CELLS) {
    return [
      ...head,
      ...midA.map((text): EditLine => ({ kind: "del", text })),
      ...midB.map((text): EditLine => ({ kind: "add", text })),
      ...tail,
    ];
  }
  // lcs[i][j]: the longest common run of midA[i..] and midB[j..].
  const lcs: Uint32Array[] = [];
  for (let i = 0; i <= n; i++) lcs.push(new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] =
        midA[i] === midB[j]
          ? lcs[i + 1][j + 1] + 1
          : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const middle: EditLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && midA[i] === midB[j]) {
      middle.push({ kind: "context", text: midA[i] });
      i++;
      j++;
    } else if (j >= m || (i < n && lcs[i + 1][j] >= lcs[i][j + 1])) {
      middle.push({ kind: "del", text: midA[i] });
      i++;
    } else {
      middle.push({ kind: "add", text: midB[j] });
      j++;
    }
  }
  return [...head, ...middle, ...tail];
}

/** Lines added and removed, for a summary. */
export function editCounts(rows: EditLine[]): {
  added: number;
  removed: number;
} {
  let added = 0;
  let removed = 0;
  for (const row of rows) {
    if (row.kind === "add") added++;
    else if (row.kind === "del") removed++;
  }
  return { added, removed };
}

/** A path inside the run's container, as the repository names it. */
export function workspacePath(path: string): string {
  return path.startsWith("/workspace/")
    ? path.slice("/workspace/".length)
    : path;
}
