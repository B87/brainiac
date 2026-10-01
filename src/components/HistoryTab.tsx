import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { shortAgo } from "../lib/format";
import {
  type CommitPage,
  type CommitSummary,
  errorMessage,
  ipc,
  type RefEntry,
  type RefsResult,
} from "../lib/ipc";
import { step, useKeys } from "../lib/keys";
import {
  avatarTone,
  decorations,
  groupByDay,
  initials,
  parseHistoryFilter,
} from "../lib/repo";
import { createLatest } from "../lib/stale";
import CommitDetails from "./CommitDetails";
import { BranchIcon, ChevronDown, SearchIcon, TagIcon } from "./icons";
import Popover from "./Popover";

/** The branch or tag whose history is shown; a `RefEntry` fits. */
export type HistoryScope = Pick<RefEntry, "full_name" | "name" | "kind">;

type Props = {
  repositoryId: string;
  /** Label for HEAD in the ref picker, such as the current branch. */
  headName: string;
  /** Branch or tag shown instead of HEAD; null means HEAD. */
  historyRef: HistoryScope | null;
  onHistoryRef: (ref: HistoryScope | null) => void;
  /** Commit to select once it is loaded, such as the tip of an activity event. */
  initialCommitId?: string | null;
  refs: RefsResult | null;
  onNeedRefs: () => void;
  /** Increments when the backend reports this repository changed. */
  changeTick: number;
  onError: (message: string | null) => void;
  onOpenInEditor: (path: string, line?: number) => void;
};

export default function HistoryTab({
  repositoryId,
  headName,
  historyRef,
  onHistoryRef,
  initialCommitId = null,
  refs,
  onNeedRefs,
  changeTick,
  onError,
  onOpenInEditor,
}: Props) {
  const [pages, setPages] = useState<CommitPage[]>([]);
  const [filter, setFilter] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(initialCommitId);
  const [loading, setLoading] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const latest = useRef(createLatest()).current;
  const filterRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const page = useCallback(
    (cursor: string | null, limit: number | null) => {
      const query = parseHistoryFilter(filter);
      return ipc.listCommits({
        repository_id: repositoryId,
        ref: historyRef?.full_name ?? null,
        filter: query.text || null,
        author: query.author,
        cursor,
        limit,
      });
    },
    [repositoryId, filter, historyRef],
  );

  /** Load from the top until at least `target` commits are in, then show them at once. */
  const reload = useCallback(
    (target: number) => {
      setLoading(true);
      void latest.run(
        async () => {
          const loadedPages: CommitPage[] = [];
          let cursor: string | null = null;
          let count = 0;
          do {
            const next: CommitPage = await page(
              cursor,
              Math.min(500, Math.max(100, target - count)),
            );
            loadedPages.push(next);
            count += next.items.length;
            cursor = next.next_cursor;
          } while (cursor && count < target);
          return loadedPages;
        },
        (loadedPages) => {
          if (loadedPages[0]?.repository_id !== repositoryId) return;
          setPages(loadedPages);
          setLoading(false);
        },
        (e) => {
          setLoading(false);
          onError(errorMessage(e));
        },
      );
    },
    [repositoryId, page, latest, onError],
  );

  const loadMore = (cursor: string) => {
    setLoading(true);
    void latest.run(
      () => page(cursor, null),
      (next) => {
        if (next.repository_id !== repositoryId) return;
        setPages((prev) => [...prev, next]);
        setLoading(false);
      },
      (e) => {
        setLoading(false);
        onError(errorMessage(e));
      },
    );
  };

  // A refresh after a backend event reloads as many commits as are shown,
  // page by page, so the list keeps its length and selection. A different
  // filter or ref starts over with one page.
  const loaded = useRef({ scope: "", count: 0 });
  const scope = `${historyRef?.full_name ?? "HEAD"}\u0000${filter}`;
  // biome-ignore lint/correctness/useExhaustiveDependencies: changeTick refreshes history after backend events; scope is read through the ref.
  useEffect(() => {
    const same = loaded.current.scope === scope;
    loaded.current.scope = scope;
    const target = same ? Math.max(100, loaded.current.count) : 100;
    const t = setTimeout(() => reload(target), filter ? 300 : 0);
    return () => clearTimeout(t);
  }, [reload, changeTick]);

  const commits = useMemo(() => pages.flatMap((p) => p.items), [pages]);
  loaded.current.count = commits.length;
  const groups = useMemo(() => groupByDay(commits), [commits]);
  const nextCursor = pages.length ? pages[pages.length - 1].next_cursor : null;
  const anchor = pages[0]?.anchor_commit_id ?? null;
  const selected =
    commits.find((c) => c.id === selectedId) ?? commits[0] ?? null;
  const scopeName = historyRef ? historyRef.name : `HEAD · ${headName}`;

  const move = (delta: number) => {
    const next = step(commits, selected, delta);
    if (!next) return;
    setSelectedId(next.id);
    listRef.current
      ?.querySelector(`[data-commit="${next.id}"]`)
      ?.scrollIntoView({ block: "nearest" });
  };
  useKeys({
    j: () => move(1),
    k: () => move(-1),
    ArrowDown: () => move(1),
    ArrowUp: () => move(-1),
    "/": () => filterRef.current?.focus(),
  });

  const pick = (ref: HistoryScope | null) => {
    setPickerOpen(false);
    setSelectedId(null);
    setPages([]);
    onHistoryRef(ref);
  };

  return (
    <div className="flex min-h-0 flex-1">
      <section
        aria-label="Commits"
        className="flex w-[360px] shrink-0 flex-col border-r bg-panel"
      >
        <div className="flex flex-col gap-2 border-b px-3 py-2.5">
          <div className="flex items-center gap-2">
            <div className="relative min-w-0">
              <button
                type="button"
                className="btn btn-sm max-w-[220px] bg-control text-fg"
                aria-haspopup="menu"
                aria-expanded={pickerOpen}
                onClick={() => {
                  if (!pickerOpen) onNeedRefs();
                  setPickerOpen(!pickerOpen);
                }}
              >
                {historyRef?.kind === "tag" ? (
                  <TagIcon size={12} />
                ) : (
                  <BranchIcon size={12} />
                )}
                <span className="mono truncate">{scopeName}</span>
                <ChevronDown size={10} className="text-muted" />
              </button>
              {pickerOpen && (
                <Popover onClose={() => setPickerOpen(false)}>
                  <RefMenu
                    refs={refs}
                    headName={headName}
                    active={historyRef?.full_name ?? null}
                    onPick={pick}
                  />
                </Popover>
              )}
            </div>
            <span className="text-[12px] text-muted">
              {anchor === ""
                ? "no commits yet"
                : `${commits.length}${nextCursor ? "+" : ""} commits`}
            </span>
          </div>
          <label className="search" title="Add author:name to filter by author">
            <SearchIcon />
            <input
              ref={filterRef}
              type="text"
              placeholder="Message, hash, or author:name"
              value={filter}
              onChange={(e) => {
                setFilter(e.target.value);
                setSelectedId(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Escape") e.currentTarget.blur();
              }}
            />
            <span className="kbd">/</span>
          </label>
        </div>
        <div ref={listRef} className="relative min-h-0 flex-1 overflow-y-auto">
          {loading && pages.length > 0 && <div className="progress-line" />}
          {groups.map((g) => (
            <div key={g.label}>
              <div className="sticky top-0 z-[1] flex items-center gap-2 border-b border-line-soft bg-panel/95 px-3.5 pt-2 pb-1.5 backdrop-blur-sm">
                <span className="section-label">{g.label}</span>
                <span className="text-[11px] text-muted">
                  {g.commits.length === 1
                    ? "1 commit"
                    : `${g.commits.length} commits`}
                </span>
              </div>
              {g.commits.map((c) => (
                <CommitRow
                  key={c.id}
                  commit={c}
                  selected={c === selected}
                  onSelect={() => setSelectedId(c.id)}
                />
              ))}
            </div>
          ))}
          {loading && pages.length === 0 && <CommitSkeleton />}
          {!loading && anchor === "" && (
            <div className="p-4 text-muted">
              This repository has no commits yet.
            </div>
          )}
          {!loading && commits.length === 0 && !!anchor && (
            <div className="p-4 text-muted">
              No matching commits
              {parseHistoryFilter(filter).author ? " by that author" : ""}.
            </div>
          )}
          {!loading && nextCursor && (
            <div className="p-3">
              <button
                type="button"
                className="btn btn-sm w-full"
                onClick={() => loadMore(nextCursor)}
              >
                Load more
              </button>
            </div>
          )}
        </div>
      </section>
      <section
        aria-label="Commit details"
        className="flex min-w-0 flex-1 flex-col"
      >
        {selected ? (
          <CommitDetails
            key={selected.id}
            repositoryId={repositoryId}
            commit={selected}
            onError={onError}
            onOpenInEditor={onOpenInEditor}
            onSelectCommit={(id) => {
              if (commits.some((c) => c.id === id)) setSelectedId(id);
            }}
          />
        ) : (
          <div className="p-6 text-muted">
            {loading ? "" : "Select a commit to see its details."}
          </div>
        )}
      </section>
    </div>
  );
}

function CommitSkeleton() {
  return (
    <div
      className="flex flex-col gap-5 p-4"
      role="status"
      aria-label="Loading commits"
    >
      {[80, 62, 71, 54, 66].map((w) => (
        <div key={w} className="flex gap-2.5">
          <div className="skeleton h-[22px] w-[22px] rounded-full" />
          <div className="flex flex-1 flex-col gap-2">
            <div className="skeleton" style={{ width: `${w}%` }} />
            <div className="skeleton h-2 w-1/3" />
          </div>
        </div>
      ))}
    </div>
  );
}

export function Avatar({ name, size = 22 }: { name: string; size?: number }) {
  return (
    <span
      className="avatar"
      data-tone={avatarTone(name)}
      style={size === 22 ? undefined : { width: size, height: size }}
      aria-hidden="true"
    >
      {initials(name)}
    </span>
  );
}

function CommitRow({
  commit: c,
  selected,
  onSelect,
}: {
  commit: CommitSummary;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      data-commit={c.id}
      className="list-row gap-2.5 pt-2 pr-3.5 pb-[9px] pl-[11px]"
      aria-current={selected}
      onClick={onSelect}
    >
      <span className="mt-px">
        <Avatar name={c.author_name} />
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="w-full truncate font-medium">{c.subject}</span>
        <span className="meta flex w-full min-w-0 items-center gap-1.5 text-[11.5px] text-muted">
          <span className="mono shrink-0">{c.short_id}</span>
          <span aria-hidden="true">·</span>
          <span className="truncate" title={c.author_email}>
            {c.author_name}
          </span>
          <span className="flex min-w-0 shrink-0 gap-1 overflow-hidden">
            {decorations(c).map((d) => (
              <span key={d.label} className="deco" data-tone={d.tone}>
                {d.label}
              </span>
            ))}
          </span>
          <span className="flex-1" />
          <span className="shrink-0">{shortAgo(c.committed_at)}</span>
        </span>
      </span>
    </button>
  );
}

function RefMenu({
  refs,
  headName,
  active,
  onPick,
}: {
  refs: RefsResult | null;
  headName: string;
  active: string | null;
  onPick: (ref: HistoryScope | null) => void;
}) {
  const local = refs?.refs.filter((r) => r.kind === "local_branch") ?? [];
  const tags = refs?.refs.filter((r) => r.kind === "tag") ?? [];
  const remote = refs?.refs.filter((r) => r.kind === "remote_branch") ?? [];
  const sections: Array<[string, RefEntry[]]> = [
    ["Local branches", local],
    ["Tags", tags],
    ["Remote-tracking", remote],
  ];
  const item = (r: RefEntry) => (
    <button
      type="button"
      role="menuitemradio"
      aria-checked={active === r.full_name}
      key={r.full_name}
      className="menu-item"
      onClick={() => onPick(r)}
    >
      {r.kind === "tag" ? <TagIcon size={12} /> : <BranchIcon size={12} />}
      <span className="mono truncate">{r.name}</span>
    </button>
  );
  return (
    <div className="w-[260px]">
      <button
        type="button"
        role="menuitemradio"
        aria-checked={active === null}
        className="menu-item"
        onClick={() => onPick(null)}
      >
        <BranchIcon size={12} />
        <span className="mono">HEAD</span>
        <span className="muted-in-menu truncate text-muted">{headName}</span>
      </button>
      {!refs && <div className="px-2 py-1 text-muted">Loading refs…</div>}
      {sections.map(([label, list]) =>
        list.length ? (
          <div key={label}>
            <div className="menu-sep" />
            <div className="section-label px-2 py-1">{label}</div>
            {list.map(item)}
          </div>
        ) : null,
      )}
    </div>
  );
}
