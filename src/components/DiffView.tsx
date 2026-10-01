import type { DiffResult, Hunk } from "../lib/ipc";

type Props = {
  diff: DiffResult | null;
  loading: boolean;
  empty: string;
  onOpenInEditor: (path: string) => void;
};

export default function DiffView({
  diff,
  loading,
  empty,
  onOpenInEditor,
}: Props) {
  if (!diff)
    return <div className="muted p-4">{loading ? "Loading diff…" : empty}</div>;
  const path = diff.selector.path;
  const header = (
    <div className="flex items-center gap-2 border-b px-3 py-1">
      <span className="mono selectable truncate">{path}</span>
      <span className="badge muted border">{labelFor(diff.selector.kind)}</span>
      {loading && <span className="muted">updating…</span>}
      <button
        type="button"
        className="ml-auto rounded border px-2 py-0.5"
        onClick={() => onOpenInEditor(path)}
      >
        Open in editor
      </button>
    </div>
  );

  if (diff.content.kind === "non_text") {
    return (
      <>
        {header}
        <div className="p-4">
          <div className="font-medium">{reasonLabel(diff.content.reason)}</div>
          <div className="muted selectable">{diff.content.summary}</div>
        </div>
      </>
    );
  }

  const { hunks, truncated, total_lines } = diff.content;
  return (
    <>
      {header}
      {hunks.length === 0 && (
        <div className="muted p-4">No textual differences.</div>
      )}
      <div className="mono selectable">
        {hunks.map((h) => (
          <HunkView key={`${h.old_start}:${h.new_start}`} hunk={h} />
        ))}
      </div>
      {truncated && (
        <div className="m-3 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
          Output truncated
          {total_lines !== null ? ` (${total_lines} lines in total)` : ""}. Open
          the file in your editor for the full content.
        </div>
      )}
    </>
  );
}

function HunkView({ hunk }: { hunk: Hunk }) {
  return (
    <table className="w-full border-collapse">
      <tbody>
        <tr className="muted bg-black/5 dark:bg-white/5">
          <td className="diff-gutter" colSpan={2}>
            …
          </td>
          <td className="px-2 py-0.5">
            @@ -{hunk.old_start},{hunk.old_lines} +{hunk.new_start},
            {hunk.new_lines} @@ {hunk.header}
          </td>
        </tr>
        {hunk.lines.map((l) => (
          <tr
            key={`${l.kind}:${l.old_no}:${l.new_no}`}
            className={
              l.kind === "add"
                ? "diff-add"
                : l.kind === "delete"
                  ? "diff-del"
                  : ""
            }
          >
            <td className="diff-gutter">{l.old_no ?? ""}</td>
            <td className="diff-gutter">{l.new_no ?? ""}</td>
            <td className="whitespace-pre px-2">
              <span className="muted inline-block w-3">
                {l.kind === "add" ? "+" : l.kind === "delete" ? "-" : " "}
              </span>
              {l.text}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function labelFor(kind: DiffResult["selector"]["kind"]): string {
  switch (kind) {
    case "index_vs_head":
      return "staged · HEAD → index";
    case "worktree_vs_index":
      return "unstaged · index → working tree";
    case "untracked_preview":
      return "untracked · preview";
    case "commit":
      return "commit";
  }
}

function reasonLabel(reason: string): string {
  switch (reason) {
    case "binary":
      return "Binary file";
    case "submodule":
      return "Submodule";
    case "lfs_pointer":
      return "Git LFS pointer";
    case "symlink":
      return "Symbolic link";
    case "too_large":
      return "File too large to display";
    default:
      return reason;
  }
}
