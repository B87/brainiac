import { ask } from "@tauri-apps/plugin-dialog";
import { type ReactNode, useMemo, useState } from "react";
import { depthLabel, formatCost } from "../lib/explain";
import {
  type ConceptKind,
  type ExplainCost,
  type ExplanationSummary,
  ipc,
  type KnownConcept,
} from "../lib/ipc";
import { plural } from "../lib/repo";
import { requestOpenSubject } from "../lib/windowEvents";
import { ChevronDown, ChevronRight } from "./icons";
import Popover from "./Popover";
import { Hint } from "./SettingsPanes";

type Act = (what: () => Promise<unknown>) => Promise<void>;

const KIND_WORD: Record<ConceptKind, string> = {
  language: "Language",
  library: "Library",
  system: "System tool",
  project_pattern: "Project pattern",
};
const KINDS: ConceptKind[] = [
  "language",
  "library",
  "system",
  "project_pattern",
];
const KIND_CHIP: Record<ConceptKind, string> = {
  language: "Language",
  library: "Library",
  system: "System",
  project_pattern: "Project patterns",
};

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** "28 Sep", or "28 Sep 2025" outside this year. */
function shortDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: d.getFullYear() === new Date().getFullYear() ? undefined : "numeric",
  });
}

/** "2026-09" for grouping and the cost month. */
function monthKey(iso: string): string {
  return iso.slice(0, 7);
}

function monthLabel(key: string): string {
  const [y, m] = key.split("-").map(Number);
  return new Date(y, m - 1, 1).toLocaleDateString(undefined, {
    month: "long",
    year: "numeric",
  });
}

/** Costs added up per currency, as "$8.10" or "$8.10 + €1.20". */
function totalCost(items: ExplanationSummary[]): string {
  const sums: ExplainCost[] = [];
  for (const c of items.flatMap((e) => (e.cost ? [e.cost] : []))) {
    const same = sums.find((s) => s.currency === c.currency);
    if (same) same.micros += c.micros;
    else sums.push({ ...c });
  }
  return sums.map(formatCost).join(" + ");
}

/** The bar under a list while rows are selected. */
function SelectionBar({ children }: { children: ReactNode }) {
  return (
    <div
      role="status"
      className="sticky bottom-3 z-10 mt-3 flex flex-wrap items-center gap-3 self-start rounded-lg border bg-header px-3 py-2 text-[12.5px] shadow-lg"
    >
      {children}
    </div>
  );
}

function toggled(set: Set<string>, id: string): Set<string> {
  const next = new Set(set);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

/**
 * Concepts you know (SPEC.md, section 14, Settings → Explanations): by kind,
 * newest first, with what each explanation said of it and where it was
 * learned. Select some to merge them into one or forget them.
 */
export function ConceptList({
  concepts,
  act,
}: {
  concepts: KnownConcept[];
  act: Act;
}) {
  const [filter, setFilter] = useState("");
  const [kind, setKind] = useState<ConceptKind | "all">("all");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [keepOpen, setKeepOpen] = useState(false);

  // A merged concept is shown inside the one it was merged into.
  const roots = concepts.filter((c) => !c.merged_into);
  const mergedInto = new Map<string, string[]>();
  for (const c of concepts)
    if (c.merged_into)
      mergedInto.set(c.merged_into, [
        ...(mergedInto.get(c.merged_into) ?? []),
        c.name,
      ]);
  const needle = filter.trim().toLowerCase();
  const shown = roots.filter(
    (c) =>
      (kind === "all" || c.kind === kind) &&
      (!needle ||
        c.name.toLowerCase().includes(needle) ||
        c.description.toLowerCase().includes(needle) ||
        (mergedInto.get(c.id) ?? []).some((n) =>
          n.toLowerCase().includes(needle),
        )),
  );
  const chosen = roots.filter((c) => selected.has(c.id));
  // One idea named two ways: same kind, and a project pattern in one repository.
  const mergeable =
    chosen.length > 1 &&
    chosen.every(
      (c) =>
        c.kind === chosen[0].kind &&
        c.repository_id === chosen[0].repository_id,
    );

  if (roots.length === 0)
    return (
      <div className="settings-group">
        <div className="settings-row text-muted">
          I know this on a concept in an explanation adds it here.
        </div>
      </div>
    );

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <input
          type="search"
          className="field w-56"
          placeholder="Filter concepts"
          aria-label="Filter concepts"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <fieldset className="m-0 flex flex-wrap gap-1 border-0 p-0">
          <legend className="sr-only">Kind</legend>
          {(["all", ...KINDS] as const).map((k) => {
            const count =
              k === "all"
                ? roots.length
                : roots.filter((c) => c.kind === k).length;
            if (k !== "all" && count === 0) return null;
            return (
              <button
                key={k}
                type="button"
                className="btn btn-sm rounded-full"
                aria-pressed={kind === k}
                onClick={() => setKind(k)}
              >
                {k === "all" ? "All" : KIND_CHIP[k]}{" "}
                <span className="text-muted">{count}</span>
              </button>
            );
          })}
        </fieldset>
      </div>
      {needle && (
        <Hint>
          {plural(shown.length, "concept")} match “{filter.trim()}” · newest
          first
        </Hint>
      )}
      <ul className="settings-group m-0 list-none p-0">
        {shown.map((c) => {
          const also = mergedInto.get(c.id) ?? [];
          const from = [
            c.learned_from && `from ${c.learned_from}`,
            c.learned_in_name,
            shortDate(c.learned_at),
          ]
            .filter(Boolean)
            .join(" · ");
          return (
            <li
              key={c.id}
              className="settings-row items-start"
              aria-current={selected.has(c.id) || undefined}
            >
              <input
                type="checkbox"
                className="mt-1"
                aria-label={`Select ${c.name}`}
                checked={selected.has(c.id)}
                onChange={() => setSelected(toggled(selected, c.id))}
              />
              <span className="flex min-w-55 flex-1 flex-col gap-0.5">
                <span>
                  <span className="font-semibold">{c.name}</span>
                  <span className="ml-2 text-[11.5px] text-muted">
                    {KIND_WORD[c.kind]}
                    {c.repository_id &&
                      ` · only in ${c.repository_name ?? "a removed repository"}`}
                  </span>
                </span>
                {c.description && (
                  <span className="text-[12.5px] text-fg-3">
                    {c.description}
                  </span>
                )}
                {also.length > 0 && <Hint>Also called {also.join(", ")}</Hint>}
              </span>
              <span className="shrink-0 text-[12px] text-muted">{from}</span>
            </li>
          );
        })}
        {shown.length === 0 && (
          <li className="settings-row text-muted">No concept matches.</li>
        )}
      </ul>
      <Hint>
        A project pattern belongs to its repository: one repository's patterns
        never reach another's explanations.
      </Hint>
      {selected.size > 0 && (
        <SelectionBar>
          <span>{selected.size} selected</span>
          <span className="relative">
            <button
              type="button"
              className="btn btn-sm"
              disabled={!mergeable}
              title={
                mergeable
                  ? "Keep one name; explanations treat the others as it"
                  : "Choose two or more of one kind (project patterns of one repository)"
              }
              aria-haspopup="menu"
              aria-expanded={keepOpen}
              onClick={() => setKeepOpen(!keepOpen)}
            >
              Merge into One…
            </button>
            {keepOpen && (
              <Popover onClose={() => setKeepOpen(false)}>
                {chosen.map((keep) => (
                  <button
                    key={keep.id}
                    type="button"
                    role="menuitem"
                    className="menu-item"
                    onClick={() => {
                      setKeepOpen(false);
                      setSelected(new Set());
                      void act(async () => {
                        for (const c of chosen)
                          if (c.id !== keep.id)
                            await ipc.mergeConcept(c.id, keep.id);
                      });
                    }}
                  >
                    Keep “{keep.name}”
                  </button>
                ))}
              </Popover>
            )}
          </span>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => {
              const ids = [...selected];
              setSelected(new Set());
              void act(async () => {
                for (const id of ids) await ipc.forgetConcept(id);
              });
            }}
          >
            Forget
          </button>
          <Hint>Forgotten concepts are explained again</Hint>
        </SelectionBar>
      )}
    </div>
  );
}

type Grouping = "repository" | "month";

/** The subject's short label: a commit's hash, a branch, "#42", or a run. */
function subjectShort(e: ExplanationSummary): string {
  const r = e.subject.reference;
  switch (e.subject.kind) {
    case "commit":
      return r.slice(0, 7);
    case "branch":
      return r.replace(/^refs\/(heads|remotes)\//, "");
    case "pull_request":
      return `#${r.split("#").pop() ?? ""}`;
    case "run":
      return `run ${r.slice(0, 8)}`;
  }
}

/**
 * Stored explanations (SPEC.md, section 14, Settings → Explanations): by
 * repository or month, folded, with their size and cost; select some to
 * delete them, or delete a repository's at once.
 */
export function StoredList({
  stored,
  totalBytes,
  act,
}: {
  stored: ExplanationSummary[];
  totalBytes: number;
  act: Act;
}) {
  const [filter, setFilter] = useState("");
  const [grouping, setGrouping] = useState<Grouping>("repository");
  const [open, setOpen] = useState<Set<string> | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const months = useMemo(
    () =>
      [...new Set(stored.map((e) => monthKey(e.created_at)))].sort().reverse(),
    [stored],
  );
  const [costMonth, setCostMonth] = useState<string>(() => months[0] ?? "all");

  const needle = filter.trim().toLowerCase();
  const shown = stored.filter(
    (e) =>
      !needle ||
      e.title.toLowerCase().includes(needle) ||
      subjectShort(e).toLowerCase().includes(needle) ||
      e.repository_name.toLowerCase().includes(needle),
  );
  const groups = new Map<string, ExplanationSummary[]>();
  for (const e of shown) {
    const key =
      grouping === "repository" ? e.repository_name : monthKey(e.created_at);
    groups.set(key, [...(groups.get(key) ?? []), e]);
  }
  const keys = [...groups.keys()].sort((a, b) =>
    grouping === "month" ? b.localeCompare(a) : a.localeCompare(b),
  );
  // The first group starts open; a filter opens every group it matches.
  const isOpen = (key: string) =>
    !!needle || (open ? open.has(key) : key === keys[0]);
  const inMonth = (items: ExplanationSummary[]) =>
    costMonth === "all"
      ? items
      : items.filter((e) => monthKey(e.created_at) === costMonth);
  const monthWord =
    costMonth === "all"
      ? "in all"
      : `in ${monthLabel(costMonth).split(" ")[0]}`;

  const remove = async (items: ExplanationSummary[], what: string) => {
    const deletable = items.filter((e) => e.state !== "working");
    if (!deletable.length) return;
    const sure = await ask(
      `Delete ${what}? Each goes with its run's conversation, how it was written.`,
      { title: "Delete Explanations", kind: "warning", okLabel: "Delete" },
    );
    if (!sure) return;
    setSelected(new Set());
    await act(async () => {
      for (const e of deletable) await ipc.deleteExplanation(e.id);
    });
  };

  if (stored.length === 0)
    return (
      <div className="settings-group">
        <div className="settings-row text-muted">
          No explanation is stored. Explain keeps each one here until you delete
          it.
        </div>
      </div>
    );

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <input
          type="search"
          className="field w-56"
          placeholder="Filter by commit, branch, or title"
          aria-label="Filter explanations"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <div role="tablist" aria-label="Group by" className="seg seg-sm">
          {(["repository", "month"] as const).map((g) => (
            <button
              key={g}
              type="button"
              role="tab"
              aria-selected={grouping === g}
              onClick={() => {
                setGrouping(g);
                setOpen(null);
              }}
            >
              {g === "repository" ? "Repository" : "Month"}
            </button>
          ))}
        </div>
        <label className="flex items-center gap-1.5 text-[12.5px] text-fg-2">
          Cost for
          <select
            className="field"
            value={costMonth}
            onChange={(e) => setCostMonth(e.target.value)}
          >
            {months.map((m) => (
              <option key={m} value={m}>
                {monthLabel(m)}
              </option>
            ))}
            <option value="all">All time</option>
          </select>
        </label>
        <span className="text-[12.5px] font-semibold">
          {totalCost(inMonth(stored)) || "no cost reported"}
        </span>
        <span className="flex-1" />
        <Hint>
          {plural(stored.length, "explanation")} · {bytes(totalBytes)}
        </Hint>
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => void remove(stored, "every stored explanation")}
        >
          Delete All…
        </button>
      </div>

      <div className="flex flex-col">
        {keys.map((key) => {
          const items = groups.get(key) ?? [];
          const expanded = isOpen(key);
          const label = grouping === "month" ? monthLabel(key) : key;
          const cost = totalCost(inMonth(items));
          const size = items.reduce((n, e) => n + e.size_bytes, 0);
          return (
            <section key={key} className="flex flex-col">
              <div className="flex flex-wrap items-center gap-2.5 border-b py-2">
                <button
                  type="button"
                  className="flex items-center gap-1 font-semibold"
                  aria-expanded={expanded}
                  onClick={() => {
                    const base = open ?? new Set(keys.slice(0, 1));
                    setOpen(toggled(base, key));
                  }}
                >
                  {expanded ? (
                    <ChevronDown size={10} />
                  ) : (
                    <ChevronRight size={10} />
                  )}
                  {label}
                </button>
                <span className="flex-1 text-[12px] text-muted">
                  {items.length} · {bytes(size)}
                  {cost && grouping === "repository"
                    ? ` · ${cost} ${monthWord}`
                    : ""}
                  {cost && grouping === "month" ? ` · ${cost}` : ""}
                </span>
                {grouping === "repository" && (
                  <button
                    type="button"
                    className="text-[12px] text-conflict hover:underline"
                    onClick={() =>
                      void remove(
                        items,
                        `${label}'s ${plural(items.length, "explanation")}`,
                      )
                    }
                  >
                    Delete {label}'s {items.length}…
                  </button>
                )}
              </div>
              {expanded && (
                <ul className="m-0 list-none p-0 py-1">
                  {items.map((e) => (
                    <li
                      key={e.id}
                      className="flex flex-wrap items-center gap-2.5 rounded-md px-2.5 py-1.5"
                      style={
                        selected.has(e.id)
                          ? { background: "var(--sel)" }
                          : undefined
                      }
                    >
                      <input
                        type="checkbox"
                        aria-label={`Select ${e.title}`}
                        disabled={e.state === "working"}
                        checked={selected.has(e.id)}
                        onChange={() => setSelected(toggled(selected, e.id))}
                      />
                      <span className="mono text-[12px] text-muted">
                        {subjectShort(e)}
                      </span>
                      <span className="min-w-0 flex-1 truncate" title={e.title}>
                        {grouping === "month" && (
                          <span className="text-muted">
                            {e.repository_name} ·{" "}
                          </span>
                        )}
                        {e.title}
                        {e.state !== "ready" && (
                          <span className="ml-2 text-[11.5px] text-muted">
                            {e.state === "working" ? "being written" : e.state}
                          </span>
                        )}
                      </span>
                      <span className="text-[12px] text-muted">
                        {depthLabel(e.depth)} · {e.model || e.agent}
                        {e.cost ? ` · ${formatCost(e.cost)}` : ""} ·{" "}
                        {shortDate(e.created_at)}
                      </span>
                      {e.subject.kind !== "branch" ? (
                        <button
                          type="button"
                          className="text-[12px] text-link hover:underline"
                          onClick={() =>
                            requestOpenSubject({
                              repositoryId: e.repository_id,
                              kind: e.subject.kind as
                                | "commit"
                                | "pull_request"
                                | "run",
                              reference: e.subject.reference,
                            })
                          }
                        >
                          Open
                        </button>
                      ) : (
                        <span
                          className="text-[12px] text-muted"
                          title="Open it from Branches & tags → Changes against the default branch"
                        >
                          Branch
                        </span>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </section>
          );
        })}
        {keys.length === 0 && (
          <span className="py-2 text-muted">No explanation matches.</span>
        )}
      </div>

      {selected.size > 0 && (
        <SelectionBar>
          <span>{selected.size} selected</span>
          <button
            type="button"
            className="btn btn-sm text-conflict"
            onClick={() =>
              void remove(
                stored.filter((e) => selected.has(e.id)),
                plural(selected.size, "explanation"),
              )
            }
          >
            Delete…
          </button>
          <Hint>Each goes with how it was written</Hint>
        </SelectionBar>
      )}
    </div>
  );
}
