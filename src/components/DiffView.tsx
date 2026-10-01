import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import type { DiffLine, DiffResult, Hunk } from "../lib/ipc";
import { visibleRange } from "../lib/virtual";

type Props = {
  diff: DiffResult | null;
  loading: boolean;
  empty: string;
  onOpenInEditor: (path: string) => void;
  /** Commit patches open the current working-tree file, so they say so. */
  openLabel?: string;
};

/** Must match `.diff-row` in index.css. */
const ROW_HEIGHT = 18;

type Row = { kind: "hunk"; hunk: Hunk } | { kind: "line"; line: DiffLine };

export function diffRows(hunks: Hunk[]): Row[] {
  const rows: Row[] = [];
  for (const hunk of hunks) {
    rows.push({ kind: "hunk", hunk });
    for (const line of hunk.lines) rows.push({ kind: "line", line });
  }
  return rows;
}

export default function DiffView({
  diff,
  loading,
  empty,
  onOpenInEditor,
  openLabel = "Open in editor",
}: Props) {
  if (!diff)
    return <div className="muted p-4">{loading ? "Loading diff…" : empty}</div>;
  const path = diff.selector.path;
  const copy = (text: string) => void navigator.clipboard?.writeText(text);
  const header = (
    <div className="flex shrink-0 items-center gap-2 border-b px-3 py-1">
      <span className="mono selectable truncate">
        {diff.content.kind === "text" &&
          diff.content.old_path &&
          diff.content.old_path !== path && (
            <span className="muted">{diff.content.old_path} → </span>
          )}
        {path}
      </span>
      <span className="badge muted shrink-0 border">
        {labelFor(diff.selector)}
      </span>
      {loading && <span className="muted">updating…</span>}
      <button
        type="button"
        className="ml-auto shrink-0 rounded border px-2 py-0.5"
        onClick={() => copy(path)}
      >
        Copy path
      </button>
      <button
        type="button"
        className="shrink-0 rounded border px-2 py-0.5"
        onClick={() => onOpenInEditor(path)}
      >
        {openLabel}
      </button>
    </div>
  );

  if (diff.content.kind === "non_text") {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        {header}
        <div className="p-4">
          <div className="font-medium">{reasonLabel(diff.content.reason)}</div>
          <div className="muted selectable">{diff.content.summary}</div>
        </div>
      </div>
    );
  }

  const { hunks, truncated, total_lines } = diff.content;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {header}
      {hunks.length === 0 ? (
        <div className="muted p-4">
          {diff.content.old_path && diff.content.old_path !== path
            ? "Renamed without content changes."
            : "No textual differences."}
        </div>
      ) : (
        <PatchRows
          hunks={hunks}
          resetKey={JSON.stringify(diff.selector)}
          footer={
            truncated && (
              <div className="m-3 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
                Output truncated
                {total_lines !== null ? ` (${total_lines} lines in total)` : ""}
                . Open the file in your editor for the full content.
              </div>
            )
          }
        />
      )}
    </div>
  );
}

/** Scrollable patch body; only rows near the viewport are in the DOM for long diffs. */
function PatchRows({
  hunks,
  resetKey,
  footer,
}: {
  hunks: Hunk[];
  /** Changes when a different file is shown, which scrolls back to the top. */
  resetKey: string;
  footer: ReactNode;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(800);
  const rows = useMemo(() => diffRows(hunks), [hunks]);

  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    setViewport(el.clientHeight);
    const observer = new ResizeObserver(() => setViewport(el.clientHeight));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: resetKey marks a different file, whose patch starts at the top.
  useEffect(() => {
    if (scroller.current) scroller.current.scrollTop = 0;
    setScrollTop(0);
  }, [resetKey]);

  const { start, end } = visibleRange(
    rows.length,
    ROW_HEIGHT,
    scrollTop,
    viewport,
  );
  return (
    <div
      ref={scroller}
      className="mono selectable min-h-0 flex-1 overflow-auto"
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
    >
      <div className="min-w-max" style={{ height: rows.length * ROW_HEIGHT }}>
        <div style={{ transform: `translateY(${start * ROW_HEIGHT}px)` }}>
          {rows.slice(start, end).map((row, i) => (
            // Rows have no stable identity of their own; the absolute index is stable for one patch.
            // biome-ignore lint/suspicious/noArrayIndexKey: see above.
            <RowView key={start + i} row={row} />
          ))}
        </div>
      </div>
      {footer}
    </div>
  );
}

function RowView({ row }: { row: Row }) {
  if (row.kind === "hunk") {
    const h = row.hunk;
    return (
      <div className="diff-row muted bg-black/5 dark:bg-white/5">
        <span className="diff-gutter">…</span>
        <span className="diff-gutter" />
        <span className="px-2">
          @@ -{h.old_start},{h.old_lines} +{h.new_start},{h.new_lines} @@{" "}
          {h.header}
        </span>
      </div>
    );
  }
  const l = row.line;
  return (
    <div
      className={`diff-row ${
        l.kind === "add" ? "diff-add" : l.kind === "delete" ? "diff-del" : ""
      }`}
    >
      <span className="diff-gutter">{l.old_no ?? ""}</span>
      <span className="diff-gutter">{l.new_no ?? ""}</span>
      <span className="px-2">
        <span className="muted inline-block w-3">
          {l.kind === "add" ? "+" : l.kind === "delete" ? "-" : " "}
        </span>
        {l.text}
      </span>
    </div>
  );
}

function labelFor(selector: DiffResult["selector"]): string {
  switch (selector.kind) {
    case "index_vs_head":
      return "staged · HEAD → index";
    case "worktree_vs_index":
      return "unstaged · index → working tree";
    case "untracked_preview":
      return "untracked · preview";
    case "commit":
      return selector.parent_index === 0
        ? `commit ${selector.commit_id.slice(0, 7)}`
        : `commit ${selector.commit_id.slice(0, 7)} · vs parent ${selector.parent_index + 1}`;
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
