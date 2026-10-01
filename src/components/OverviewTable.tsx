import { useMemo, useState } from "react";
import { relativeTime, shortPath } from "../lib/format";
import type { AppSnapshot, RepositorySummary } from "../lib/ipc";
import { stateDot } from "./Sidebar";

type Filter = "all" | "dirty" | "conflicted" | "stale";
type Sort = "name" | "commit";

type Props = {
  snapshot: AppSnapshot | null;
  onSelect: (id: string) => void;
  onAdd: () => void;
};

export default function OverviewTable({ snapshot, onSelect, onAdd }: Props) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<Sort>("name");
  const repos = snapshot?.repositories ?? [];

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    let list = repos.filter(
      (r) =>
        !q ||
        r.name.toLowerCase().includes(q) ||
        r.display_path.toLowerCase().includes(q),
    );
    if (filter === "dirty")
      list = list.filter((r) => (r.counts?.unique_paths ?? 0) > 0);
    if (filter === "conflicted")
      list = list.filter((r) => (r.counts?.conflicted ?? 0) > 0);
    if (filter === "stale")
      list = list.filter(
        (r) =>
          r.state === "stale" || r.state === "error" || r.state === "missing",
      );
    return [...list].sort((a, b) =>
      sort === "name"
        ? a.name.localeCompare(b.name)
        : (b.last_commit_at ?? "").localeCompare(a.last_commit_at ?? ""),
    );
  }, [repos, query, filter, sort]);

  const totals = useMemo(
    () =>
      repos.reduce(
        (t, r) => ({
          dirty: t.dirty + ((r.counts?.unique_paths ?? 0) > 0 ? 1 : 0),
          conflicted: t.conflicted + ((r.counts?.conflicted ?? 0) > 0 ? 1 : 0),
          problems:
            t.problems + (r.state === "error" || r.state === "missing" ? 1 : 0),
        }),
        { dirty: 0, conflicted: 0, problems: 0 },
      ),
    [repos],
  );

  if (!snapshot) return <div className="muted p-4">Loading…</div>;

  if (repos.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
        <div className="text-lg font-semibold">No repositories yet</div>
        <p className="muted max-w-md">
          Open a local Git repository to see its changes, history, and branches.
          Brainiac only reads; it never commits, checks out, or fetches.
        </p>
        <button
          type="button"
          className="rounded-md border px-3 py-1"
          onClick={onAdd}
        >
          Open Repository… <kbd>⌘O</kbd>
        </button>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <input
          className="selectable w-56 rounded-md border px-2 py-1"
          placeholder="Filter by name or path"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <FilterChip
          label={`All ${repos.length}`}
          active={filter === "all"}
          onClick={() => setFilter("all")}
        />
        <FilterChip
          label={`Dirty ${totals.dirty}`}
          active={filter === "dirty"}
          onClick={() => setFilter("dirty")}
        />
        <FilterChip
          label={`Conflicted ${totals.conflicted}`}
          active={filter === "conflicted"}
          onClick={() => setFilter("conflicted")}
        />
        <FilterChip
          label={`Stale/Error ${totals.problems}`}
          active={filter === "stale"}
          onClick={() => setFilter("stale")}
        />
        <span className="muted ml-auto">Sort</span>
        <select
          className="rounded border px-1"
          value={sort}
          onChange={(e) => setSort(e.target.value as Sort)}
        >
          <option value="name">Name</option>
          <option value="commit">Latest commit</option>
        </select>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full border-collapse text-left">
          <thead
            className="muted sticky top-0 text-[11px] uppercase tracking-wide"
            style={{ background: "var(--pane)" }}
          >
            <tr>
              <th className="px-3 py-1 font-semibold">Repository</th>
              <th className="px-2 py-1 font-semibold">Branch</th>
              <th className="px-2 py-1 text-right font-semibold">Staged</th>
              <th className="px-2 py-1 text-right font-semibold">Unstaged</th>
              <th className="px-2 py-1 text-right font-semibold">Untracked</th>
              <th className="px-2 py-1 text-right font-semibold">Conflicts</th>
              <th className="px-2 py-1 font-semibold">Upstream</th>
              <th className="px-2 py-1 font-semibold">Checked</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <Row key={r.id} repo={r} onSelect={onSelect} />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function FilterChip({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={`badge border ${active ? "selected-bg" : ""}`}
      aria-pressed={active}
      onClick={onClick}
    >
      {label}
    </button>
  );
}

function Row({
  repo,
  onSelect,
}: {
  repo: RepositorySummary;
  onSelect: (id: string) => void;
}) {
  const dot = stateDot(repo);
  const c = repo.counts;
  const head = repo.head;
  const branch = !head
    ? "—"
    : head.kind === "branch"
      ? head.branch
      : head.kind === "detached"
        ? `detached @ ${head.commit_id?.slice(0, 7)}`
        : "no commits";
  return (
    <tr
      className="cursor-default border-t hover:bg-black/5 dark:hover:bg-white/5"
      onDoubleClick={() => onSelect(repo.id)}
      onClick={() => onSelect(repo.id)}
    >
      <td className="px-3 py-1.5">
        <div className="flex items-center gap-2">
          <span
            className={`inline-block h-2 w-2 rounded-full ${dot.className}`}
            title={dot.title}
          />
          <span className="font-medium">{repo.name}</span>
        </div>
        <div className="muted mono truncate text-[11px]">
          {shortPath(repo.display_path)}
        </div>
        {repo.error && (
          <div className="text-[11px] text-red-600 dark:text-red-400">
            {repo.error.message}
          </div>
        )}
      </td>
      <td className="mono px-2 py-1.5">{branch}</td>
      <Num value={c?.staged} />
      <Num value={c?.unstaged} />
      <Num value={c?.untracked} />
      <Num value={c?.conflicted} warn />
      <td className="mono px-2 py-1.5">
        {repo.upstream ? (
          `↑${repo.upstream.ahead} ↓${repo.upstream.behind}`
        ) : (
          <span className="muted">none</span>
        )}
      </td>
      <td className="muted px-2 py-1.5">
        {relativeTime(repo.last_checked_at)}
      </td>
    </tr>
  );
}

function Num({ value, warn }: { value: number | undefined; warn?: boolean }) {
  const v = value ?? 0;
  return (
    <td
      className={`mono px-2 py-1.5 text-right ${v === 0 ? "muted" : warn ? "text-orange-600 dark:text-orange-400" : ""}`}
    >
      {value === undefined ? "—" : v}
    </td>
  );
}
