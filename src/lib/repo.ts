/** Presentation helpers shared by the sidebar, dashboard, and repository viewer. */
import type {
  ChangeEntry,
  CommitFile,
  CommitSummary,
  RepositorySummary,
} from "./ipc";

/** Visual state of a repository, driving the status dot. */
export type RepoTone =
  | "clean"
  | "dirty"
  | "conflict"
  | "stale"
  | "missing"
  | "error"
  | "refreshing"
  | "unknown";

export function repoTone(r: RepositorySummary): RepoTone {
  if (r.state === "missing") return "missing";
  if (r.state === "error") return "error";
  if (r.state === "refreshing") return "refreshing";
  if (r.counts?.conflicted) return "conflict";
  if (r.counts && r.counts.unique_paths > 0) return "dirty";
  if (r.state === "stale") return "stale";
  if (!r.counts) return "unknown";
  return "clean";
}

export const TONE_LABEL: Record<RepoTone, string> = {
  clean: "Clean",
  dirty: "Uncommitted changes",
  conflict: "Conflicts",
  stale: "Stale",
  missing: "Folder not found",
  error: "Error",
  refreshing: "Refreshing",
  unknown: "Not checked yet",
};

export function headLabel(r: RepositorySummary): string {
  const head = r.head;
  if (!head) return "—";
  if (head.kind === "branch") return head.branch ?? "—";
  if (head.kind === "detached")
    return `detached @ ${head.commit_id?.slice(0, 7) ?? "?"}`;
  return "no commits";
}

/** Short upstream comparison: "in sync", "↑2", "↓3", "↑1 ↓2", or null without upstream. */
export function upstreamLabel(r: RepositorySummary): string | null {
  const u = r.upstream;
  if (!u) return null;
  if (u.ahead === 0 && u.behind === 0) return "in sync";
  return [u.ahead ? `↑${u.ahead}` : "", u.behind ? `↓${u.behind}` : ""]
    .filter(Boolean)
    .join(" ");
}

/**
 * Short form of an upstream for tight spaces: just the remote (`origin`) when
 * the upstream branch has the same name as the local one, else the full ref.
 */
export function upstreamTarget(upstream: string, branch: string): string {
  const cut = upstream.indexOf("/");
  if (cut > 0 && upstream.slice(cut + 1) === branch)
    return upstream.slice(0, cut);
  return upstream;
}

export type KindTone = "mod" | "add" | "del" | "ren" | "other";

export const KIND_LETTER: Record<ChangeEntry["kind"], string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  copied: "C",
  type_changed: "T",
  unmerged: "!",
  untracked: "?",
};

export function kindTone(kind: ChangeEntry["kind"]): KindTone {
  switch (kind) {
    case "added":
      return "add";
    case "deleted":
    case "unmerged":
      return "del";
    case "renamed":
    case "copied":
      return "ren";
    case "untracked":
      return "other";
    default:
      return "mod";
  }
}

/** Splits a repository-relative path into its directory (with trailing slash) and file name. */
export function splitPath(path: string): { dir: string; name: string } {
  const cut = path.lastIndexOf("/");
  return cut === -1
    ? { dir: "", name: path }
    : { dir: path.slice(0, cut + 1), name: path.slice(cut + 1) };
}

export type Decoration = { label: string; tone: "head" | "tag" | "other" };

/** Turns Git's decoration strings into labeled chips: `HEAD -> main` becomes `HEAD → main`. */
export function decorations(c: CommitSummary): Decoration[] {
  return c.decorations.map((d) => {
    if (d.startsWith("tag: "))
      return { label: `tag ${d.slice(5)}`, tone: "tag" };
    if (d === "HEAD" || d.startsWith("HEAD -> "))
      return { label: d.replace(" -> ", " → "), tone: "head" };
    return { label: d, tone: "other" };
  });
}

/** Groups commits under local-day headings ("Today", "Yesterday", "Mon 28 Sep"). */
export function groupByDay(
  commits: CommitSummary[],
  now = new Date(),
): Array<{ label: string; commits: CommitSummary[] }> {
  const groups: Array<{ label: string; commits: CommitSummary[] }> = [];
  for (const c of commits) {
    const label = dayLabel(new Date(c.committed_at), now);
    const last = groups.at(-1);
    if (last && last.label === label) last.commits.push(c);
    else groups.push({ label, commits: [c] });
  }
  return groups;
}

export function dayLabel(date: Date, now = new Date()): string {
  const startOf = (d: Date) =>
    new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((startOf(now) - startOf(date)) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  return date.toLocaleDateString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
    ...(date.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
  });
}

/** Commit files grouped by directory: root files first, then each directory in path order. */
export function groupFilesByDir(
  files: CommitFile[],
): Array<{ dir: string; files: CommitFile[] }> {
  const byDir = new Map<string, CommitFile[]>();
  for (const f of files) {
    const { dir } = splitPath(f.path);
    const list = byDir.get(dir);
    if (list) list.push(f);
    else byDir.set(dir, [f]);
  }
  return [...byDir.entries()]
    .sort(([a], [b]) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)))
    .map(([dir, list]) => ({
      dir,
      files: list.sort((a, b) => a.path.localeCompare(b.path)),
    }));
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** Up to two initials: "Alex Kim" → "AK", "sam" → "S". */
export function initials(name: string): string {
  const words = name
    .trim()
    .split(/[\s._-]+/)
    .filter(Boolean);
  if (words.length === 0) return "?";
  const first = words[0][0] ?? "";
  const last = words.length > 1 ? (words.at(-1)?.[0] ?? "") : "";
  return (first + last).toUpperCase();
}

/** A stable avatar color (0 to 5) for a name. */
export function avatarTone(name: string): number {
  let h = 0;
  for (const c of name.toLowerCase()) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return h % 6;
}

/**
 * Shorten in the middle, keeping the start (often a ticket ID) and the end:
 * "feature/ABC-123-very-long-name" → "feature/ABC-1…long-name".
 */
export function middleTruncate(text: string, max: number): string {
  if (text.length <= max) return text;
  const keep = max - 1;
  const head = Math.ceil(keep / 2);
  return `${text.slice(0, head)}…${text.slice(text.length - (keep - head))}`;
}

/** Newest version first: "v2.10.0" before "v2.9.1"; non-versions after, by name. */
export function compareVersionsDesc(a: string, b: string): number {
  const parts = (s: string) => s.match(/\d+/g)?.map(Number) ?? null;
  const pa = parts(a);
  const pb = parts(b);
  if (pa && pb) {
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
      const d = (pb[i] ?? -1) - (pa[i] ?? -1);
      if (d) return d;
    }
    return a.localeCompare(b);
  }
  if (pa) return -1;
  if (pb) return 1;
  return a.localeCompare(b);
}

/** Splits "author:alex fix login" into the author part and the message part. */
export function parseHistoryFilter(input: string): {
  text: string;
  author: string | null;
} {
  let author: string | null = null;
  const text = input
    .replace(/(?:^|\s)author:(?:"([^"]*)"|(\S*))/i, (_, quoted, word) => {
      author = (quoted ?? word ?? "").trim() || null;
      return " ";
    })
    .trim();
  return { text, author };
}

/** Width percentages of the added and removed parts of a change bar. */
export function barWidths(
  additions: number | null,
  deletions: number | null,
): { add: number; del: number } {
  const a = additions ?? 0;
  const d = deletions ?? 0;
  const total = a + d;
  if (!total) return { add: 0, del: 0 };
  return {
    add: Math.round((a / total) * 100),
    del: 100 - Math.round((a / total) * 100),
  };
}

/** True when an ISO time is more than `days` days in the past. */
export function olderThan(
  iso: string | null | undefined,
  days: number,
  now = Date.now(),
): boolean {
  if (!iso) return false;
  const t = Date.parse(iso);
  return !Number.isNaN(t) && now - t > days * 86_400_000;
}
