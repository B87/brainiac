import { describe, expect, it } from "vitest";
import {
  checksLine,
  estimateText,
  forgetGroupQuestion,
  formatCost,
  formatDuration,
  groupConceptsByRepository,
  knownLeftOutLine,
  noteSequence,
  notesIn,
  pickRecord,
  readingOrder,
  sameSubject,
  sortConcepts,
  startsNotCovered,
  subjectLabel,
} from "./explain";
import type {
  Explanation,
  ExplanationNote,
  ExplanationRecord,
  KnownConcept,
} from "./ipc";

const note = (
  path: string,
  start: number,
  end: number,
  dropped = 0,
): ExplanationNote => ({
  path,
  start,
  end,
  text: `${path} ${start}`,
  sources: [],
  sources_dropped: dropped,
  lines_hash: "",
});

const explanation = (over: Partial<Explanation> = {}): Explanation => ({
  summary: "Why.",
  sources_read: [],
  tour: [
    { path: "src/rule.rs", role: "the rule" },
    { path: "src/fix.rs", role: "the fix" },
  ],
  notes: [note("src/fix.rs", 4, 6), note("src/rule.rs", 9, 9, 1)],
  concepts: [],
  questions: [],
  disagreements: [],
  checks: { moved: 2, left_out: ["x"] },
  known_left_out: [],
  ...over,
});

describe("explanations", () => {
  it("say cost and time in words", () => {
    expect(formatCost({ micros: 420_000, currency: "USD" })).toBe("$0.42");
    expect(formatCost({ micros: 42_000, currency: "USD" })).toBe("$0.042");
    expect(formatCost({ micros: 1_500_000, currency: "EUR" })).toBe("1.50 EUR");
    expect(formatDuration(45)).toBe("45 s");
    expect(formatDuration(130)).toBe("2 min");
    expect(formatDuration(3900)).toBe("1 h 5 min");
    expect(estimateText(undefined, "api_key")).toBe("No estimate yet");
    expect(
      estimateText(
        {
          profile_id: "p",
          depth: "brief",
          duration_secs: 120,
          cost: { micros: 400_000, currency: "USD" },
          from: 3,
        },
        "api_key",
      ),
    ).toBe("usually about 2 min, $0.40");
  });

  it("say what the checks did, only when they did something", () => {
    expect(checksLine(explanation())).toBe(
      "2 quotes found at other lines · 1 claim left out · 1 note with a source not found",
    );
    expect(
      checksLine(
        explanation({
          checks: { moved: 0, left_out: [] },
          notes: [note("a", 1, 1)],
        }),
      ),
    ).toBeNull();
  });

  it("count the known concepts left out, only when there were some", () => {
    expect(knownLeftOutLine(explanation())).toBeNull();
    expect(
      knownLeftOutLine(
        explanation({
          known_left_out: [
            { id: "1", name: "go:embed", kind: "language" },
            { id: "2", name: "DKIM", kind: "protocol" },
          ],
        }),
      ),
    ).toBe("Left out because you know them: 2");
  });

  it("order files and notes by the tour", () => {
    const files = [
      { path: "README.md" },
      { path: "src/fix.rs" },
      { path: "src/rule.rs" },
    ];
    expect(readingOrder(files, explanation()).map((f) => f.path)).toEqual([
      "src/rule.rs",
      "src/fix.rs",
      "README.md",
    ]);
    expect(readingOrder(files, null)).toBe(files);
    const seq = noteSequence(explanation(), null);
    expect(seq.map((n) => n.note.path)).toEqual(["src/rule.rs", "src/fix.rs"]);
  });

  it("place notes where a moved branch has them", () => {
    const e = explanation();
    const placed = notesIn(
      e,
      {
        explanation_id: "x",
        tip: "t",
        moved: true,
        gone: false,
        notes: [{ index: 0, start: 10, end: 12, out_of_date: false }],
        uncovered: [],
      },
      "src/fix.rs",
    );
    expect(placed).toEqual([
      {
        index: 0,
        note: e.notes[0],
        start: 10,
        end: 12,
        outOfDate: false,
      },
    ]);
  });

  it("show the working one, else the newest ready one", () => {
    const r = (id: string, state: ExplanationRecord["state"]) =>
      ({ id, state }) as ExplanationRecord;
    const records = [r("a", "failed"), r("b", "ready"), r("c", "working")];
    expect(pickRecord(records, null)?.id).toBe("c");
    expect(pickRecord(records, "a")?.id).toBe("a");
    expect(pickRecord([r("a", "failed"), r("b", "ready")], null)?.id).toBe("b");
    expect(pickRecord([], null)).toBeNull();
  });
});

describe("subjects", () => {
  it("names where a shared explanation was made", () => {
    expect(
      subjectLabel({ kind: "branch", reference: "refs/remotes/origin/feat/x" }),
    ).toBe("the branch origin/feat/x");
    expect(
      subjectLabel({
        kind: "pull_request",
        reference: "github.com/acme/api#42",
      }),
    ).toBe("the pull request #42");
  });

  it("tells a subject from another with the same changes", () => {
    const pr = { kind: "pull_request" as const, reference: "github.com/a/b#1" };
    expect(sameSubject(pr, { ...pr })).toBe(true);
    expect(sameSubject(pr, { kind: "branch", reference: "refs/heads/x" })).toBe(
      false,
    );
  });
});

describe("startsNotCovered", () => {
  const files = ["a", "b", "new1", "new2"].map((path) => ({ path }));
  const steps = new Map([
    ["a", 1],
    ["b", 2],
  ]);

  it("marks only the first file the tour does not list", () => {
    expect(files.map((_, i) => startsNotCovered(files, i, steps))).toEqual([
      false,
      false,
      true,
      false,
    ]);
  });

  it("has no heading in Path order", () => {
    expect(files.some((_, i) => startsNotCovered(files, i, new Map()))).toBe(
      false,
    );
  });
});

const concept = (over: Partial<KnownConcept>): KnownConcept => ({
  id: over.name ?? "c",
  kind: "language",
  name: "c",
  repository_id: null,
  repository_name: null,
  merged_into: null,
  learned_at: "2026-10-01T00:00:00Z",
  description: "",
  learned_from: "",
  learned_in: null,
  learned_in_name: null,
  explanation_id: null,
  ...over,
});

describe("Concepts You Know", () => {
  const list = [
    concept({
      name: "serde",
      kind: "library",
      learned_at: "2026-10-02T00:00:00Z",
    }),
    concept({
      name: "Async",
      kind: "language",
      learned_at: "2026-10-03T00:00:00Z",
    }),
    concept({
      name: "dkim",
      kind: "protocol",
      learned_at: "2026-10-01T00:00:00Z",
    }),
    concept({
      name: "borrow",
      kind: "language",
      learned_at: "2026-10-01T00:00:00Z",
    }),
  ];
  const names = (l: KnownConcept[]) => l.map((c) => c.name);

  it("sort by newest, name, or kind, never changing the list given", () => {
    const before = names(list);
    expect(names(sortConcepts(list, "newest"))).toEqual([
      "Async",
      "serde",
      "borrow",
      "dkim",
    ]);
    expect(names(sortConcepts(list, "name"))).toEqual([
      "Async",
      "borrow",
      "dkim",
      "serde",
    ]);
    // Kinds in the page's order; equal kinds by name.
    expect(names(sortConcepts(list, "kind"))).toEqual([
      "Async",
      "borrow",
      "serde",
      "dkim",
    ]);
    expect(names(list)).toEqual(before);
  });

  it("group by the repository a project pattern is in, else where it was learned", () => {
    const grouped = groupConceptsByRepository(
      [
        concept({
          name: "outbox table",
          kind: "project_pattern",
          repository_id: "r1",
          repository_name: "widgets",
          learned_in_name: "widgets",
        }),
        concept({ name: "go:embed", learned_in: "r2", learned_in_name: "api" }),
        concept({ name: "spf", kind: "protocol", learned_in_name: null }),
        concept({
          name: "tls",
          kind: "protocol",
          learned_in: "r2",
          learned_in_name: "api",
        }),
        concept({
          name: "old pattern",
          kind: "project_pattern",
          repository_id: "gone",
          repository_name: null,
        }),
      ],
      "name",
    );
    expect(grouped.map((g) => [g.label, names(g.concepts)])).toEqual([
      ["A removed repository", ["old pattern"]],
      ["api", ["go:embed", "tls"]],
      ["widgets", ["outbox table"]],
      ["Not from a tracked repository", ["spf"]],
    ]);
  });

  it("say what forgetting a repository's concepts also forgets", () => {
    const pattern = concept({
      name: "p",
      kind: "project_pattern",
      repository_id: "r",
    });
    const lang = concept({ name: "l" });
    expect(forgetGroupQuestion("widgets", [pattern])).toBe(
      "Forget 1 concept learned in widgets? Explanations will teach it again.",
    );
    expect(forgetGroupQuestion("widgets", [pattern, lang, lang])).toContain(
      "2 of them are known in every repository",
    );
    expect(forgetGroupQuestion("widgets", [lang])).toContain(
      "1 of it is known in every repository",
    );
  });
});
