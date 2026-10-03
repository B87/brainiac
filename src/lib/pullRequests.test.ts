import { describe, expect, it } from "vitest";
import type { PullRequest, RepositorySummary } from "./ipc";
import {
  byLongestWait,
  checkedOutPullRequest,
  checksLabel,
  matchesFilter,
  sizeLabel,
} from "./pullRequests";

function pr(over: Partial<PullRequest>): PullRequest {
  return {
    reference: "github.com/acme/api#1",
    number: 1,
    kind: "github",
    title: "Add parser",
    description: "",
    author: { id: "7", login: "ada", display_name: null },
    state: "open",
    source_repository: "acme/api",
    source_branch: "feature",
    head_sha: "a".repeat(40),
    target_branch: "main",
    reviewers: [],
    checks: { state: null, total: 0, passed: 0, failed: 0, pending: 0 },
    mergeability: "mergeable",
    counts: {
      comments: 0,
      unresolved_threads: null,
      additions: null,
      deletions: null,
      changed_files: null,
      commits: null,
    },
    web_url: "https://github.com/acme/api/pull/1",
    created_at: "2026-10-01T10:00:00.000Z",
    updated_at: "2026-10-03T10:00:00.000Z",
    closed_at: null,
    version: "v",
    actions: {
      comment: { allowed: true, reason: null },
      review: { allowed: true, reason: null },
      approve: { allowed: true, reason: null },
      merge: { allowed: true, reason: null },
    },
    mine: false,
    awaiting_my_review: false,
    ...over,
  };
}

describe("pull request lists", () => {
  it("filters by who and what", () => {
    const mine = pr({ mine: true });
    const waiting = pr({ awaiting_my_review: true });
    const draft = pr({ state: "draft" });
    expect(matchesFilter(mine, "mine")).toBe(true);
    expect(matchesFilter(mine, "others")).toBe(false);
    expect(matchesFilter(waiting, "needs_review")).toBe(true);
    expect(matchesFilter(mine, "needs_review")).toBe(false);
    expect(matchesFilter(draft, "drafts")).toBe(true);
    expect(matchesFilter(mine, "drafts")).toBe(false);
    expect(matchesFilter(draft, "all")).toBe(true);
  });

  it("puts what waited longest first", () => {
    const old = pr({ number: 1, updated_at: "2026-09-01T00:00:00.000Z" });
    const recent = pr({ number: 2, updated_at: "2026-10-03T00:00:00.000Z" });
    expect([recent, old].sort(byLongestWait).map((p) => p.number)).toEqual([
      1, 2,
    ]);
  });

  it("finds the checked-out branch's pull request through its upstream only", () => {
    const base = {
      head: { kind: "branch", branch: "feature", commit_id: "x" },
      forge: {
        kind: "github",
        owner: "acme",
        name: "api",
        reference: "github.com/acme/api",
        source: "origin",
      },
      upstream: { ref: "origin/feature", ahead: 2, behind: 0 },
    } as unknown as RepositorySummary;
    const match = pr({ source_branch: "feature" });
    const fork = pr({ source_branch: "feature", source_repository: "jo/api" });
    expect(checkedOutPullRequest(base, [fork, match])).toBe(match);
    expect(
      checkedOutPullRequest({ ...base, upstream: null }, [match]),
    ).toBeNull();
  });

  it("labels checks and sizes", () => {
    expect(
      checksLabel({ state: null, total: 0, passed: 0, failed: 0, pending: 0 }),
    ).toBe("No checks");
    expect(
      checksLabel({
        state: "failure",
        total: 3,
        passed: 1,
        failed: 1,
        pending: 1,
      }),
    ).toBe("1 of 3 failed");
    expect(
      checksLabel({
        state: "pending",
        total: 3,
        passed: 2,
        failed: 0,
        pending: 1,
      }),
    ).toBe("2 of 3 passed, 1 pending");
    expect(sizeLabel(pr({}))).toBe("—");
    expect(
      sizeLabel(
        pr({
          counts: {
            comments: 0,
            unresolved_threads: 0,
            additions: 12,
            deletions: 3,
            changed_files: 1,
            commits: 1,
          },
        }),
      ),
    ).toBe("+12 −3 in 1 file");
  });
});
