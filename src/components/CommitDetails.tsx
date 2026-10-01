import { useEffect, useRef, useState } from "react";
import {
  type CommitDetail,
  type CommitFile,
  type CommitSummary,
  type DiffResult,
  errorMessage,
  ipc,
} from "../lib/ipc";
import { createLatest } from "../lib/stale";
import { KIND_LETTER, kindColor } from "./ChangesList";
import DiffView from "./DiffView";

type Props = {
  repositoryId: string;
  /** Render with `key={commit.id}` so state resets when the selection changes. */
  commit: CommitSummary;
  onError: (message: string | null) => void;
  onOpenInEditor: (path: string) => void;
};

export default function CommitDetails({
  repositoryId,
  commit,
  onError,
  onOpenInEditor,
}: Props) {
  const [parentIndex, setParentIndex] = useState(0);
  const [detail, setDetail] = useState<CommitDetail | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [diff, setDiff] = useState<DiffResult | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const detailLatest = useRef(createLatest()).current;
  const diffLatest = useRef(createLatest()).current;

  useEffect(() => {
    void detailLatest.run(
      () => ipc.getCommit(repositoryId, commit.id, parentIndex),
      (d) => {
        if (d.repository_id !== repositoryId) return;
        setDetail(d);
        // Keep the selected file when switching parents; otherwise show the first one.
        setSelectedPath((current) =>
          current && d.files.some((f) => f.path === current)
            ? current
            : (d.files[0]?.path ?? null),
        );
      },
      (e) => onError(errorMessage(e)),
    );
  }, [repositoryId, commit.id, parentIndex, detailLatest, onError]);

  const selectedFile =
    detail?.files.find((f) => f.path === selectedPath) ?? null;
  useEffect(() => {
    if (!detail || !selectedFile) {
      diffLatest.cancel();
      setDiff(null);
      setDiffLoading(false);
      return;
    }
    setDiffLoading(true);
    void diffLatest.run(
      () =>
        ipc.getDiff(repositoryId, {
          kind: "commit",
          commit_id: detail.id,
          path: selectedFile.path,
          old_path: selectedFile.old_path,
          parent_index: detail.compared_parent_index,
        }),
      (result) => {
        setDiff(result);
        setDiffLoading(false);
      },
      (e) => {
        setDiffLoading(false);
        onError(errorMessage(e));
      },
    );
  }, [repositoryId, detail, selectedFile, diffLatest, onError]);

  const copy = (text: string) => void navigator.clipboard?.writeText(text);
  const isMerge = commit.parent_ids.length > 1;
  const committerDiffers =
    detail &&
    (detail.committer_name !== commit.author_name ||
      detail.committer_email !== commit.author_email);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="selectable max-h-[50%] shrink-0 overflow-y-auto border-b p-4">
        <h2 className="mb-2 text-base font-semibold">{commit.subject}</h2>
        {detail?.body && (
          <p className="mb-3 whitespace-pre-wrap">{detail.body}</p>
        )}
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
          <dt className="muted">Commit</dt>
          <dd className="mono flex items-center gap-2">
            {commit.id}
            <button
              type="button"
              className="rounded border px-1 text-[11px]"
              onClick={() => copy(commit.id)}
            >
              Copy
            </button>
          </dd>
          <dt className="muted">Author</dt>
          <dd>
            {commit.author_name} &lt;{commit.author_email}&gt; ·{" "}
            {new Date(commit.authored_at).toLocaleString()}
          </dd>
          <dt className="muted">Committer</dt>
          <dd>
            {committerDiffers && (
              <>
                {detail.committer_name} &lt;{detail.committer_email}&gt; ·{" "}
              </>
            )}
            {new Date(commit.committed_at).toLocaleString()}
          </dd>
          <dt className="muted">{isMerge ? "Compare with" : "Parent"}</dt>
          <dd className="mono flex flex-wrap items-center gap-1">
            {commit.parent_ids.length === 0 && (
              <span className="muted font-sans">none (root commit)</span>
            )}
            {!isMerge &&
              commit.parent_ids.map((p) => (
                <span key={p}>{p.slice(0, 10)}</span>
              ))}
            {isMerge &&
              commit.parent_ids.map((p, i) => (
                <button
                  type="button"
                  key={p}
                  className="row border py-0"
                  aria-pressed={i === parentIndex}
                  title={`Show changes relative to parent ${i + 1}`}
                  onClick={() => setParentIndex(i)}
                >
                  <span className="font-sans">Parent {i + 1}</span>
                  {p.slice(0, 10)}
                </button>
              ))}
          </dd>
          {commit.decorations.length > 0 && (
            <>
              <dt className="muted">Refs</dt>
              <dd className="mono">{commit.decorations.join(", ")}</dd>
            </>
          )}
        </dl>
        <FileList
          detail={detail}
          parentIndex={parentIndex}
          selectedPath={selectedPath}
          onSelect={setSelectedPath}
        />
      </div>
      <DiffView
        diff={diff}
        loading={diffLoading}
        empty={
          !detail
            ? "Loading…"
            : detail.files.length === 0
              ? "This commit changes no files relative to the compared parent."
              : "Select a file to see its patch."
        }
        openLabel="Open current file"
        onOpenInEditor={onOpenInEditor}
      />
    </div>
  );
}

function FileList({
  detail,
  parentIndex,
  selectedPath,
  onSelect,
}: {
  detail: CommitDetail | null;
  parentIndex: number;
  selectedPath: string | null;
  onSelect: (path: string) => void;
}) {
  if (!detail || detail.compared_parent_index !== parentIndex)
    return <div className="muted mt-4">Loading changed files…</div>;
  const additions = detail.files.reduce((n, f) => n + (f.additions ?? 0), 0);
  const deletions = detail.files.reduce((n, f) => n + (f.deletions ?? 0), 0);
  return (
    <div className="mt-4">
      <div className="muted mb-1 text-[11px] font-semibold uppercase tracking-wide">
        {detail.files.length} {detail.files.length === 1 ? "file" : "files"}{" "}
        changed
        <span className="ml-2 font-normal normal-case">
          <span className="text-emerald-600 dark:text-emerald-400">
            +{additions}
          </span>{" "}
          <span className="text-red-600 dark:text-red-400">−{deletions}</span>
        </span>
      </div>
      {detail.files.map((f) => (
        <FileRow
          key={f.path}
          file={f}
          selected={f.path === selectedPath}
          onSelect={() => onSelect(f.path)}
        />
      ))}
    </div>
  );
}

function FileRow({
  file,
  selected,
  onSelect,
}: {
  file: CommitFile;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      className="row w-full text-left"
      aria-pressed={selected}
      onClick={onSelect}
      title={file.old_path ? `${file.old_path} → ${file.path}` : file.path}
    >
      <span className={`mono w-4 shrink-0 text-center ${kindColor(file.kind)}`}>
        {KIND_LETTER[file.kind]}
      </span>
      <span className="mono truncate">
        {file.old_path && <span className="muted">{file.old_path} → </span>}
        {file.path}
      </span>
      <span className="mono ml-auto shrink-0 text-[11px]">
        {file.is_binary ? (
          <span className="badge muted border">binary</span>
        ) : (
          <>
            <span className="text-emerald-600 dark:text-emerald-400">
              +{file.additions ?? 0}
            </span>{" "}
            <span className="text-red-600 dark:text-red-400">
              −{file.deletions ?? 0}
            </span>
          </>
        )}
      </span>
    </button>
  );
}
