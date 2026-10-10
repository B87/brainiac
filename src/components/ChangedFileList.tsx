import type { CommitFile } from "../lib/ipc";
import { KIND_LETTER, kindTone, splitPath } from "../lib/repo";
import {
  type ExplainedPatch,
  FileNotes,
  NotCoveredHeading,
  OrderSwitch,
  startsNotCovered,
} from "./useExplainedPatch";

/**
 * A subject's changed files as a flat list beside its patch, as a branch
 * comparison and a run's result show them (SPEC.md, section 14): each file's
 * step in the explanation's tour, its kind, name, and folder, its notes, and
 * its line counts, with Reading order and Path above the list when there is
 * an explanation. Commit details and a pull request keep lists of their own
 * (a folder tree; review marks and folds) built from the same pieces.
 */
export default function ChangedFileList({
  files,
  selectedPath,
  onSelect,
  explained,
  placeholder,
}: {
  /** In the order to list them: the explanation's reading order, or by path. */
  files: CommitFile[];
  selectedPath: string | null;
  onSelect: (path: string) => void;
  explained: Pick<
    ExplainedPatch<CommitFile>,
    "hasExplanation" | "order" | "setOrder" | "steps" | "fileNotes"
  >;
  /** Said in place of the list when there are no files: loading, or none. */
  placeholder?: string;
}) {
  const { steps, fileNotes } = explained;
  return (
    <div className="flex w-[260px] shrink-0 flex-col border-r">
      {explained.hasExplanation && (
        <div className="flex shrink-0 justify-center border-b px-2 py-1.5">
          <OrderSwitch order={explained.order} setOrder={explained.setOrder} />
        </div>
      )}
      <ul
        aria-label="Changed files"
        className="m-0 min-h-0 flex-1 list-none overflow-y-auto p-1"
      >
        {files.length === 0 && placeholder && (
          <li className="p-3 text-muted">{placeholder}</li>
        )}
        {files.map((f, i) => {
          const { dir, name } = splitPath(f.path);
          return (
            <li key={f.path}>
              {startsNotCovered(files, i, steps) && <NotCoveredHeading />}
              <button
                type="button"
                className="side-row h-auto w-full items-start gap-2 py-1.5 text-left"
                aria-current={f.path === selectedPath}
                onClick={() => onSelect(f.path)}
              >
                {steps.size > 0 && (
                  <span className="tabular mt-px w-4 shrink-0 text-right text-[11.5px] text-muted">
                    {steps.get(f.path) ?? ""}
                  </span>
                )}
                <span className="kind mt-px" data-tone={kindTone(f.kind)}>
                  {KIND_LETTER[f.kind]}
                </span>
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate">{name}</span>
                  {dir && (
                    <span className="truncate text-[11px] text-muted">
                      {dir.replace(/\/$/, "")}
                    </span>
                  )}
                </span>
                <FileNotes count={fileNotes.get(f.path)} />
                <span className="mt-px shrink-0 text-[11px] tabular">
                  {f.additions != null && (
                    <span className="text-added">+{f.additions} </span>
                  )}
                  {f.deletions != null && (
                    <span className="text-deleted">−{f.deletions}</span>
                  )}
                  {f.is_binary && <span className="text-muted">bin</span>}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
