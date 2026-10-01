import { relativeTime } from "../lib/format";
import type { CommitSummary } from "../lib/ipc";

type Props = {
  commits: CommitSummary[];
  anchor: string | null;
  refName: string;
  selectedId: string | null;
  loading: boolean;
  hasMore: boolean;
  filterActive: boolean;
  onSelect: (commit: CommitSummary) => void;
  onLoadMore: () => void;
};

export default function HistoryList({
  commits,
  anchor,
  refName,
  selectedId,
  loading,
  hasMore,
  filterActive,
  onSelect,
  onLoadMore,
}: Props) {
  return (
    <div className="py-1">
      <div className="muted px-3 py-1 text-[11px]">
        {anchor === ""
          ? "No commits yet."
          : `${refName}${filterActive ? " · filtered" : ""}`}
      </div>
      {commits.map((c) => (
        <button
          type="button"
          key={c.id}
          className="row mx-1 w-[calc(100%-0.5rem)] items-start text-left"
          aria-pressed={selectedId === c.id}
          onClick={() => onSelect(c)}
        >
          <div className="min-w-0 flex-1">
            <div className="truncate">{c.subject}</div>
            <div className="muted flex items-center gap-2 text-[11px]">
              <span className="mono">{c.short_id}</span>
              <span className="truncate">{c.author_name}</span>
              <span className="ml-auto shrink-0">
                {relativeTime(c.committed_at)}
              </span>
            </div>
            {c.decorations.length > 0 && (
              <div className="mt-0.5 flex flex-wrap gap-1">
                {c.decorations.map((d) => (
                  <span key={d} className="badge border mono">
                    {d}
                  </span>
                ))}
              </div>
            )}
          </div>
        </button>
      ))}
      {loading && <div className="muted px-3 py-2">Loading…</div>}
      {!loading && commits.length === 0 && anchor !== "" && anchor !== null && (
        <div className="muted px-3 py-2">No matching commits.</div>
      )}
      {!loading && hasMore && (
        <button
          type="button"
          className="row mx-1 w-[calc(100%-0.5rem)] justify-center"
          onClick={onLoadMore}
        >
          Load more
        </button>
      )}
    </div>
  );
}
