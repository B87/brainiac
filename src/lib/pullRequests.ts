/**
 * Pull request lists (SPEC.md, section 10): filters, the order, and the
 * short labels the table shows.
 */
import { relativeTime } from "./format";
import type {
  ChangedFile,
  CheckState,
  ChecksSummary,
  DiffSide,
  ForgeKind,
  MergeMethod,
  PullRequest,
  PullRequestGroup,
  PullRequestState,
  RepositorySummary,
  RequestBudget,
  ReviewDraft,
  ReviewState,
  ReviewVerdict,
  Thread,
  ThreadAnchor,
} from "./ipc";
import { plural } from "./repo";

/** The workspace tab's filters, and the repository tab's `closed`. */
export type PullRequestFilter =
  | "all"
  | "needs_review"
  | "mine"
  | "others"
  | "drafts"
  | "closed";

export const WORKSPACE_FILTERS: Array<{
  value: PullRequestFilter;
  label: string;
}> = [
  { value: "all", label: "All open" },
  { value: "needs_review", label: "Needs your review" },
  { value: "mine", label: "Yours" },
  { value: "others", label: "Others" },
  { value: "drafts", label: "Drafts" },
];

export const REPOSITORY_FILTERS: Array<{
  value: PullRequestFilter;
  label: string;
}> = [
  { value: "all", label: "Open" },
  { value: "needs_review", label: "Needs your review" },
  { value: "mine", label: "Yours" },
  { value: "drafts", label: "Drafts" },
  { value: "closed", label: "Merged and closed" },
];

export function matchesFilter(
  pr: PullRequest,
  filter: PullRequestFilter,
): boolean {
  switch (filter) {
    case "all":
    case "closed":
      return true;
    case "needs_review":
      return pr.awaiting_my_review;
    case "mine":
      return pr.mine;
    case "others":
      return !pr.mine;
    case "drafts":
      return pr.state === "draft";
  }
}

/** What has waited longest first: the least recently updated. */
export function byLongestWait(a: PullRequest, b: PullRequest): number {
  return a.updated_at < b.updated_at ? -1 : a.updated_at > b.updated_at ? 1 : 0;
}

/** The pull request of the checked-out branch, found through its upstream only. */
export function checkedOutPullRequest(
  repository: RepositorySummary,
  pullRequests: PullRequest[],
): PullRequest | null {
  const branch = repository.head?.branch;
  const forge = repository.forge;
  if (!branch || !forge || !repository.upstream) return null;
  const own = `${forge.owner}/${forge.name}`;
  return (
    pullRequests.find(
      (pr) => pr.source_branch === branch && pr.source_repository === own,
    ) ?? null
  );
}

export const STATE_LABEL: Record<PullRequestState, string> = {
  open: "Open",
  draft: "Draft",
  merged: "Merged",
  closed: "Closed",
};

export const REVIEW_LABEL: Record<ReviewState, string> = {
  requested: "Review requested",
  approved: "Approved",
  changes_requested: "Changes requested",
  commented: "Commented",
};

export const PROVIDER_LABEL: Record<ForgeKind, string> = {
  github: "GitHub",
  bitbucket_cloud: "Bitbucket",
};

export const CHECK_LABEL: Record<CheckState, string> = {
  pending: "Pending",
  success: "Passed",
  failure: "Failed",
  neutral: "Skipped",
};

/** "2 of 3 passed", "1 failed", "no checks". */
export function checksLabel(checks: ChecksSummary): string {
  if (checks.state === null) return "No checks";
  if (checks.failed > 0) return `${checks.failed} of ${checks.total} failed`;
  if (checks.pending > 0)
    return `${checks.passed} of ${checks.total} passed, ${checks.pending} pending`;
  return `${checks.passed} of ${checks.total} passed`;
}

/** "+120 −8 in 4 files", or an em dash while unknown. */
export function sizeLabel(pr: PullRequest): string {
  const { additions, deletions, changed_files } = pr.counts;
  if (additions === null || deletions === null) return "—";
  const files =
    changed_files === null
      ? ""
      : ` in ${changed_files} ${changed_files === 1 ? "file" : "files"}`;
  return `+${additions} −${deletions}${files}`;
}

/** How long ago it was opened, as the age column. */
export function ageLabel(pr: PullRequest, now = Date.now()): string {
  return relativeTime(pr.created_at, now).replace(" ago", "");
}

/** The number without its repository: "#42". */
export function shortReference(pr: PullRequest): string {
  return `#${pr.number}`;
}

/** "github.com/acme/api" → "acme/api". */
export function repositoryName(forge: string): string {
  return forge.replace(/^[^/]+\//, "");
}

export function budgetLabel(b: RequestBudget): string {
  return `${b.used} of ${b.limit} requests used this hour`;
}

/** The oldest group read, for "as of" in the side panel. */
export function oldestFetch(groups: PullRequestGroup[]): string | null {
  const times = groups
    .map((g) => g.fetched_at)
    .filter((t): t is string => !!t)
    .sort();
  return times[0] ?? null;
}

/** How many pull requests wait on your review, for a badge. */
export function awaitingCount(groups: PullRequestGroup[]): number {
  return groups.reduce(
    (n, g) => n + g.pull_requests.filter((p) => p.awaiting_my_review).length,
    0,
  );
}

// --- The pull request view (SPEC.md, Pull request) ---------------------------

/** "N new commits since your review", or null when there is nothing new. */
export function sinceReviewLabel(pr: PullRequest): string | null {
  if (!pr.reviewed_sha || pr.reviewed_sha === pr.head_sha) return null;
  const n = pr.commits_since_review;
  if (n === null) return "New commits since your review";
  return `${n} new ${n === 1 ? "commit" : "commits"} since your review`;
}

/** Lock files, minified bundles, snapshots, and generated folders: folded in Files Changed. */
export function isGenerated(path: string): boolean {
  const name = path.slice(path.lastIndexOf("/") + 1);
  if (
    /^(package-lock\.json|pnpm-lock\.yaml|yarn\.lock|Cargo\.lock|composer\.lock|Gemfile\.lock|poetry\.lock|go\.sum)$/.test(
      name,
    )
  )
    return true;
  if (/\.(min\.(js|css)|snap|pb\.go|generated\.[a-z]+)$/.test(name))
    return true;
  return /(^|\/)(generated|__generated__|__snapshots__|dist|vendor)\//.test(
    path,
  );
}

/** What a viewed mark remembers of a file: when any of it changes, the mark clears. */
export function fileKey(f: ChangedFile): string {
  return `${f.status}:${f.additions}:${f.deletions}:${f.binary ? "b" : "t"}`;
}

export type ViewedMarks = Record<string, string>;

const VIEWED_PREFIX = "brainiac.pr.viewed:";

export function loadViewed(reference: string): ViewedMarks {
  try {
    const raw = localStorage.getItem(VIEWED_PREFIX + reference);
    return raw ? (JSON.parse(raw) as ViewedMarks) : {};
  } catch {
    return {};
  }
}

export function saveViewed(reference: string, marks: ViewedMarks): void {
  try {
    localStorage.setItem(VIEWED_PREFIX + reference, JSON.stringify(marks));
  } catch {
    // Private mode or a full store: the marks last for the session only.
  }
}

/** Whether a file is marked viewed and unchanged since. */
export function isViewed(marks: ViewedMarks, f: ChangedFile): boolean {
  return marks[f.path] === fileKey(f);
}

/** Threads on each path: how many, and how many still open. */
export function threadCounts(
  threads: Thread[],
): Map<string, { total: number; open: number }> {
  const out = new Map<string, { total: number; open: number }>();
  for (const t of threads) {
    if (!t.anchor) continue;
    const c = out.get(t.anchor.path) ?? { total: 0, open: 0 };
    c.total++;
    if (!t.resolved) c.open++;
    out.set(t.anchor.path, c);
  }
  return out;
}

/** Inline threads still open, as the merge checklist counts them. */
export function unresolvedThreads(threads: Thread[]): number {
  return threads.filter((t) => t.anchor && !t.resolved).length;
}

/** "src/parse.rs:12", "docs/a.md:3–7", or the path alone. */
export function anchorLabel(a: ThreadAnchor): string {
  if (a.line === null) return a.path;
  if (a.start_line !== null && a.start_line !== a.line)
    return `${a.path}:${a.start_line}–${a.line}`;
  return `${a.path}:${a.line}`;
}

// --- Reviewing (SPEC.md, Reviewing) ------------------------------------------

export const VERDICT_LABEL: Record<ReviewVerdict, string> = {
  comment: "Comment",
  approve: "Approve",
  request_changes: "Request changes",
};

/** Drafts not sent yet: the ones a review still has to post. */
export function unsentDrafts(drafts: ReviewDraft[]): ReviewDraft[] {
  return drafts.filter((d) => d.remote_id === null);
}

/**
 * The commit the review started on: the earliest unsent draft's. Null
 * without drafts, or when a draft carries no commit.
 */
export function reviewStartCommit(drafts: ReviewDraft[]): string | null {
  const unsent = unsentDrafts(drafts);
  if (unsent.length === 0) return null;
  const commits = unsent.map((d) => d.anchor.commit);
  return commits.every((c) => c !== null) ? (commits[0] ?? null) : null;
}

/** Whether any unsent draft was written on a commit other than `head`. */
export function draftsBehind(drafts: ReviewDraft[], head: string): boolean {
  return unsentDrafts(drafts).some(
    (d) => d.anchor.commit !== null && !sameCommit(d.anchor.commit, head),
  );
}

/** Bitbucket names commits by their first 12 characters until expanded. */
export function sameCommit(a: string, b: string): boolean {
  return a === b || a.startsWith(b) || b.startsWith(a);
}

/** What Finish Review needs before it can send: words, or a line comment, unless approving. */
export function reviewIncomplete(
  body: string,
  drafts: ReviewDraft[],
  verdict: ReviewVerdict,
): string | null {
  if (verdict === "approve") return null;
  if (body.trim() || unsentDrafts(drafts).length > 0) return null;
  return verdict === "comment"
    ? "Write a summary or a comment on a line first."
    : "Say what to change: a summary or a comment on a line.";
}

/** `new:12`: the key a diff line is annotated under. */
export function lineKey(side: DiffSide, line: number): string {
  return `${side}:${line}`;
}

export type LineNotes = { threads: Thread[]; drafts: ReviewDraft[] };

/**
 * The threads and drafts on each line of one file, keyed by `lineKey`.
 * Threads without a line (the provider no longer places them) are left out.
 */
export function lineNotes(
  path: string,
  threads: Thread[],
  drafts: ReviewDraft[],
): Map<string, LineNotes> {
  const out = new Map<string, LineNotes>();
  const at = (a: ThreadAnchor) => {
    if (a.path !== path || a.line === null) return null;
    const key = lineKey(a.side, a.line);
    let notes = out.get(key);
    if (!notes) {
      notes = { threads: [], drafts: [] };
      out.set(key, notes);
    }
    return notes;
  };
  for (const t of threads) {
    if (t.anchor) at(t.anchor)?.threads.push(t);
  }
  for (const d of drafts) at(d.anchor)?.drafts.push(d);
  return out;
}

// --- Merging (SPEC.md, Merging) ----------------------------------------------

export const MERGE_METHOD_LABEL: Record<MergeMethod, string> = {
  merge_commit: "Create a merge commit",
  squash: "Squash and merge",
  rebase: "Rebase and merge",
  fast_forward: "Fast-forward",
};

/** One line of the side panel's checklist; `state` is a `StateIcon` state. */
export type ChecklistItem = {
  key: "checks" | "reviews" | "threads" | "conflicts";
  state: "success" | "failure" | "pending" | "neutral" | "requested";
  label: string;
  /** Unmet, and the merge waits for it. */
  blocking: boolean;
};

/**
 * What the pull request needs before merging. Complete when nothing blocks:
 * no failed or running checks, no reviewer asking for changes, no unresolved
 * thread, no conflicts. Approvals are shown, not required.
 */
export function mergeChecklist(
  pr: PullRequest,
  unresolved: number | null,
): { items: ChecklistItem[]; complete: boolean } {
  const checks = pr.checks.state;
  const approvals = pr.reviewers.filter((r) => r.state === "approved").length;
  const changesRequested = pr.reviewers.some(
    (r) => r.state === "changes_requested",
  );
  const items: ChecklistItem[] = [
    {
      key: "checks",
      state:
        checks === null || checks === "success"
          ? "success"
          : checks === "failure"
            ? "failure"
            : checks === "pending"
              ? "pending"
              : "neutral",
      label: checksLabel(pr.checks),
      blocking: checks === "failure" || checks === "pending",
    },
    {
      key: "reviews",
      state: changesRequested
        ? "failure"
        : approvals > 0
          ? "success"
          : "requested",
      label: changesRequested
        ? `${plural(pr.reviewers.filter((r) => r.state === "changes_requested").length, "reviewer")} asked for changes`
        : approvals === 0
          ? "No approvals yet"
          : plural(approvals, "approval"),
      blocking: changesRequested,
    },
    {
      key: "threads",
      state:
        unresolved === 0
          ? "success"
          : unresolved === null
            ? "neutral"
            : "pending",
      label:
        unresolved === null
          ? "Threads not read yet"
          : unresolved === 0
            ? "No unresolved threads"
            : plural(unresolved, "unresolved thread"),
      blocking: unresolved !== null && unresolved > 0,
    },
    {
      key: "conflicts",
      state:
        pr.mergeability === "mergeable"
          ? "success"
          : pr.mergeability === "conflicting"
            ? "failure"
            : "neutral",
      label:
        pr.mergeability === "mergeable"
          ? "No conflicts"
          : pr.mergeability === "conflicting"
            ? "Has conflicts with the target"
            : pr.mergeability === "computing"
              ? "Checking for conflicts…"
              : "Conflicts are found when merging",
      blocking: pr.mergeability === "conflicting",
    },
  ];
  return { items, complete: items.every((i) => !i.blocking) };
}

/**
 * The commit message as the provider would write it: a merge commit names
 * the pull request with the title below; a squash is titled with the title
 * and number and carries the description; a rebase or fast-forward keeps
 * the commits' own messages. Bitbucket takes one text.
 */
export function defaultMergeMessage(
  pr: PullRequest,
  method: MergeMethod,
): { title: string; message: string } {
  if (method === "rebase" || method === "fast_forward")
    return { title: "", message: "" };
  if (pr.kind === "bitbucket_cloud") {
    return {
      title: "",
      message:
        method === "squash"
          ? `${pr.title} (pull request #${pr.number})\n\n${pr.description.trim()}`.trim()
          : `Merged in ${pr.source_branch} (pull request #${pr.number})\n\n${pr.title}`,
    };
  }
  return method === "squash"
    ? { title: `${pr.title} (#${pr.number})`, message: pr.description.trim() }
    : {
        title: `Merge pull request #${pr.number} from ${pr.source_repository.split("/")[0]}/${pr.source_branch}`,
        message: pr.title,
      };
}
