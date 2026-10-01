import type { AppSnapshot, RepositorySummary } from "../lib/ipc";

type Props = {
  snapshot: AppSnapshot | null;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onAdd: () => void;
  onPalette: () => void;
};

export function stateDot(r: RepositorySummary): {
  className: string;
  title: string;
} {
  if (r.state === "missing" || r.state === "error")
    return {
      className: "bg-red-500",
      title: r.error?.message ?? "Unavailable",
    };
  if (r.state === "refreshing")
    return { className: "bg-sky-500 animate-pulse", title: "Refreshing" };
  if (r.counts?.conflicted)
    return { className: "bg-orange-500", title: "Conflicts" };
  if (r.counts && r.counts.unique_paths > 0)
    return { className: "bg-amber-400", title: "Uncommitted changes" };
  if (r.state === "stale") return { className: "bg-gray-400", title: "Stale" };
  return { className: "bg-emerald-500", title: "Clean" };
}

export default function Sidebar({
  snapshot,
  selectedId,
  onSelect,
  onAdd,
  onPalette,
}: Props) {
  const repos = snapshot?.repositories ?? [];
  const recent = (snapshot?.recent_repository_ids ?? [])
    .map((id) => repos.find((r) => r.id === id))
    .filter((r): r is RepositorySummary => !!r)
    .slice(0, 5);

  return (
    <aside className="flex w-56 shrink-0 flex-col overflow-y-auto px-2 py-2">
      <div className="mb-2 flex items-center justify-between px-1">
        <span className="font-semibold">Brainiac</span>
        <div className="flex gap-1">
          <button
            type="button"
            className="rounded border px-1.5"
            title="Switch repository (⌘K)"
            onClick={onPalette}
          >
            ⌘K
          </button>
          <button
            type="button"
            className="rounded border px-1.5"
            title="Open repository (⌘O)"
            onClick={onAdd}
          >
            +
          </button>
        </div>
      </div>

      <button
        type="button"
        className="row"
        aria-pressed={selectedId === null}
        onClick={() => onSelect(null)}
      >
        All repositories
        <span className="muted ml-auto">{repos.length}</span>
      </button>

      <Section title="Repositories">
        {repos.length === 0 && (
          <div className="muted px-2 py-1">None yet. Press ⌘O.</div>
        )}
        {repos.map((r) => (
          <RepoRow
            key={r.id}
            repo={r}
            selected={r.id === selectedId}
            onSelect={onSelect}
          />
        ))}
      </Section>

      {recent.length > 0 && (
        <Section title="Recent">
          {recent.map((r) => (
            <RepoRow
              key={r.id}
              repo={r}
              selected={r.id === selectedId}
              onSelect={onSelect}
            />
          ))}
        </Section>
      )}
    </aside>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="mt-3">
      <div className="muted px-2 pb-1 text-[11px] font-semibold uppercase tracking-wide">
        {title}
      </div>
      {children}
    </div>
  );
}

function RepoRow({
  repo,
  selected,
  onSelect,
}: {
  repo: RepositorySummary;
  selected: boolean;
  onSelect: (id: string) => void;
}) {
  const dot = stateDot(repo);
  return (
    <button
      type="button"
      className="row w-full text-left"
      aria-pressed={selected}
      onClick={() => onSelect(repo.id)}
      title={repo.display_path}
    >
      <span
        className={`inline-block h-2 w-2 shrink-0 rounded-full ${dot.className}`}
        title={dot.title}
      />
      <span className="truncate">{repo.name}</span>
      {repo.counts && repo.counts.unique_paths > 0 && (
        <span className="muted ml-auto">{repo.counts.unique_paths}</span>
      )}
    </button>
  );
}
