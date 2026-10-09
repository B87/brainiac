import { useEffect, useMemo, useState } from "react";
import {
  type BranchComparison as Comparison,
  errorMessage,
  ipc,
} from "../lib/ipc";
import { KIND_LETTER, kindTone, splitPath } from "../lib/repo";
import { useDiff } from "../lib/useDiff";
import DiffView from "./DiffView";
import { ChevronLeft } from "./icons";
import { OrderSwitch, useExplainedPatch } from "./useExplainedPatch";

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
}: {
  repositoryId: string;
  /** The branch's full ref name. */
  branch: string;
  onBack: () => void;
  onError: (message: string | null) => void;
}) {
  const [comparison, setComparison] = useState<Comparison | null>(null);
  const [selected, setSelected] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
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
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="flex w-[260px] shrink-0 flex-col border-r">
          {explained.hasExplanation && (
            <div className="flex shrink-0 justify-center border-b px-2 py-1.5">
              <OrderSwitch
                order={explained.order}
                setOrder={explained.setOrder}
              />
            </div>
          )}
          <ul
            aria-label="Changed files"
            className="m-0 min-h-0 flex-1 list-none overflow-y-auto p-1"
          >
            {!comparison && <li className="p-3 text-muted">Loading…</li>}
            {comparison?.files.length === 0 && (
              <li className="p-3 text-muted">No changes against {base}.</li>
            )}
            {files.map((f) => {
              const { dir, name } = splitPath(f.path);
              return (
                <li key={f.path}>
                  <button
                    type="button"
                    className="side-row h-auto w-full items-start gap-2 py-1.5 text-left"
                    aria-current={f === file}
                    onClick={() => setSelected(f.path)}
                  >
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
                    <span className="mt-px shrink-0 text-[11px] tabular">
                      {f.additions != null && (
                        <span className="text-added">+{f.additions} </span>
                      )}
                      {f.deletions != null && (
                        <span className="text-deleted">−{f.deletions}</span>
                      )}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
        <div className="flex min-w-0 flex-1 flex-col">
          <DiffView
            diff={diff}
            loading={loading}
            empty={comparison ? "Select a file" : "Loading…"}
            historical
            onOpenInEditor={() => {}}
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
