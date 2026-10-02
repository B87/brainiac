/**
 * A line diff of two versions of a note, for Compare… after a save conflict.
 * Common lines at both ends are matched first, then the middle by longest
 * common subsequence; a middle too large for that is shown as replaced.
 */
export type DiffRow =
  | { kind: "same"; text: string }
  | { kind: "removed"; text: string }
  | { kind: "added"; text: string };

const MAX_CELLS = 4_000_000;

export function diffLines(before: string, after: string): DiffRow[] {
  const a = before.split(/\r?\n/);
  const b = after.split(/\r?\n/);
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const rows: DiffRow[] = a
    .slice(0, start)
    .map((text) => ({ kind: "same", text }));
  const midA = a.slice(start, endA);
  const midB = b.slice(start, endB);
  if (midA.length * midB.length > MAX_CELLS) {
    for (const text of midA) rows.push({ kind: "removed", text });
    for (const text of midB) rows.push({ kind: "added", text });
  } else {
    // lcs[i][j]: length of the common subsequence of midA[i..] and midB[j..].
    const n = midA.length;
    const m = midB.length;
    const lcs: Uint32Array[] = Array.from(
      { length: n + 1 },
      () => new Uint32Array(m + 1),
    );
    for (let i = n - 1; i >= 0; i--)
      for (let j = m - 1; j >= 0; j--)
        lcs[i][j] =
          midA[i] === midB[j]
            ? lcs[i + 1][j + 1] + 1
            : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    let i = 0;
    let j = 0;
    while (i < n || j < m) {
      if (i < n && j < m && midA[i] === midB[j]) {
        rows.push({ kind: "same", text: midA[i] });
        i++;
        j++;
      } else if (j < m && (i === n || lcs[i][j + 1] >= lcs[i + 1][j])) {
        rows.push({ kind: "added", text: midB[j++] });
      } else {
        rows.push({ kind: "removed", text: midA[i++] });
      }
    }
  }
  for (const text of a.slice(endA)) rows.push({ kind: "same", text });
  return rows;
}
