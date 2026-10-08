import { useEffect, useMemo, useRef, useState } from "react";
import { relativeTime, shortAgo } from "../lib/format";
import {
  type CommitSummary,
  errorMessage,
  ipc,
  type RefEntry,
  type RefsResult,
} from "../lib/ipc";
import { step, useKeys } from "../lib/keys";
import { usePref } from "../lib/prefs";
import {
  compareVersionsDesc,
  middleTruncate,
  olderThan,
  plural,
} from "../lib/repo";
import { useSidePanel } from "../lib/sidePanel";
import { createLatest } from "../lib/stale";
import BranchComparison from "./BranchComparison";
import {
  BranchIcon,
  ChevronDown,
  ChevronRight,
  SearchIcon,
  TagIcon,
} from "./icons";

type Props = {
  repositoryId: string;
  refs: RefsResult | null;
  onShowHistory: (ref: RefEntry) => void;
  onError: (message: string | null) => void;
};

type Sort = "recent" | "name";

/** A foldable list section. */
type Section = { key: string; label: string; refs: RefEntry[] };

/** Commits shown in the preview timeline. */
const PREVIEW_LIMIT = 30;
/** Local branches whose tip is older than this fold into their own group. */
const STALE_DAYS = 90;
/** Characters of a ref name shown before it is shortened in the middle. */
const NAME_BUDGET = 38;

const COLUMNS =
  "grid grid-cols-[minmax(140px,1.1fr)_minmax(100px,0.7fr)_92px_minmax(120px,1.2fr)_52px] gap-x-4";

export function upstreamText(r: RefEntry): { text: string; tone: string } {
  if (r.kind !== "local_branch") return { text: "—", tone: "text-faint" };
  if (!r.upstream) return { text: "no upstream", tone: "text-muted" };
  if (r.ahead === null || r.behind === null)
    return { text: `${r.upstream} · gone`, tone: "text-muted" };
  if (r.ahead === 0 && r.behind === 0)
    return { text: `${r.upstream} · in sync`, tone: "text-clean" };
  const parts = [r.ahead ? `↑${r.ahead}` : "", r.behind ? `↓${r.behind}` : ""];
  return {
    text: `${r.upstream} · ${parts.filter(Boolean).join(" ")}`,
    tone: r.behind ? "text-dirty" : "text-link",
  };
}

/** "↑3 ↓1" against the default branch, "same" when both are zero. */
export function baseText(r: RefEntry): string | null {
  if (r.base_ahead === null || r.base_behind === null) return null;
  if (!r.base_ahead && !r.base_behind) return "same";
  return [
    r.base_ahead ? `↑${r.base_ahead}` : "",
    r.base_behind ? `↓${r.base_behind}` : "",
  ]
    .filter(Boolean)
    .join(" ");
}

/** Group refs into sections: active local branches, stale ones, each remote, and tags. */
export function sections(
  refs: RefEntry[],
  sort: Sort,
  now = Date.now(),
): Section[] {
  const byRecent = (a: RefEntry, b: RefEntry) =>
    (b.committed_at ?? "").localeCompare(a.committed_at ?? "") ||
    a.name.localeCompare(b.name);
  const byName = (a: RefEntry, b: RefEntry) => a.name.localeCompare(b.name);
  const order = sort === "recent" ? byRecent : byName;
  const local = refs.filter((r) => r.kind === "local_branch");
  const active = local.filter(
    (r) => r.is_head || !olderThan(r.committed_at, STALE_DAYS, now),
  );
  const stale = local.filter((r) => !active.includes(r));
  const out: Section[] = [
    { key: "local", label: "Local branches", refs: active.sort(order) },
    {
      key: "stale",
      label: "Not touched in 3 months",
      refs: stale.sort(order),
    },
  ];
  const remotes = new Map<string, RefEntry[]>();
  for (const r of refs.filter((r) => r.kind === "remote_branch")) {
    const remote = r.name.split("/")[0];
    remotes.set(remote, [...(remotes.get(remote) ?? []), r]);
  }
  for (const [remote, list] of [...remotes.entries()].sort(([a], [b]) =>
    a.localeCompare(b),
  ))
    out.push({
      key: `remote:${remote}`,
      label: remote,
      refs: list.sort(order),
    });
  out.push({
    key: "tags",
    label: "Tags",
    refs: refs
      .filter((r) => r.kind === "tag")
      .sort((a, b) =>
        sort === "name" ? byName(a, b) : compareVersionsDesc(a.name, b.name),
      ),
  });
  return out.filter((s) => s.refs.length > 0);
}

export default function BranchesTab({
  repositoryId,
  refs,
  onShowHistory,
  onError,
}: Props) {
  const { open: panelOpen } = useSidePanel();
  const [filter, setFilter] = useState("");
  const [sort, setSort] = usePref<Sort>("brainiac.refs.sort", "recent");
  const [folded, setFolded] = usePref<string[]>("brainiac.refs.folded", [
    "stale",
  ]);
  const [selectedName, setSelectedName] = useState<string | null>(null);
  /** Changes against main, open for this branch. */
  const [comparing, setComparing] = useState<string | null>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const all = refs?.refs ?? [];
    return q ? all.filter((r) => r.name.toLowerCase().includes(q)) : all;
  }, [refs, filter]);
  const groups = useMemo(() => sections(shown, sort), [shown, sort]);
  // A filter opens every group so matches are never hidden.
  const isOpen = (key: string) => !!filter.trim() || !folded.includes(key);
  const ordered = useMemo(() => {
    const filtering = !!filter.trim();
    return groups
      .filter((g) => filtering || !folded.includes(g.key))
      .flatMap((g) => g.refs);
  }, [groups, folded, filter]);

  const selected =
    ordered.find((r) => r.full_name === selectedName) ??
    shown.find((r) => r.full_name === selectedName) ??
    ordered.find((r) => r.is_head) ??
    ordered[0] ??
    null;

  const move = (delta: number) => {
    const next = step(ordered, selected, delta);
    if (!next) return;
    setSelectedName(next.full_name);
    listRef.current
      ?.querySelector(`[data-ref="${CSS.escape(next.full_name)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  };
  useKeys(
    {
      j: () => move(1),
      k: () => move(-1),
      ArrowDown: () => move(1),
      ArrowUp: () => move(-1),
      Enter: () => selected && onShowHistory(selected),
      "/": () => filterRef.current?.focus(),
    },
    !comparing,
  );

  const toggle = (key: string) =>
    setFolded(
      folded.includes(key) ? folded.filter((k) => k !== key) : [...folded, key],
    );
  const base = refs?.base ?? null;

  if (comparing)
    return (
      <BranchComparison
        repositoryId={repositoryId}
        branch={comparing}
        onBack={() => setComparing(null)}
        onError={onError}
      />
    );

  return (
    <div className="flex min-h-0 flex-1">
      <section
        aria-label="Refs"
        className="flex min-w-0 flex-1 flex-col border-r"
      >
        <div className="flex items-center gap-3 border-b px-4 py-2.5">
          <label className="search w-72">
            <SearchIcon />
            <input
              ref={filterRef}
              type="text"
              placeholder="Filter branches and tags"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") e.currentTarget.blur();
              }}
            />
            <span className="kbd">/</span>
          </label>
          <div role="tablist" aria-label="Sort refs" className="seg seg-sm">
            <button
              type="button"
              role="tab"
              aria-selected={sort === "recent"}
              onClick={() => setSort("recent")}
            >
              Recent
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={sort === "name"}
              onClick={() => setSort("name")}
            >
              Name
            </button>
          </div>
          <span className="flex-1" />
          <span className="truncate text-[12px] text-muted">
            Read-only. Selecting a ref never checks it out.
          </span>
        </div>
        <div
          className={`${COLUMNS} section-label border-b px-4 py-2 [&>*]:truncate`}
        >
          <span>Name</span>
          <span>Upstream</span>
          <span title={base ? `Commits ahead of / behind ${base}` : undefined}>
            vs {base ? base.slice(base.indexOf("/") + 1) : "main"}
          </span>
          <span>Tip commit</span>
          <span className="text-right">Updated</span>
        </div>
        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto pb-2">
          {!refs && <div className="p-4 text-muted">Loading…</div>}
          {refs && refs.refs.length === 0 && (
            <div className="p-4 text-muted">No branches or tags yet.</div>
          )}
          {refs && refs.refs.length > 0 && shown.length === 0 && (
            <div className="p-4 text-muted">No matching branches or tags.</div>
          )}
          {groups.map((g) => {
            const open = isOpen(g.key);
            return (
              <div key={g.key}>
                <button
                  type="button"
                  className="fold px-4 pt-3.5 pb-1.5"
                  aria-expanded={open}
                  onClick={() => toggle(g.key)}
                >
                  {open ? (
                    <ChevronDown size={10} className="text-muted" />
                  ) : (
                    <ChevronRight size={10} className="text-muted" />
                  )}
                  <span
                    className={`section-label text-fg-2 ${g.key.startsWith("remote:") ? "normal-case" : ""}`}
                  >
                    {g.key.startsWith("remote:")
                      ? `Remote ${g.label}`
                      : g.label}
                  </span>
                  <span className="text-[11px] text-muted">
                    {g.refs.length}
                  </span>
                </button>
                {open &&
                  g.refs.map((r) => (
                    <RefRow
                      key={r.full_name}
                      entry={r}
                      isBase={!!base && r.name === base}
                      selected={r === selected}
                      onSelect={() => setSelectedName(r.full_name)}
                      onOpen={() => onShowHistory(r)}
                    />
                  ))}
              </div>
            );
          })}
        </div>
      </section>
      {selected && panelOpen && (
        <RefPreview
          key={selected.full_name}
          repositoryId={repositoryId}
          entry={selected}
          base={base}
          onShowHistory={() => onShowHistory(selected)}
          onCompare={() => setComparing(selected.full_name)}
          onError={onError}
        />
      )}
    </div>
  );
}

function RefRow({
  entry: r,
  isBase,
  selected,
  onSelect,
  onOpen,
}: {
  entry: RefEntry;
  isBase: boolean;
  selected: boolean;
  onSelect: () => void;
  onOpen: () => void;
}) {
  const up = upstreamText(r);
  const vs = baseText(r);
  return (
    <button
      type="button"
      data-ref={r.full_name}
      className={`list-row ${COLUMNS} h-10 items-center pr-4 pl-[13px]`}
      aria-current={selected}
      onClick={onSelect}
      onDoubleClick={onOpen}
      title={`${r.full_name} — double-click or press Enter to show its history`}
    >
      <span className="flex min-w-0 items-center gap-2">
        {r.kind === "tag" ? (
          <TagIcon size={13} className="shrink-0 text-dirty" />
        ) : (
          <BranchIcon size={13} className="shrink-0 text-fg-2" />
        )}
        <span
          className={`mono min-w-0 truncate text-[12.5px] ${r.is_head ? "font-semibold" : ""}`}
        >
          {middleTruncate(r.name, NAME_BUDGET)}
        </span>
        {r.is_head && (
          <span className="deco shrink-0" data-tone="head">
            HEAD
          </span>
        )}
      </span>
      <span className={`mono truncate text-[12px] ${up.tone}`}>{up.text}</span>
      <span
        className={`mono text-[12px] ${
          isBase
            ? "text-muted"
            : !vs
              ? "text-faint"
              : vs === "same"
                ? "text-clean"
                : r.base_behind
                  ? "text-dirty"
                  : "text-link"
        }`}
      >
        {isBase ? "base" : (vs ?? "—")}
      </span>
      <span className="meta truncate text-fg-3">{r.subject}</span>
      <span className="text-right text-[12px] text-muted">
        {shortAgo(r.committed_at)}
      </span>
    </button>
  );
}

function RefPreview({
  repositoryId,
  entry: r,
  base,
  onShowHistory,
  onCompare,
  onError,
}: {
  repositoryId: string;
  entry: RefEntry;
  base: string | null;
  onShowHistory: () => void;
  /** Changes against main: one patch from the merge base, which can be explained. */
  onCompare: () => void;
  onError: (message: string | null) => void;
}) {
  const [recent, setRecent] = useState<CommitSummary[] | null>(null);
  const [unique, setUnique] = useState<CommitSummary[] | null>(null);
  const [moreUnique, setMoreUnique] = useState(false);
  const recentLatest = useRef(createLatest()).current;
  const uniqueLatest = useRef(createLatest()).current;
  const compare = !!base && r.kind !== "tag" && r.name !== base;

  useEffect(() => {
    void recentLatest.run(
      () =>
        ipc.listCommits({
          repository_id: repositoryId,
          ref: r.full_name,
          limit: PREVIEW_LIMIT,
        }),
      (page) => {
        if (page.repository_id === repositoryId) setRecent(page.items);
      },
      (e) => onError(errorMessage(e)),
    );
    if (!compare || !base) return;
    void uniqueLatest.run(
      () =>
        ipc.listCommits({
          repository_id: repositoryId,
          ref: r.full_name,
          exclude: base,
          limit: 50,
        }),
      (page) => {
        if (page.repository_id !== repositoryId) return;
        setUnique(page.items);
        setMoreUnique(!!page.next_cursor);
      },
      () => setUnique([]),
    );
  }, [
    repositoryId,
    r.full_name,
    base,
    compare,
    recentLatest,
    uniqueLatest,
    onError,
  ]);

  const copy = (text: string) => void navigator.clipboard?.writeText(text);
  const kindLabel =
    r.kind === "tag"
      ? "tag"
      : r.kind === "remote_branch"
        ? "remote-tracking branch"
        : "branch";
  const uniqueIds = new Set((unique ?? []).map((c) => c.id));
  const uniqueCount = r.base_ahead ?? (unique ? unique.length : null);
  const shared =
    uniqueCount === 0
      ? (recent ?? [])
      : (recent ?? []).filter((c) => !uniqueIds.has(c.id));

  return (
    <aside
      aria-label="Ref history preview"
      className="flex w-[420px] shrink-0 flex-col bg-panel"
    >
      <div className="flex flex-col gap-2.5 border-b px-[18px] pt-4 pb-3.5">
        <div className="flex min-w-0 items-start gap-2">
          {r.kind === "tag" ? (
            <TagIcon size={15} className="mt-[3px] shrink-0 text-dirty" />
          ) : (
            <BranchIcon size={15} className="mt-[3px] shrink-0 text-fg-2" />
          )}
          <h2 className="mono selectable m-0 min-w-0 text-base font-semibold break-all">
            {r.name}
          </h2>
        </div>
        <div className="text-[12px] text-fg-2">
          {kindLabel} → <span className="mono">{r.target_id.slice(0, 7)}</span>{" "}
          · newest commit {relativeTime(r.committed_at)}
          {r.upstream && ` · tracks ${r.upstream}`}
        </div>
        {compare && uniqueCount !== null && (
          <div className="text-[12.5px]">
            {uniqueCount === 0 ? (
              <span className="text-clean">Everything here is on {base}.</span>
            ) : (
              <span>
                <b>{plural(uniqueCount, "commit")}</b> not on {base}
                {r.base_behind ? (
                  <span className="text-muted">
                    {" "}
                    · {plural(r.base_behind, "commit")} behind it
                  </span>
                ) : null}
              </span>
            )}
          </div>
        )}
        <div className="flex gap-2">
          <button
            type="button"
            className="btn btn-primary"
            onClick={onShowHistory}
          >
            Show in History
          </button>
          {compare && uniqueCount !== 0 && (
            <button type="button" className="btn" onClick={onCompare}>
              Changes against {base?.slice(base.indexOf("/") + 1)}
            </button>
          )}
          <button type="button" className="btn" onClick={() => copy(r.name)}>
            Copy name
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => copy(r.target_id)}
          >
            Copy hash
          </button>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto pb-3">
        {compare && uniqueCount !== 0 && unique && unique.length > 0 && (
          <>
            <div className="section-label px-[18px] pt-2.5 pb-1">
              Only on this branch
            </div>
            <Timeline commits={unique} accent />
            {moreUnique && (
              <div className="px-[18px] text-[12px] text-muted">
                and more; Show in History lists them all.
              </div>
            )}
          </>
        )}
        <div className="section-label px-[18px] pt-2.5 pb-1">
          {compare && uniqueCount !== 0 && unique?.length
            ? `Shared with ${base}`
            : `Latest on ${r.name}`}
        </div>
        {!recent && <div className="px-[18px] py-2 text-muted">Loading…</div>}
        {recent && <Timeline commits={shared} />}
      </div>
    </aside>
  );
}

function Timeline({
  commits,
  accent = false,
}: {
  commits: CommitSummary[];
  accent?: boolean;
}) {
  return (
    <div>
      {commits.map((c, i) => (
        <div key={c.id} className="flex gap-3 px-[18px]">
          <div className="flex w-2.5 flex-col items-center">
            <span
              className="h-3 w-0.5"
              style={{
                background: i === 0 ? "transparent" : "var(--hunk-line)",
              }}
            />
            <span
              className="h-[9px] w-[9px] shrink-0 rounded-full border-2"
              style={{
                borderColor: accent ? "var(--dirty-dot)" : "var(--sel-bar)",
                background:
                  i === 0
                    ? accent
                      ? "var(--dirty-dot)"
                      : "var(--sel-bar)"
                    : "var(--panel)",
              }}
            />
            <span
              className="w-0.5 flex-1"
              style={{
                background:
                  i === commits.length - 1 ? "transparent" : "var(--hunk-line)",
              }}
            />
          </div>
          <div className="flex min-w-0 flex-1 flex-col gap-0.5 pt-[7px] pb-[9px]">
            <span className="truncate">{c.subject}</span>
            <span className="text-[11.5px] text-muted">
              <span className="mono">{c.short_id}</span> · {c.author_name} ·{" "}
              {relativeTime(c.committed_at)}
            </span>
          </div>
        </div>
      ))}
    </div>
  );
}
