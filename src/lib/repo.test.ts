import { describe, expect, it } from "vitest";
import type { CommitFile, CommitSummary, RepositorySummary } from "./ipc";
import {
  avatarTone,
  barWidths,
  branchStatus,
  compareVersionsDesc,
  dayLabel,
  decorations,
  groupByDay,
  groupFilesByDir,
  initials,
  middleTruncate,
  olderThan,
  parentFolder,
  parseHistoryFilter,
  relocatedNotice,
  relocationQuestion,
  upstreamTarget,
} from "./repo";

function commit(id: string, committed_at: string, decos: string[] = []) {
  return {
    id,
    short_id: id.slice(0, 7),
    subject: id,
    author_name: "A",
    author_email: "a@example.com",
    authored_at: committed_at,
    committed_at,
    parent_ids: [],
    decorations: decos,
  } satisfies CommitSummary;
}

function file(path: string): CommitFile {
  return {
    path,
    old_path: null,
    kind: "modified",
    additions: 1,
    deletions: 0,
    is_binary: false,
  };
}

describe("decorations", () => {
  it("labels HEAD, tags, and other refs", () => {
    const c = commit("a", "2026-01-01T00:00:00Z", [
      "HEAD -> main",
      "tag: v1.0",
      "origin/main",
    ]);
    expect(decorations(c)).toEqual([
      { label: "HEAD → main", tone: "head" },
      { label: "tag v1.0", tone: "tag" },
      { label: "origin/main", tone: "other" },
    ]);
  });
});

describe("day grouping", () => {
  const now = new Date(2026, 9, 1, 12, 0);
  it("names today and yesterday", () => {
    expect(dayLabel(new Date(2026, 9, 1, 1, 0), now)).toBe("Today");
    expect(dayLabel(new Date(2026, 8, 30, 23, 0), now)).toBe("Yesterday");
  });
  it("merges consecutive commits of the same day", () => {
    const groups = groupByDay(
      [
        commit("a", new Date(2026, 9, 1, 11).toISOString()),
        commit("b", new Date(2026, 9, 1, 9).toISOString()),
        commit("c", new Date(2026, 8, 30, 9).toISOString()),
      ],
      now,
    );
    expect(groups.map((g) => [g.label, g.commits.length])).toEqual([
      ["Today", 2],
      ["Yesterday", 1],
    ]);
  });
});

describe("groupFilesByDir", () => {
  it("puts root files first and sorts directories", () => {
    const groups = groupFilesByDir([
      file("src/b.ts"),
      file("README.md"),
      file("docs/x.md"),
      file("src/a.ts"),
    ]);
    expect(groups.map((g) => [g.dir, g.files.map((f) => f.path)])).toEqual([
      ["", ["README.md"]],
      ["docs/", ["docs/x.md"]],
      ["src/", ["src/a.ts", "src/b.ts"]],
    ]);
  });
});

describe("upstreamTarget", () => {
  it("shortens a same-named upstream to its remote", () => {
    expect(
      upstreamTarget("origin/feature/long-name", "feature/long-name"),
    ).toBe("origin");
    expect(upstreamTarget("origin/main", "develop")).toBe("origin/main");
  });
});

describe("people and names", () => {
  it("takes initials from the first and last word", () => {
    expect(initials("Alex Kim")).toBe("AK");
    expect(initials("sam")).toBe("S");
    expect(initials("jo.van.martin")).toBe("JM");
  });
  it("keeps an avatar tone stable per name", () => {
    expect(avatarTone("Alex Kim")).toBe(avatarTone("alex kim"));
    expect(avatarTone("Alex Kim")).toBeGreaterThanOrEqual(0);
    expect(avatarTone("Alex Kim")).toBeLessThan(6);
  });
  it("shortens in the middle", () => {
    expect(middleTruncate("feature/ABC-123-long-name", 15)).toBe(
      "feature…ng-name",
    );
    expect(middleTruncate("main", 15)).toBe("main");
  });
});

describe("versions and filters", () => {
  it("sorts versions newest first", () => {
    expect(
      ["v2.9.1", "nightly", "v2.10.0", "v1.0"].sort(compareVersionsDesc),
    ).toEqual(["v2.10.0", "v2.9.1", "v1.0", "nightly"]);
  });
  it("reads an author token out of the history filter", () => {
    expect(parseHistoryFilter("author:alex fix login")).toEqual({
      text: "fix login",
      author: "alex",
    });
    expect(parseHistoryFilter('fix author:"Alex Kim"')).toEqual({
      text: "fix",
      author: "Alex Kim",
    });
    expect(parseHistoryFilter("plain")).toEqual({
      text: "plain",
      author: null,
    });
  });
  it("splits a change bar and tells old from new", () => {
    expect(barWidths(3, 1)).toEqual({ add: 75, del: 25 });
    expect(barWidths(null, null)).toEqual({ add: 0, del: 0 });
    const now = Date.parse("2026-10-01T00:00:00Z");
    expect(olderThan("2026-06-01T00:00:00Z", 90, now)).toBe(true);
    expect(olderThan("2026-09-20T00:00:00Z", 90, now)).toBe(false);
  });
});

describe("relocation", () => {
  it("opens the picker in the old folder's parent", () => {
    expect(parentFolder("/work/code/api")).toBe("/work/code");
    expect(parentFolder("/work/code/api/")).toBe("/work/code");
    expect(parentFolder("/api")).toBe("/");
  });
  it("explains each concern", () => {
    const text = relocationQuestion("api", "/work/api-2", [
      "inside_repository",
      "unrelated_history",
    ]);
    expect(text.split("\n\n")).toHaveLength(3);
    expect(text).toContain("Point “api” at /work/api-2?");
    expect(text).toContain("looks like a different repository");
  });
  it("mentions what moved along", () => {
    expect(relocatedNotice("api", "/work/api-2", [])).toBe(
      "“api” now points to /work/api-2",
    );
    expect(relocatedNotice("app", "/work/app-2", ["a", "b"])).toBe(
      "“app” now points to /work/app-2, with 2 repositories inside it",
    );
  });
});

describe("branchStatus", () => {
  const now = Date.parse("2026-10-12T12:00:00Z");
  const repo = (over: Partial<RepositorySummary> = {}): RepositorySummary => ({
    id: "r1",
    name: "app",
    canonical_root: "/work/app",
    display_path: "/work/app",
    state: "fresh",
    last_checked_at: null,
    last_commit_at: null,
    head: { kind: "branch", branch: "feature/login", commit_id: "abc1234def" },
    counts: null,
    upstream: { ref: "origin/feature/login", ahead: 2, behind: 0 },
    error: null,
    last_tab: null,
    last_fetch_at: "2026-10-12T11:56:00Z",
    fetch_error: null,
    remote_url: null,
    forge: null,
    ...over,
  });

  it("hides an upstream of the same name and shows the fetch's age", () => {
    const s = branchStatus(repo(), false, now);
    expect(s.name).toBe("feature/login");
    expect(s.upstream).toBeNull();
    expect(s.ahead).toBe(2);
    expect(s.note).toEqual({ text: "4m", tone: "muted" });
  });

  it("names an upstream with another branch name", () => {
    const s = branchStatus(
      repo({ upstream: { ref: "origin/release/v1", ahead: 0, behind: 3 } }),
      false,
      now,
    );
    expect(s.upstream).toBe("origin/release/v1");
    expect(s.behind).toBe(3);
  });

  it("warns when the last fetch is stale", () => {
    const s = branchStatus(
      repo({ last_fetch_at: "2026-10-09T12:00:00Z" }),
      false,
      now,
    );
    expect(s.note).toEqual({ text: "fetched 3d", tone: "warn" });
  });

  it("puts fetching and a failed fetch before the age", () => {
    expect(branchStatus(repo(), true, now).note?.text).toBe("fetching…");
    const failed = (code: "UNAUTHENTICATED" | "TIMEOUT" | "IO") =>
      branchStatus(
        repo({
          fetch_error: { code, message: "x", retryable: true, details: null },
        }),
        false,
        now,
      ).note;
    expect(failed("UNAUTHENTICATED")).toEqual({
      text: "needs sign-in",
      tone: "error",
    });
    expect(failed("TIMEOUT")?.text).toBe("fetch timed out");
    expect(failed("IO")?.text).toBe("fetch failed");
  });

  it("says when there is no upstream, and names a detached HEAD by its commit", () => {
    expect(branchStatus(repo({ upstream: null }), false, now).note).toEqual({
      text: "no upstream",
      tone: "muted",
    });
    const detached = branchStatus(
      repo({
        head: { kind: "detached", branch: null, commit_id: "abc1234def" },
        upstream: null,
      }),
      false,
      now,
    );
    expect(detached).toMatchObject({
      name: "detached at abc1234",
      detached: true,
      upstream: null,
    });
  });
});
