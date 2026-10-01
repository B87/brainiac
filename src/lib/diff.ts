/**
 * Patch presentation computed in the frontend (SPEC §7, Changes and diffs):
 * pairing removed and added lines, split rows, and changed-word highlights.
 */
import type { DiffLine, Hunk } from "./ipc";

/** A piece of a line's text; `changed` pieces are highlighted. */
export type Segment = { text: string; changed: boolean };

/** One row of the split layout: the old line on the left, the new on the right. */
export type SplitRow =
  | { kind: "hunk"; hunk: Hunk; index: number }
  | { kind: "pair"; left: DiffLine | null; right: DiffLine | null };

/** Rows of the unified layout. */
export type UnifiedRow =
  | { kind: "hunk"; hunk: Hunk; index: number }
  | { kind: "line"; line: DiffLine };

export function unifiedRows(hunks: Hunk[]): UnifiedRow[] {
  const rows: UnifiedRow[] = [];
  hunks.forEach((hunk, index) => {
    rows.push({ kind: "hunk", hunk, index });
    for (const line of hunk.lines) rows.push({ kind: "line", line });
  });
  return rows;
}

/**
 * Each run of removed lines followed by a run of added lines is a modified
 * block; its lines pair up in order. Context lines pair with themselves.
 */
export function pairLines(
  lines: DiffLine[],
): Array<{ left: DiffLine | null; right: DiffLine | null }> {
  const out: Array<{ left: DiffLine | null; right: DiffLine | null }> = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.kind === "context") {
      out.push({ left: line, right: line });
      i++;
      continue;
    }
    const dels: DiffLine[] = [];
    const adds: DiffLine[] = [];
    while (i < lines.length && lines[i].kind === "delete")
      dels.push(lines[i++]);
    while (i < lines.length && lines[i].kind === "add") adds.push(lines[i++]);
    for (let k = 0; k < Math.max(dels.length, adds.length); k++)
      out.push({ left: dels[k] ?? null, right: adds[k] ?? null });
  }
  return out;
}

export function splitRows(hunks: Hunk[]): SplitRow[] {
  const rows: SplitRow[] = [];
  hunks.forEach((hunk, index) => {
    rows.push({ kind: "hunk", hunk, index });
    for (const p of pairLines(hunk.lines)) rows.push({ kind: "pair", ...p });
  });
  return rows;
}

/** Words, runs of spaces, and single punctuation characters. */
export function tokenize(text: string): string[] {
  return text.match(/[\p{L}\p{N}_]+|\s+|[^\p{L}\p{N}_\s]/gu) ?? [];
}

/** Lines longer than this many tokens or characters are not compared word by word. */
const MAX_TOKENS = 200;
const MAX_CHARS = 500;

/**
 * Changed-word segments for a removed/added line pair, from the longest
 * common token subsequence. Returns null when the lines share too little for
 * highlights to help (then the whole line is the change).
 */
export function wordDiff(
  oldText: string,
  newText: string,
): { old: Segment[]; new: Segment[] } | null {
  if (oldText.length > MAX_CHARS || newText.length > MAX_CHARS) return null;
  const a = tokenize(oldText);
  const b = tokenize(newText);
  if (a.length > MAX_TOKENS || b.length > MAX_TOKENS) return null;
  // lcs[i][j] = length of the common subsequence of a[i..] and b[j..].
  const lcs: Uint16Array[] = Array.from(
    { length: a.length + 1 },
    () => new Uint16Array(b.length + 1),
  );
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      lcs[i][j] =
        a[i] === b[j]
          ? lcs[i + 1][j + 1] + 1
          : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
  // Mostly different lines read better as a whole change than as confetti.
  const common = lcs[0][0];
  if (common === 0 || common / Math.max(a.length, b.length) < 0.25) return null;

  const oldSegs: Segment[] = [];
  const newSegs: Segment[] = [];
  const push = (segs: Segment[], text: string, changed: boolean) => {
    const last = segs.at(-1);
    if (last && last.changed === changed) last.text += text;
    else segs.push({ text, changed });
  };
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      push(oldSegs, a[i++], false);
      push(newSegs, b[j++], false);
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) push(oldSegs, a[i++], true);
    else push(newSegs, b[j++], true);
  }
  while (i < a.length) push(oldSegs, a[i++], true);
  while (j < b.length) push(newSegs, b[j++], true);
  return { old: oldSegs, new: newSegs };
}

/**
 * Changed-word highlights, computed on demand: only rows that are rendered
 * pay for the comparison, and each pair is compared once. Pairing the lines
 * is linear; the word comparison is quadratic per pair, which is why it is
 * not done for the whole patch up front.
 */
export function wordHighlighter(
  hunks: Hunk[],
): (line: DiffLine) => Segment[] | undefined {
  const partner = new Map<DiffLine, DiffLine>();
  for (const hunk of hunks)
    for (const { left, right } of pairLines(hunk.lines))
      if (left?.kind === "delete" && right?.kind === "add") {
        partner.set(left, right);
        partner.set(right, left);
      }
  const cache = new Map<DiffLine, Segment[] | null>();
  return (line) => {
    const other = partner.get(line);
    if (!other) return undefined;
    if (!cache.has(line)) {
      const [del, add] = line.kind === "delete" ? [line, other] : [other, line];
      const d = wordDiff(del.text, add.text);
      cache.set(del, d?.old ?? null);
      cache.set(add, d?.new ?? null);
    }
    return cache.get(line) ?? undefined;
  };
}

/** Length of the longest line on each side, for sizing split columns. */
export function longestLines(hunks: Hunk[]): { old: number; new: number } {
  let old = 0;
  let neu = 0;
  for (const h of hunks)
    for (const l of h.lines) {
      if (l.kind !== "add") old = Math.max(old, l.text.length);
      if (l.kind !== "delete") neu = Math.max(neu, l.text.length);
    }
  return { old, new: neu };
}

/** Index of the hunk that contains row `row` (rows as built by the layouts). */
export function hunkAtRow(
  rows: ReadonlyArray<{ kind: string; index?: number }>,
  row: number,
): number {
  for (let r = Math.min(row, rows.length - 1); r >= 0; r--) {
    const x = rows[r];
    if (x.kind === "hunk") return x.index ?? 0;
  }
  return 0;
}

/** Row index of each hunk header. */
export function hunkRowIndexes(
  rows: ReadonlyArray<{ kind: string }>,
): number[] {
  const out: number[] = [];
  rows.forEach((r, i) => {
    if (r.kind === "hunk") out.push(i);
  });
  return out;
}
