import { useMemo, useRef, useState } from "react";
import { relativeTime } from "../lib/format";
import type { ChangeEntry, ChangesResult, DiffSelector } from "../lib/ipc";
import { step, useKeys } from "../lib/keys";
import { usePref } from "../lib/prefs";
import {
  barWidths,
  KIND_LETTER,
  kindTone,
  middleTruncate,
  plural,
  splitPath,
} from "../lib/repo";
import { useDiff } from "../lib/useDiff";
import DiffView from "./DiffView";
import { ChevronDown, ChevronRight, SearchIcon } from "./icons";

type Props = {
  repositoryId: string;
  /** Reloaded by the parent after backend change events. */
  changes: ChangesResult | null;
  /** The parent is reloading `changes`; the list stays and shows a progress line. */
  refreshing: boolean;
  onError: (message: string | null) => void;
  onOpenInEditor: (path: string, line?: number) => void;
};

const GROUPS: Array<{
  key: ChangeEntry["group"];
  label: string;
  hint: string;
}> = [
  { key: "conflicted", label: "Conflicted", hint: "resolve outside Brainiac" },
  { key: "staged", label: "Staged", hint: "HEAD → index" },
  { key: "unstaged", label: "Unstaged", hint: "index → working tree" },
  { key: "untracked", label: "Untracked", hint: "preview only" },
];

/** Characters of a folder path shown before it is shortened in the middle. */
const FOLDER_BUDGET = 44;

export function selectorFor(entry: ChangeEntry): DiffSelector {
  switch (entry.group) {
    case "staged":
      return { kind: "index_vs_head", path: entry.path };
    case "untracked":
      return { kind: "untracked_preview", path: entry.path };
    default:
      return { kind: "worktree_vs_index", path: entry.path };
  }
}

export function entryKey(e: ChangeEntry): string {
  return `${e.group}:${e.path}`;
}

/** Selection key of the combined HEAD → working tree comparison of a path. */
const bothKey = (path: string) => `both:${path}`;

export default function ChangesTab({
  repositoryId,
  changes,
  refreshing,
  onError,
  onOpenInEditor,
}: Props) {
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [folded, setFolded] = usePref<string[]>("brainiac.changes.folded", []);
  const filterRef = useRef<HTMLInputElement>(null);

  const all = changes?.entries ?? [];
  const entries = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return q ? all.filter((e) => e.path.toLowerCase().includes(q)) : all;
  }, [all, filter]);

  // Display order of the open groups, for the keyboard, the stepper, and the default selection.
  const ordered = useMemo(
    () =>
      GROUPS.filter((g) => !folded.includes(g.key)).flatMap((g) =>
        entries.filter((e) => e.group === g.key),
      ),
    [entries, folded],
  );

  const inBoth = (path: string) =>
    all.some((e) => e.path === path && e.group === "staged") &&
    all.some((e) => e.path === path && e.group === "unstaged");
  const bothPath =
    selectedKey?.startsWith("both:") && inBoth(selectedKey.slice(5))
      ? selectedKey.slice(5)
      : null;
  // Keep the selection while the file is still changed; otherwise pick the first entry.
  const selected = bothPath
    ? (all.find((e) => e.path === bothPath && e.group === "unstaged") ?? null)
    : (ordered.find((e) => entryKey(e) === selectedKey) ??
      entries.find((e) => entryKey(e) === selectedKey) ??
      ordered[0] ??
      null);
  const selector: DiffSelector | null = bothPath
    ? { kind: "worktree_vs_head", path: bothPath }
    : selected
      ? selectorFor(selected)
      : null;

  // A reload of `changes` after a backend event revalidates the shown diff,
  // even when the selected path is unchanged. The old patch stays visible.
  const {
    diff,
    loading: diffLoading,
    ignoreWhitespace,
    setIgnoreWhitespace,
  } = useDiff(repositoryId, selector, changes, onError);

  const select = (e: ChangeEntry | null) => e && setSelectedKey(entryKey(e));
  const move = (delta: number) => select(step(ordered, selected, delta));
  useKeys({
    j: () => move(1),
    k: () => move(-1),
    ArrowDown: () => move(1),
    ArrowUp: () => move(-1),
    "/": () => filterRef.current?.focus(),
  });

  const toggleGroup = (key: string) =>
    setFolded(
      folded.includes(key) ? folded.filter((k) => k !== key) : [...folded, key],
    );

  const uniquePaths = new Set(all.map((e) => e.path)).size;
  const index = selected ? ordered.indexOf(selected) : -1;
  const canBoth = changes?.head.kind !== "unborn";

  return (
    <div className="flex min-h-0 flex-1">
      <section
        aria-label="Changed files"
        className="relative flex w-[360px] shrink-0 flex-col border-r bg-panel"
      >
        {refreshing && changes && <div className="progress-line" />}
        <div className="flex flex-col gap-2 border-b px-3 py-2.5">
          <div className="flex items-baseline gap-2">
            <span className="font-semibold">Working tree</span>
            {changes && (
              <span className="text-[12px] text-muted">
                {plural(uniquePaths, "path")} ·{" "}
                {plural(all.length, "entry", "entries")}
              </span>
            )}
            <span className="flex-1" />
            {changes && (
              <span className="text-[11.5px] text-muted">
                observed {relativeTime(changes.observed_at)}
              </span>
            )}
          </div>
          <label className="search">
            <SearchIcon />
            <input
              ref={filterRef}
              type="text"
              placeholder="Filter changed paths"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") e.currentTarget.blur();
              }}
            />
            <span className="kbd">/</span>
          </label>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto pb-2">
          {!changes && <ListSkeleton />}
          {changes && all.length === 0 && (
            <div className="p-4 text-muted">Working tree clean.</div>
          )}
          {changes && all.length > 0 && entries.length === 0 && (
            <div className="p-4 text-muted">No changed paths match.</div>
          )}
          {GROUPS.map((g) => {
            const list = entries.filter((e) => e.group === g.key);
            if (list.length === 0) return null;
            const open = !folded.includes(g.key);
            const adds = list.reduce((n, e) => n + (e.additions ?? 0), 0);
            const dels = list.reduce((n, e) => n + (e.deletions ?? 0), 0);
            return (
              <div key={g.key}>
                <button
                  type="button"
                  className="fold px-3 pt-3 pb-1"
                  aria-expanded={open}
                  onClick={() => toggleGroup(g.key)}
                >
                  {open ? (
                    <ChevronDown size={10} className="text-muted" />
                  ) : (
                    <ChevronRight size={10} className="text-muted" />
                  )}
                  <span className="section-label text-fg-2">{g.label}</span>
                  <span className="text-[11px] text-muted">{list.length}</span>
                  {(adds > 0 || dels > 0) && (
                    <span className="text-[11px]">
                      <span className="text-add">+{adds}</span>{" "}
                      <span className="text-del">−{dels}</span>
                    </span>
                  )}
                  <span className="flex-1" />
                  <span className="text-[11px] text-muted">{g.hint}</span>
                </button>
                {open &&
                  list.map((e) => (
                    <EntryRow
                      key={entryKey(e)}
                      entry={e}
                      selected={
                        bothPath
                          ? e.path === bothPath && e.group !== "untracked"
                          : selected === e
                      }
                      alsoIn={
                        all.find(
                          (o) =>
                            o.path === e.path &&
                            o.group !== e.group &&
                            o.group !== "untracked",
                        )?.group
                      }
                      onSelect={() => select(e)}
                    />
                  ))}
              </div>
            );
          })}
        </div>
      </section>
      <section aria-label="Diff" className="flex min-w-0 flex-1 flex-col">
        <DiffView
          diff={selected ? diff : null}
          loading={diffLoading}
          empty={
            !changes
              ? "Loading…"
              : all.length === 0
                ? "Nothing to compare: the working tree matches HEAD."
                : "Select a file to see its diff."
          }
          onOpenInEditor={onOpenInEditor}
          ignoreWhitespace={ignoreWhitespace}
          onIgnoreWhitespace={setIgnoreWhitespace}
          stepper={
            selected && index >= 0 && ordered.length > 1
              ? {
                  index,
                  total: ordered.length,
                  onPrev: () => move(-1),
                  onNext: () => move(1),
                }
              : undefined
          }
          extra={
            selected && (
              <ComparisonRow
                selected={selected}
                both={!!bothPath}
                all={all}
                canBoth={canBoth}
                onSelect={select}
                onBoth={() => setSelectedKey(bothKey(selected.path))}
              />
            )
          }
        />
      </section>
    </div>
  );
}

function ListSkeleton() {
  return (
    <div className="flex flex-col gap-4 p-4" role="status" aria-label="Loading">
      {[62, 48, 75, 40].map((w) => (
        <div key={w} className="flex items-center gap-2.5">
          <div className="skeleton h-[18px] w-[18px]" />
          <div className="skeleton" style={{ width: `${w}%` }} />
        </div>
      ))}
    </div>
  );
}

function EntryRow({
  entry,
  selected,
  alsoIn,
  onSelect,
}: {
  entry: ChangeEntry;
  selected: boolean;
  alsoIn?: ChangeEntry["group"];
  onSelect: () => void;
}) {
  const { dir, name } = splitPath(entry.path);
  const from = entry.old_path ? splitPath(entry.old_path) : null;
  const folder = middleTruncate(dir, FOLDER_BUDGET);
  const where = from
    ? from.dir === dir
      ? `${folder || "repository root "}← ${from.name}`
      : `← ${middleTruncate(entry.old_path ?? "", FOLDER_BUDGET)}`
    : folder || "repository root";
  const bar = barWidths(entry.additions, entry.deletions);
  // Untracked and binary files have no line counts.
  const counted =
    (entry.additions ?? null) !== null || (entry.deletions ?? null) !== null;
  return (
    <button
      type="button"
      className="list-row h-11 items-center gap-2.5 pr-3.5 pl-[11px]"
      aria-current={selected}
      onClick={onSelect}
      title={entry.old_path ? `${entry.old_path} → ${entry.path}` : entry.path}
    >
      <span className="kind" data-tone={kindTone(entry.kind)}>
        {KIND_LETTER[entry.kind]}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-px">
        <span className="truncate font-medium">{name}</span>
        <span className="meta truncate text-[11.5px] text-muted">{where}</span>
      </span>
      {entry.is_submodule && <span className="tag-box">submodule</span>}
      {alsoIn && (
        <span className="shrink-0 rounded border border-control-line px-1.5 text-[10.5px] text-fg-3">
          also {alsoIn}
        </span>
      )}
      {counted && (
        <span className="flex shrink-0 flex-col items-end gap-1">
          <span className="tabular text-[11px]">
            <span className="text-add">+{entry.additions ?? 0}</span>{" "}
            <span className="text-del">−{entry.deletions ?? 0}</span>
          </span>
          <span className="change-bar" aria-hidden="true">
            <span className="a" style={{ width: `${bar.add}%` }} />
            <span className="d" style={{ width: `${bar.del}%` }} />
          </span>
        </span>
      )}
    </button>
  );
}

function counts(e: ChangeEntry | undefined) {
  if (!e || ((e.additions ?? null) === null && (e.deletions ?? null) === null))
    return null;
  return (
    <span className="tabular text-[11px] font-normal">
      <span className="text-add">+{e.additions ?? 0}</span>{" "}
      <span className="text-del">−{e.deletions ?? 0}</span>
    </span>
  );
}

/** Staged / Unstaged / Both switch for a file in both groups; a plain label otherwise. */
function ComparisonRow({
  selected,
  both,
  all,
  canBoth,
  onSelect,
  onBoth,
}: {
  selected: ChangeEntry;
  both: boolean;
  all: ChangeEntry[];
  canBoth: boolean;
  onSelect: (e: ChangeEntry) => void;
  onBoth: () => void;
}) {
  const staged = all.find(
    (e) => e.path === selected.path && e.group === "staged",
  );
  const unstaged = all.find(
    (e) => e.path === selected.path && e.group === "unstaged",
  );
  if (staged && unstaged) {
    return (
      <div role="tablist" aria-label="Comparison" className="seg seg-sm">
        <button
          type="button"
          role="tab"
          aria-selected={!both && selected === staged}
          title="HEAD → index"
          onClick={() => onSelect(staged)}
        >
          Staged {counts(staged)}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={!both && selected === unstaged}
          title="index → working tree"
          onClick={() => onSelect(unstaged)}
        >
          Unstaged {counts(unstaged)}
        </button>
        {canBoth && (
          <button
            type="button"
            role="tab"
            aria-selected={both}
            title="HEAD → working tree: staged and unstaged changes as one patch"
            onClick={onBoth}
          >
            Both
          </button>
        )}
      </div>
    );
  }
  const label = GROUPS.find((g) => g.key === selected.group);
  return (
    <div className="flex items-center gap-2 text-[12px] text-muted">
      <span className="font-medium text-fg-2">{label?.label}</span>
      <span className="mono">
        {selected.group === "untracked"
          ? "new file, not yet added to Git"
          : selected.group === "conflicted"
            ? "working tree, with conflict markers"
            : label?.hint}
      </span>
    </div>
  );
}
