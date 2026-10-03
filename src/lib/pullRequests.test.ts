import { describe, expect, it } from "vitest";
import type {
  PullRequest,
  RepositorySummary,
  ReviewDraft,
  Thread,
} from "./ipc";
import {
  anchorLabel,
  byLongestWait,
  checkedOutPullRequest,
  checksLabel,
  defaultMergeMessage,
  draftsBehind,
  fileKey,
  isGenerated,
  isViewed,
  lineNotes,
  matchesFilter,
  mergeChecklist,
  reviewIncomplete,
  reviewStartCommit,
  sinceReviewLabel,
  sizeLabel,
  threadCounts,
  unresolvedThreads,
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
      resolve: { allowed: true, reason: null },
    },
    mine: false,
    awaiting_my_review: false,
    description_html: "",
    base_sha: "b".repeat(40),
    reviewed_sha: null,
    commits_since_review: null,
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

  it("says what arrived since your review", () => {
    expect(sinceReviewLabel(pr({}))).toBeNull();
    expect(sinceReviewLabel(pr({ reviewed_sha: "a".repeat(40) }))).toBeNull();
    expect(
      sinceReviewLabel(
        pr({ reviewed_sha: "c".repeat(40), commits_since_review: 1 }),
      ),
    ).toBe("1 new commit since your review");
    expect(sinceReviewLabel(pr({ reviewed_sha: "c".repeat(40) }))).toBe(
      "New commits since your review",
    );
  });

  it("folds generated files and keeps viewed marks until a file changes", () => {
    expect(isGenerated("pnpm-lock.yaml")).toBe(true);
    expect(isGenerated("web/dist/app.min.js")).toBe(true);
    expect(isGenerated("src/lib/generated/Task.ts")).toBe(true);
    expect(isGenerated("src/lib/tasks.ts")).toBe(false);
    const file = {
      path: "a.rs",
      old_path: null,
      status: "modified" as const,
      additions: 3,
      deletions: 1,
      binary: false,
    };
    const marks = { "a.rs": fileKey(file) };
    expect(isViewed(marks, file)).toBe(true);
    expect(isViewed(marks, { ...file, additions: 4 })).toBe(false);
    expect(isViewed({}, file)).toBe(false);
  });

  it("counts threads per file and labels where they hang", () => {
    const thread = (path: string, resolved: boolean, line = 3): Thread => ({
      id: path + line,
      anchor: { path, side: "new", line, start_line: null, commit: null },
      resolved,
      outdated: false,
      comments: [],
    });
    const threads = [
      thread("a.rs", false),
      thread("a.rs", true, 9),
      thread("b.rs", false),
      { ...thread("c.rs", false), anchor: null },
    ];
    expect(threadCounts(threads).get("a.rs")).toEqual({ total: 2, open: 1 });
    expect(threadCounts(threads).has("c.rs")).toBe(false);
    expect(unresolvedThreads(threads)).toBe(2);
    expect(
      anchorLabel({
        path: "a.rs",
        side: "new",
        line: 12,
        start_line: 10,
        commit: null,
      }),
    ).toBe("a.rs:10–12");
    expect(
      anchorLabel({
        path: "a.rs",
        side: "old",
        line: null,
        start_line: null,
        commit: null,
      }),
    ).toBe("a.rs");
  });

  it("knows which commit a review started on and what it still needs", () => {
    const draft = (
      path: string,
      line: number,
      commit: string,
      remote: string | null = null,
    ): ReviewDraft => ({
      id: `${path}:${line}`,
      reference: "github.com/acme/api#1",
      anchor: { path, side: "new", line, start_line: null, commit },
      body: "x",
      html: "<p>x</p>",
      remote_id: remote,
      created_at: "",
      updated_at: "",
    });
    const head = "a".repeat(40);
    const drafts = [draft("a.rs", 3, head), draft("b.rs", 5, "0".repeat(40))];
    expect(reviewStartCommit(drafts)).toBe(head);
    expect(draftsBehind(drafts, head)).toBe(true);
    expect(draftsBehind([drafts[0]], head)).toBe(false);
    // Bitbucket's short commits count as the same.
    expect(draftsBehind([draft("a.rs", 3, head.slice(0, 12))], head)).toBe(
      false,
    );
    // A draft already sent no longer holds the review back.
    expect(draftsBehind([draft("b.rs", 5, "0".repeat(40), "10")], head)).toBe(
      false,
    );
    expect(reviewStartCommit([])).toBeNull();
    expect(reviewIncomplete("", [], "comment")).toMatch(/Write a summary/);
    expect(reviewIncomplete("", [], "request_changes")).toMatch(
      /what to change/,
    );
    expect(reviewIncomplete("", [], "approve")).toBeNull();
    expect(reviewIncomplete("", drafts, "comment")).toBeNull();
    expect(reviewIncomplete("ok", [], "comment")).toBeNull();
  });

  it("places threads and drafts on their lines of one file", () => {
    const thread = (path: string, line: number | null): Thread => ({
      id: `${path}:${line}`,
      anchor: { path, side: "new", line, start_line: null, commit: null },
      resolved: false,
      outdated: false,
      comments: [],
    });
    const draft: ReviewDraft = {
      id: "d1",
      reference: "r",
      anchor: {
        path: "a.rs",
        side: "old",
        line: 7,
        start_line: null,
        commit: null,
      },
      body: "x",
      html: "<p>x</p>",
      remote_id: null,
      created_at: "",
      updated_at: "",
    };
    const notes = lineNotes(
      "a.rs",
      [
        thread("a.rs", 3),
        thread("a.rs", 3),
        thread("b.rs", 3),
        thread("a.rs", null),
      ],
      [draft],
    );
    expect(notes.get("new:3")?.threads).toHaveLength(2);
    expect(notes.get("old:7")?.drafts).toEqual([draft]);
    expect(notes.size).toBe(2);
  });

  it("completes the merge checklist only when nothing blocks", () => {
    const ready = pr({
      checks: { state: "success", total: 1, passed: 1, failed: 0, pending: 0 },
      reviewers: [],
      mergeability: "mergeable",
    });
    const done = mergeChecklist(ready, 0);
    expect(done.complete).toBe(true);
    expect(done.items.map((i) => i.label)).toEqual([
      "1 of 1 passed",
      "No approvals yet",
      "No unresolved threads",
      "No conflicts",
    ]);
    expect(mergeChecklist(ready, 2).complete).toBe(false);
    expect(mergeChecklist(ready, null).complete).toBe(true);
    const running = pr({
      ...ready,
      checks: { state: "pending", total: 2, passed: 1, failed: 0, pending: 1 },
    });
    expect(mergeChecklist(running, 0).items[0]).toMatchObject({
      state: "pending",
      blocking: true,
    });
    const asked = pr({
      ...ready,
      reviewers: [
        {
          user: { id: "9", login: "bob", display_name: null },
          state: "changes_requested",
          is_me: false,
        },
      ],
    });
    expect(mergeChecklist(asked, 0).items[1]).toMatchObject({
      label: "1 reviewer asked for changes",
      blocking: true,
    });
    expect(
      mergeChecklist(pr({ ...ready, mergeability: "unknown" }), 0).complete,
    ).toBe(true);
    expect(
      mergeChecklist(pr({ ...ready, mergeability: "conflicting" }), 0).complete,
    ).toBe(false);
  });

  it("prefills the commit message as the provider would", () => {
    const github = pr({
      number: 12,
      title: "Parse nested lists",
      description: "Lists inside lists.\n",
      source_repository: "team/parser",
      source_branch: "lists",
    });
    expect(defaultMergeMessage(github, "merge_commit")).toEqual({
      title: "Merge pull request #12 from team/lists",
      message: "Parse nested lists",
    });
    expect(defaultMergeMessage(github, "squash")).toEqual({
      title: "Parse nested lists (#12)",
      message: "Lists inside lists.",
    });
    expect(defaultMergeMessage(github, "rebase")).toEqual({
      title: "",
      message: "",
    });
    const bitbucket = pr({ ...github, kind: "bitbucket_cloud" });
    expect(defaultMergeMessage(bitbucket, "merge_commit")).toEqual({
      title: "",
      message: "Merged in lists (pull request #12)\n\nParse nested lists",
    });
    expect(defaultMergeMessage(bitbucket, "squash").message).toBe(
      "Parse nested lists (pull request #12)\n\nLists inside lists.",
    );
  });
});
