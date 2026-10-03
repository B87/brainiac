/**
 * Pull request lists (SPEC.md, section 10): filters, the order, and the
 * short labels the table shows.
 */
import { relativeTime } from "./format";
import type {
  CheckState,
  ChecksSummary,
  ForgeKind,
  PullRequest,
  PullRequestGroup,
  PullRequestState,
  RepositorySummary,
  RequestBudget,
  ReviewState,
} from "./ipc";

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
