import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { absoluteTime, relativeTime } from "../lib/format";
import {
  type CommitDetail,
  type CommitFile,
  type CommitSummary,
  errorMessage,
  ipc,
} from "../lib/ipc";
import { usePref } from "../lib/prefs";
import {
  barWidths,
  decorations,
  groupFilesByDir,
  KIND_LETTER,
  kindTone,
  plural,
  splitPath,
} from "../lib/repo";
import { createLatest } from "../lib/stale";
import { useDiff } from "../lib/useDiff";
import DiffView from "./DiffView";
import { Avatar } from "./HistoryTab";
import { CopyIcon, FolderIcon, SidebarIcon } from "./icons";
import { OrderSwitch, useExplainedPatch } from "./useExplainedPatch";

type Props = {
  repositoryId: string;
  /** Render with `key={commit.id}` so state resets when the selection changes. */
  commit: CommitSummary;
  onError: (message: string | null) => void;
  onOpenInEditor: (path: string, line?: number) => void;
  /** Jump to another commit, such as a parent, when it is in the loaded list. */
  onSelectCommit: (id: string) => void;
  /** A short message in the window, such as "Saved as …". */
  onNotice?: (message: string) => void;
};

/** Message lines shown before "Show full message". */
const BODY_PREVIEW_LINES = 3;

export default function CommitDetails({
  repositoryId,
  commit,
  onError,
  onOpenInEditor,
  onSelectCommit,
  onNotice,
}: Props) {
  const [parentIndex, setParentIndex] = useState(0);
  const [detail, setDetail] = useState<CommitDetail | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [fullBody, setFullBody] = useState(false);
  const [showFiles, setShowFiles] = usePref("brainiac.history.files", true);
  const detailLatest = useRef(createLatest()).current;

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
            : (groupFilesByDir(d.files)[0]?.files[0]?.path ?? null),
        );
      },
      (e) => onError(errorMessage(e)),
    );
  }, [repositoryId, commit.id, parentIndex, detailLatest, onError]);

  const selectedFile =
    detail?.files.find((f) => f.path === selectedPath) ?? null;
  const {
    diff,
    loading: diffLoading,
    ignoreWhitespace,
    setIgnoreWhitespace,
  } = useDiff(
    repositoryId,
    detail && selectedFile
      ? {
          kind: "commit",
          commit_id: detail.id,
          path: selectedFile.path,
          old_path: selectedFile.old_path,
          parent_index: detail.compared_parent_index,
        }
      : null,
    detail,
    onError,
  );

  const copy = (text: string) => void navigator.clipboard?.writeText(text);
  const isMerge = commit.parent_ids.length > 1;
  const committerDiffers =
    detail &&
    (detail.committer_name !== commit.author_name ||
      detail.committer_email !== commit.author_email);
  const bodyLines = detail?.body.trim() ? detail.body.trim().split("\n") : [];
  const hiddenLines = fullBody
    ? 0
    : Math.max(0, bodyLines.length - BODY_PREVIEW_LINES);
  const comparedParent = commit.parent_ids[parentIndex];
  // Files in the order the tree shows them, for the stepper and [ ] keys;
  // with an explanation, in its reading order unless Path is chosen.
  const byPath = useMemo(
    () => (detail ? groupFilesByDir(detail.files).flatMap((g) => g.files) : []),
    [detail],
  );
  const subject = useMemo(
    () => ({ kind: "commit" as const, reference: commit.id }),
    [commit.id],
  );
  const explained = useExplainedPatch({
    repositoryId,
    subject,
    files: byPath,
    selectedPath,
    onSelectFile: setSelectedPath,
    onNotice,
  });
  const reading = explained.hasExplanation && explained.order === "reading";
  const files = explained.ordered;
  const fileIndex = files.findIndex((f) => f.path === selectedPath);
  const stepFile = (delta: number) => {
    const next = files[fileIndex + delta];
    if (next) setSelectedPath(next.path);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex max-h-[45%] shrink-0 flex-col gap-2.5 overflow-y-auto border-b px-[22px] pt-4 pb-3.5">
        <div className="flex items-start gap-4">
          <h1 className="selectable m-0 flex-1 text-[17px] leading-snug font-semibold">
            {commit.subject}
          </h1>
          <button
            type="button"
            className="btn btn-sm mono bg-control text-fg"
            title={`Copy ${commit.id}`}
            onClick={() => copy(commit.id)}
          >
            {commit.short_id}
            <CopyIcon size={12} />
          </button>
        </div>
        <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[12px] text-fg-2">
          <Avatar name={commit.author_name} size={20} />
          <span className="text-fg" title={commit.author_email}>
            {commit.author_name}
          </span>
          <span>committed {relativeTime(commit.committed_at)}</span>
          <span className="text-muted">
            {absoluteTime(commit.committed_at)}
          </span>
          {committerDiffers && (
            <span className="text-muted">by {detail.committer_name}</span>
          )}
          <Sep />
          {commit.parent_ids.length === 0 && (
            <span className="text-muted">Root commit</span>
          )}
          {!isMerge &&
            commit.parent_ids.map((p) => (
              <span key={p} className="flex items-center gap-2">
                <span className="text-muted">Parent</span>
                <button
                  type="button"
                  className="mono text-link hover:underline"
                  title="Select this commit when it is in the list"
                  onClick={() => onSelectCommit(p)}
                >
                  {p.slice(0, 7)}
                </button>
              </span>
            ))}
          {isMerge && (
            <span className="flex items-center gap-1.5">
              <span className="text-muted">Compare with</span>
              <span className="seg" role="tablist" aria-label="Compared parent">
                {commit.parent_ids.map((p, i) => (
                  <button
                    type="button"
                    role="tab"
                    key={p}
                    aria-selected={i === parentIndex}
                    title={`Show changes relative to parent ${i + 1}`}
                    onClick={() => setParentIndex(i)}
                  >
                    Parent {i + 1}
                    <span className="mono text-[11px]">{p.slice(0, 7)}</span>
                  </button>
                ))}
              </span>
            </span>
          )}
          {commit.decorations.length > 0 && <Sep />}
          {decorations(commit).map((d) => (
            <span key={d.label} className="deco" data-tone={d.tone}>
              {d.label}
            </span>
          ))}
        </div>
        {bodyLines.length > 0 && (
          <div className="selectable max-w-[760px] text-[12.5px] leading-normal whitespace-pre-wrap text-fg-3">
            {bodyLines
              .slice(0, fullBody ? undefined : BODY_PREVIEW_LINES)
              .join("\n")}
          </div>
        )}
        {(hiddenLines > 0 || fullBody) &&
          bodyLines.length > BODY_PREVIEW_LINES && (
            <button
              type="button"
              className="self-start text-[12px] text-link hover:underline"
              onClick={() => setFullBody(!fullBody)}
            >
              {fullBody
                ? "Show less"
                : `Show full message (${plural(hiddenLines, "more line")})`}
            </button>
          )}
      </div>

      <div className="flex min-h-0 flex-1">
        {showFiles && (
          <FileTree
            detail={detail}
            parentIndex={parentIndex}
            selectedPath={selectedPath}
            onSelect={setSelectedPath}
            onHide={() => setShowFiles(false)}
            reading={reading ? files : null}
            orderSwitch={
              explained.hasExplanation && (
                <OrderSwitch
                  order={explained.order}
                  setOrder={explained.setOrder}
                />
              )
            }
          />
        )}
        <div className="flex min-w-0 flex-1 flex-col">
          <DiffView
            diff={diff}
            loading={diffLoading}
            historical
            ignoreWhitespace={ignoreWhitespace}
            onIgnoreWhitespace={setIgnoreWhitespace}
            stepper={
              files.length > 0 && fileIndex >= 0
                ? {
                    index: fileIndex,
                    total: files.length,
                    onPrev: () => stepFile(-1),
                    onNext: () => stepFile(1),
                  }
                : undefined
            }
            annotate={explained.annotate}
            extra={
              <>
                {!showFiles && (
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => setShowFiles(true)}
                  >
                    <SidebarIcon size={13} />
                    Show files
                  </button>
                )}
                {explained.toolbar}
              </>
            }
            meta={
              comparedParent
                ? `vs parent ${comparedParent.slice(0, 7)}`
                : "vs empty tree"
            }
            empty={
              !detail
                ? "Loading…"
                : detail.files.length === 0
                  ? "This commit changes no files relative to the compared parent."
                  : "Select a file to see its patch."
            }
            onOpenInEditor={onOpenInEditor}
          />
        </div>
        {explained.panel}
      </div>
      {explained.dialog}
    </div>
  );
}

function Sep() {
  return <span className="text-faint">|</span>;
}

function FileTree({
  detail,
  parentIndex,
  selectedPath,
  onSelect,
  onHide,
  reading,
  orderSwitch,
}: {
  detail: CommitDetail | null;
  parentIndex: number;
  selectedPath: string | null;
  onSelect: (path: string) => void;
  onHide: () => void;
  /** The files in reading order, listed flat instead of by folder. */
  reading: CommitFile[] | null;
  orderSwitch?: ReactNode;
}) {
  const ready = detail && detail.compared_parent_index === parentIndex;
  const additions = detail?.files.reduce((n, f) => n + (f.additions ?? 0), 0);
  const deletions = detail?.files.reduce((n, f) => n + (f.deletions ?? 0), 0);
  return (
    <div className="flex w-64 shrink-0 flex-col border-r bg-panel-2">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b pr-1.5 pl-3 text-[12px]">
        {ready ? (
          <>
            <span className="font-semibold">
              {plural(detail.files.length, "file")}
            </span>
            <span className="text-add">+{additions}</span>
            <span className="text-del">−{deletions}</span>
          </>
        ) : (
          <span className="text-muted">Loading files…</span>
        )}
        <span className="flex-1" />
        <button
          type="button"
          className="btn btn-sm btn-ghost w-6 px-0"
          aria-label="Hide file list"
          title="Hide file list, giving the patch the full width"
          onClick={onHide}
        >
          <SidebarIcon size={13} />
        </button>
      </div>
      {orderSwitch && (
        <div className="flex shrink-0 justify-center border-b px-2 py-1.5">
          {orderSwitch}
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto py-1.5">
        {ready &&
          reading?.map((f) => (
            <FileRow
              key={f.path}
              file={f}
              nested={false}
              full
              selected={f.path === selectedPath}
              onSelect={() => onSelect(f.path)}
            />
          ))}
        {ready &&
          !reading &&
          groupFilesByDir(detail.files).map((g) => (
            <div key={g.dir}>
              {g.dir && (
                <div
                  className="flex h-[26px] items-center gap-[7px] px-3 text-[12.5px] text-fg-2"
                  title={g.dir}
                >
                  <FolderIcon size={13} className="shrink-0 text-muted" />
                  <span className="truncate">{g.dir}</span>
                </div>
              )}
              {g.files.map((f) => (
                <FileRow
                  key={f.path}
                  file={f}
                  nested={!!g.dir}
                  selected={f.path === selectedPath}
                  onSelect={() => onSelect(f.path)}
                />
              ))}
            </div>
          ))}
      </div>
    </div>
  );
}

function FileRow({
  file,
  nested,
  selected,
  onSelect,
  full = false,
}: {
  file: CommitFile;
  nested: boolean;
  selected: boolean;
  onSelect: () => void;
  /** Show the whole path, as reading order lists files flat. */
  full?: boolean;
}) {
  const tone = kindTone(file.kind);
  const bar = barWidths(file.additions, file.deletions);
  const stats = file.is_binary
    ? "binary"
    : `+${file.additions ?? 0} −${file.deletions ?? 0}`;
  return (
    <button
      type="button"
      className={`list-row h-[26px] items-center gap-[7px] pr-3 text-[12.5px] ${nested ? "pl-[25px]" : "pl-[9px]"}`}
      aria-current={selected}
      onClick={onSelect}
      title={`${file.old_path ? `${file.old_path} → ${file.path}` : file.path} (${stats})`}
    >
      <span
        className="mono w-[13px] shrink-0 text-center text-[11px] font-semibold"
        style={{ color: `var(--k-${tone}-fg)` }}
      >
        {KIND_LETTER[file.kind]}
      </span>
      <span className="min-w-0 flex-1 truncate">
        {full ? file.path : splitPath(file.path).name}
      </span>
      {file.is_binary ? (
        <span className="text-[11px] text-muted">binary</span>
      ) : (
        <span className="change-bar" aria-hidden="true">
          <span className="a" style={{ width: `${bar.add}%` }} />
          <span className="d" style={{ width: `${bar.del}%` }} />
        </span>
      )}
    </button>
  );
}
