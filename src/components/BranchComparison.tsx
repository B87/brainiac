import { useEffect, useMemo, useState } from "react";
import {
  type BranchComparison as Comparison,
  errorMessage,
  ipc,
} from "../lib/ipc";
import { useDiff } from "../lib/useDiff";
import ChangedFileList from "./ChangedFileList";
import DiffView from "./DiffView";
import { ChevronLeft } from "./icons";
import { useExplainedPatch } from "./useExplainedPatch";

/** `refs/heads/x` → `x`; `refs/remotes/origin/x` → `origin/x`. */
function shortRef(name: string): string {
  return name.replace(/^refs\/(heads|remotes)\//, "");
}

/**
 * **Changes against main** (SPEC.md, section 14, Boundaries): one patch from
 * where a branch left the default branch (their merge base) to its tip, so
 * the default branch's later commits are left out. It can be explained.
 */
export default function BranchComparison({
  repositoryId,
  branch,
  onBack,
  onError,
  onOpenInEditor,
}: {
  repositoryId: string;
  /** The branch's full ref name. */
  branch: string;
  onBack: () => void;
  onError: (message: string | null) => void;
  /** Opens the file as it is in the working tree, as History's patches do. */
  onOpenInEditor: (path: string, line?: number) => void;
}) {
  const [comparison, setComparison] = useState<Comparison | null>(null);
  const [selected, setSelected] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    // Another branch starts clean: not the last one's file or merge base.
    setComparison(null);
    setSelected(null);
    ipc
      .getBranchComparison(repositoryId, branch)
      .then((c) => {
        if (!live) return;
        setComparison(c);
        setSelected((s) => s ?? c.files[0]?.path ?? null);
      })
      .catch((e) => live && onError(errorMessage(e)));
    return () => {
      live = false;
    };
  }, [repositoryId, branch, onError]);

  const subject = useMemo(
    () => ({ kind: "branch" as const, reference: branch }),
    [branch],
  );
  const explained = useExplainedPatch({
    repositoryId,
    view: "branch",
    subject,
    files: comparison?.files ?? [],
    selectedPath: selected,
    onSelectFile: setSelected,
    version: comparison?.tip ?? null,
  });
  const files = explained.ordered;
  const file = files.find((f) => f.path === selected) ?? null;
  const { diff, loading, ignoreWhitespace, setIgnoreWhitespace } = useDiff(
    repositoryId,
    comparison && file
      ? {
          kind: "range",
          base: comparison.merge_base,
          head: comparison.tip,
          path: file.path,
          old_path: file.old_path,
        }
      : null,
    comparison?.tip,
    (m) => onError(m),
  );
  const index = file ? files.indexOf(file) : -1;
  const base = comparison ? shortRef(comparison.base_branch) : "main";

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b px-4 py-2.5 text-[12.5px]">
        <button type="button" className="btn btn-sm" onClick={onBack}>
          <ChevronLeft size={12} />
          Branches
        </button>
        <span className="font-semibold">
          <span className="mono">{shortRef(branch)}</span>: changes against{" "}
          <span className="mono">{base}</span>
        </span>
        {comparison && (
          <span className="text-muted">
            from the merge base{" "}
            <code className="mono">{comparison.merge_base.slice(0, 7)}</code>,
            the last commit it shares with {base}, to its tip{" "}
            <code className="mono">{comparison.tip.slice(0, 7)}</code>. {base}'s
            later commits are left out.
          </span>
        )}
        <span className="flex-1" />
        {explained.header}
      </div>
      <div className="flex min-h-0 flex-1">
        <ChangedFileList
          files={files}
          selectedPath={file?.path ?? null}
          onSelect={setSelected}
          explained={explained}
          placeholder={comparison ? `No changes against ${base}.` : "Loading…"}
        />
        <div className="flex min-w-0 flex-1 flex-col">
          <DiffView
            diff={diff}
            loading={loading}
            empty={comparison ? "Select a file" : "Loading…"}
            historical
            onOpenInEditor={onOpenInEditor}
            ignoreWhitespace={ignoreWhitespace}
            onIgnoreWhitespace={setIgnoreWhitespace}
            annotate={explained.annotate}
            extra={explained.toolbar}
            meta={`vs merge base ${comparison?.merge_base.slice(0, 7) ?? ""}`}
            stepper={
              files.length > 1 && index >= 0
                ? {
                    index,
                    total: files.length,
                    onPrev: () =>
                      setSelected(files[Math.max(0, index - 1)].path),
                    onNext: () =>
                      setSelected(
                        files[Math.min(files.length - 1, index + 1)].path,
                      ),
                  }
                : undefined
            }
          />
        </div>
        {explained.panel}
      </div>
      {explained.dialog}
    </div>
  );
}
