/**
 * Explaining changes (SPEC.md, section 14): what the panel, the dialog, and
 * the patch say about an explanation, as pure functions.
 */
import type {
  AgentPayment,
  ExplainCost,
  ExplainDepth,
  ExplainEstimate,
  ExplainStep,
  ExplainSubject,
  Explanation,
  ExplanationNote,
  ExplanationPlacement,
  ExplanationRecord,
  KnownConcept,
} from "./ipc";

export const DEPTHS: ExplainDepth[] = ["brief", "teach_me", "deep"];

export function depthLabel(depth: ExplainDepth): string {
  switch (depth) {
    case "brief":
      return "Brief";
    case "teach_me":
      return "Teach me";
    case "deep":
      return "Deep";
  }
}

export function stepLabel(step: ExplainStep | null): string {
  switch (step) {
    case "copying":
      return "Copying the subject";
    case "starting":
      return "Starting the agent";
    case "reading":
      return "Reading";
    case "checking":
      return "Checking";
    case "follow_up":
      return "Fixing what the checks found";
    default:
      return "Starting";
  }
}

/** "$0.42", or "0.42 EUR" for another currency. */
export function formatCost(cost: ExplainCost): string {
  const amount = cost.micros / 1_000_000;
  const digits = amount < 0.1 ? 3 : 2;
  return cost.currency === "USD"
    ? `$${amount.toFixed(digits)}`
    : `${amount.toFixed(digits)} ${cost.currency}`;
}

/** "45 s", "2 min", "1 h 5 min". */
export function formatDuration(secs: number): string {
  if (secs < 60) return `${secs} s`;
  const minutes = Math.round(secs / 60);
  if (minutes < 60) return `${minutes} min`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m ? `${h} h ${m} min` : `${h} h`;
}

/** The time and cost a finished or failed explanation took. */
export function usageText(
  record: Pick<ExplanationRecord, "duration_secs" | "cost" | "payment">,
): string | null {
  const parts: string[] = [];
  if (record.duration_secs !== null)
    parts.push(formatDuration(record.duration_secs));
  if (record.payment === "claude_plan") parts.push("Uses your Claude plan");
  else if (record.cost) parts.push(formatCost(record.cost));
  return parts.length ? parts.join(" · ") : null;
}

/** The dialog's line on what an explanation usually takes. */
export function estimateText(
  estimate: ExplainEstimate | undefined,
  payment: AgentPayment | undefined,
): string {
  if (!estimate) return "No estimate yet";
  const time = `usually about ${formatDuration(estimate.duration_secs)}`;
  if (payment === "claude_plan") return `${time} · Uses your Claude plan`;
  return estimate.cost ? `${time}, ${formatCost(estimate.cost)}` : time;
}

/** "2 quotes found at other lines · 1 claim left out · …", or null. */
export function checksLine(e: Explanation): string | null {
  const parts: string[] = [];
  const moved = e.checks.moved;
  if (moved)
    parts.push(
      `${moved} ${moved === 1 ? "quote" : "quotes"} found at other lines`,
    );
  const left = e.checks.left_out.length;
  if (left) parts.push(`${left} ${left === 1 ? "claim" : "claims"} left out`);
  const lost = e.notes.filter((n) => n.sources_dropped > 0).length;
  if (lost)
    parts.push(
      `${lost} ${lost === 1 ? "note" : "notes"} with a source not found`,
    );
  return parts.length ? parts.join(" · ") : null;
}

/** "Left out because you know them: 3", or null when none were. */
export function knownLeftOutLine(e: Explanation): string | null {
  const n = e.known_left_out.length;
  return n ? `Left out because you know them: ${n}` : null;
}

/** Paths in reading order: the tour's, then any file it leaves out, by path. */
export function readingOrder<T extends { path: string }>(
  files: T[],
  e: Explanation | null | undefined,
): T[] {
  if (!e) return files;
  const rank = new Map(e.tour.map((s, i) => [s.path, i]));
  return [...files].sort((a, b) => {
    const ra = rank.get(a.path) ?? Number.MAX_SAFE_INTEGER;
    const rb = rank.get(b.path) ?? Number.MAX_SAFE_INTEGER;
    return ra !== rb ? ra - rb : a.path.localeCompare(b.path);
  });
}

/**
 * Whether a reading-order list starts the files its tour does not list at
 * row `i`, where the "Not in this explanation" heading goes. `steps` holds
 * the toured files' steps; empty in Path order, which has no heading.
 */
export function startsNotCovered<T extends { path: string }>(
  files: T[],
  i: number,
  steps: Map<string, number>,
): boolean {
  if (steps.size === 0 || steps.has(files[i].path)) return false;
  return i === 0 || steps.has(files[i - 1].path);
}

export type PlacedNote = {
  index: number;
  note: ExplanationNote;
  start: number;
  end: number;
  outOfDate: boolean;
};

/** A file's notes where they sit now, in line order. */
export function notesIn(
  e: Explanation,
  placement: ExplanationPlacement | null,
  path: string,
): PlacedNote[] {
  const placed = new Map(placement?.notes.map((p) => [p.index, p]) ?? []);
  return e.notes
    .map((note, index) => {
      const p = placed.get(index);
      return {
        index,
        note,
        start: p?.start ?? note.start,
        end: p?.end ?? note.end,
        outOfDate: p?.out_of_date ?? false,
      };
    })
    .filter((n) => n.note.path === path)
    .sort((a, b) => a.end - b.end || a.index - b.index);
}

/** Every note in reading order, for Shift+N and Shift+P. */
export function noteSequence(
  e: Explanation,
  placement: ExplanationPlacement | null,
): PlacedNote[] {
  const order = readingOrder(
    [...new Set(e.notes.map((n) => n.path))].map((path) => ({ path })),
    e,
  );
  return order.flatMap(({ path }) => notesIn(e, placement, path));
}

export function sameSubject(a: ExplainSubject, b: ExplainSubject): boolean {
  return a.kind === b.kind && a.reference === b.reference;
}

/**
 * Where an explanation was made, for a branch and a pull request that share
 * one (SPEC.md, section 14, Pull requests): "the branch feature/x", "the
 * pull request #42".
 */
export function subjectLabel(subject: ExplainSubject): string {
  switch (subject.kind) {
    case "branch":
      return `the branch ${subject.reference.replace(/^refs\/(heads|remotes)\//, "")}`;
    case "pull_request":
      return `the pull request #${subject.reference.split("#").pop()}`;
    case "commit":
      return `commit ${subject.reference.slice(0, 7)}`;
    case "run":
      return "a run's result";
  }
}

/** The one shown by default: the newest ready or working, else the newest. */
export function pickRecord(
  records: ExplanationRecord[],
  chosen: string | null,
): ExplanationRecord | null {
  return (
    records.find((r) => r.id === chosen) ??
    records.find((r) => r.state === "working") ??
    records.find((r) => r.state === "ready") ??
    records[0] ??
    null
  );
}

/** How Concepts You Know orders a list (SPEC.md, section 14). */
export type ConceptSort = "newest" | "name" | "kind";

/** The kinds in the order Concepts You Know lists them. */
export const CONCEPT_KINDS: KnownConcept["kind"][] = [
  "language",
  "library",
  "protocol",
  "tool",
  "technique",
  "project_pattern",
];

/** Where a concept belongs when a list is grouped by repository: a project
 * pattern's own repository, else the repository it was learned in. */
export const NO_REPOSITORY = "Not from a tracked repository";
export function conceptRepository(c: KnownConcept): string {
  if (c.repository_id) return c.repository_name ?? "A removed repository";
  return c.learned_in_name ?? NO_REPOSITORY;
}

/** A copy of the concepts in the chosen order; ties fall back to the name. */
export function sortConcepts(
  concepts: KnownConcept[],
  by: ConceptSort,
): KnownConcept[] {
  const byName = (a: KnownConcept, b: KnownConcept) =>
    a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  return [...concepts].sort((a, b) => {
    if (by === "name") return byName(a, b);
    if (by === "kind") {
      const kinds =
        CONCEPT_KINDS.indexOf(a.kind) - CONCEPT_KINDS.indexOf(b.kind);
      return kinds || byName(a, b);
    }
    // RFC 3339 times of one zone sort as text; newest first.
    return b.learned_at.localeCompare(a.learned_at) || byName(a, b);
  });
}

/** The concepts by repository, each group in the chosen order. Groups are
 * ordered by name, the concepts of no tracked repository last. */
export function groupConceptsByRepository(
  concepts: KnownConcept[],
  by: ConceptSort,
): { label: string; concepts: KnownConcept[] }[] {
  const groups = new Map<string, KnownConcept[]>();
  for (const c of sortConcepts(concepts, by)) {
    const label = conceptRepository(c);
    groups.set(label, [...(groups.get(label) ?? []), c]);
  }
  return [...groups.entries()]
    .map(([label, concepts]) => ({ label, concepts }))
    .sort((a, b) =>
      a.label === NO_REPOSITORY
        ? 1
        : b.label === NO_REPOSITORY
          ? -1
          : a.label.localeCompare(b.label),
    );
}

/** What Forget asks before it forgets a repository's concepts: the ones that
 * are known in every repository go too, and are explained again everywhere. */
export function forgetGroupQuestion(
  label: string,
  concepts: KnownConcept[],
): string {
  const everywhere = concepts.filter((c) => !c.repository_id).length;
  const n = concepts.length;
  const head = `Forget ${n} ${n === 1 ? "concept" : "concepts"} learned in ${label}?`;
  return everywhere
    ? `${head} ${everywhere} of ${n === 1 ? "it is" : "them are"} known in every repository (languages, libraries, protocols, tools, and techniques), so explanations will teach ${everywhere === 1 ? "it" : "them"} again everywhere.`
    : `${head} Explanations will teach ${n === 1 ? "it" : "them"} again.`;
}
