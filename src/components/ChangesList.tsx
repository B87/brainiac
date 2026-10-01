import type { ChangeEntry, ChangesResult } from "../lib/ipc";
import { entryKey } from "./RepositoryView";

type Props = {
  changes: ChangesResult | null;
  selectedKey: string | null;
  onSelect: (entry: ChangeEntry) => void;
};

const GROUPS: Array<{ key: ChangeEntry["group"]; label: string }> = [
  { key: "conflicted", label: "Conflicts" },
  { key: "staged", label: "Staged" },
  { key: "unstaged", label: "Unstaged" },
  { key: "untracked", label: "Untracked" },
];

export const KIND_LETTER: Record<ChangeEntry["kind"], string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  copied: "C",
  type_changed: "T",
  unmerged: "U",
  untracked: "?",
};

export default function ChangesList({ changes, selectedKey, onSelect }: Props) {
  if (!changes) return <div className="muted p-3">Loading…</div>;
  if (changes.entries.length === 0)
    return <div className="muted p-3">No changes.</div>;
  return (
    <div className="py-1">
      {GROUPS.map(({ key, label }) => {
        const entries = changes.entries.filter((e) => e.group === key);
        if (entries.length === 0) return null;
        return (
          <div key={key} className="mb-2">
            <div className="muted px-3 py-1 text-[11px] font-semibold uppercase tracking-wide">
              {label} <span className="font-normal">{entries.length}</span>
            </div>
            {entries.map((e) => {
              const k = entryKey(e);
              return (
                <button
                  type="button"
                  key={k}
                  className="row mx-1 w-[calc(100%-0.5rem)] text-left"
                  aria-pressed={selectedKey === k}
                  onClick={() => onSelect(e)}
                  title={e.old_path ? `${e.old_path} → ${e.path}` : e.path}
                >
                  <span
                    className={`mono w-4 shrink-0 text-center ${kindColor(e.kind)}`}
                  >
                    {KIND_LETTER[e.kind]}
                  </span>
                  <span className="truncate">
                    {e.old_path && (
                      <span className="muted">{e.old_path} → </span>
                    )}
                    {e.path}
                  </span>
                  {e.is_submodule && (
                    <span className="badge muted ml-auto border">
                      submodule
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        );
      })}
    </div>
  );
}

export function kindColor(kind: ChangeEntry["kind"]): string {
  switch (kind) {
    case "added":
    case "untracked":
      return "text-emerald-600 dark:text-emerald-400";
    case "deleted":
      return "text-red-600 dark:text-red-400";
    case "unmerged":
      return "text-orange-600 dark:text-orange-400";
    default:
      return "text-amber-600 dark:text-amber-400";
  }
}
