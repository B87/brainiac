import type { RefEntry, RefsResult } from "../lib/ipc";

type Props = {
  refs: RefsResult | null;
  filter: string;
  /** Full name of the ref the History tab currently shows, if not HEAD. */
  activeRef: string | null;
  onSelect: (ref: RefEntry) => void;
};

const GROUPS: Array<{ kind: RefEntry["kind"]; label: string }> = [
  { kind: "local_branch", label: "Local branches" },
  { kind: "remote_branch", label: "Remote branches" },
  { kind: "tag", label: "Tags" },
];

export default function RefsList({ refs, filter, activeRef, onSelect }: Props) {
  if (!refs) return <div className="muted p-3">Loading…</div>;
  const needle = filter.trim().toLowerCase();
  const shown = needle
    ? refs.refs.filter((r) => r.name.toLowerCase().includes(needle))
    : refs.refs;
  if (refs.refs.length === 0)
    return <div className="muted p-3">No branches or tags yet.</div>;
  if (shown.length === 0)
    return <div className="muted p-3">No matching branches or tags.</div>;
  return (
    <div className="overflow-y-auto py-1">
      <p className="muted px-3 py-1 text-[11px]">
        Select a branch or tag to view its history. Nothing is checked out.
      </p>
      {GROUPS.map(({ kind, label }) => {
        const entries = shown.filter((r) => r.kind === kind);
        if (entries.length === 0) return null;
        return (
          <div key={kind} className="mb-2">
            <div className="muted px-3 py-1 text-[11px] font-semibold uppercase tracking-wide">
              {label} <span className="font-normal">{entries.length}</span>
            </div>
            {entries.map((r) => (
              <button
                type="button"
                key={r.full_name}
                className="row mx-1 w-[calc(100%-0.5rem)] text-left"
                aria-pressed={activeRef === r.full_name}
                onClick={() => onSelect(r)}
                title={r.full_name}
              >
                <span className="mono truncate">{r.name}</span>
                {r.is_head && (
                  <span className="badge shrink-0 border border-sky-400 text-sky-700 dark:text-sky-300">
                    current
                  </span>
                )}
                {r.upstream && (
                  <span className="muted mono shrink-0 text-[11px]">
                    ↑ {r.upstream}
                  </span>
                )}
                <span className="muted mono ml-auto shrink-0 text-[11px]">
                  {r.target_id.slice(0, 7)}
                </span>
              </button>
            ))}
          </div>
        );
      })}
    </div>
  );
}
