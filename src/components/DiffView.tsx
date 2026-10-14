import {
  Fragment,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  hunkAtRow,
  hunkRowIndexes,
  longestLines,
  type Segment,
  type SplitRow,
  splitRows,
  type UnifiedRow,
  unifiedRows,
  wordHighlighter,
} from "../lib/diff";
import type { DiffLine, DiffResult, DiffSide, Hunk } from "../lib/ipc";
import { useKeys } from "../lib/keys";
import { usePref } from "../lib/prefs";
import { splitPath } from "../lib/repo";
import { type Landing, resolveLanding } from "../lib/reviewNav";
import { VIRTUALIZE_ABOVE, visibleRange } from "../lib/virtual";
import { useNarrowerThan } from "../lib/width";
import { ChevronDown, CopyIcon, ExternalIcon } from "./icons";
import Popover from "./Popover";

/** Position in a file list, with moves; shown as "‹ 1 / 7 ›" and bound to [ and ]. */
export type Stepper = {
  index: number;
  total: number;
  onPrev: () => void;
  onNext: () => void;
};

/** Where a reviewer had scrolled to and what they had selected in one file's patch. */
export type ViewState = {
  scrollTop: number;
  /** The change counted as current, which a short file cannot scroll to. */
  hunk: number;
  sel: Selection | null;
};

/**
 * Review navigation over a patch (SPEC.md, Files Changed): files and changes
 * together in one toolbar, bound to `J` / `K` and `Shift+J` / `Shift+K`.
 * The patch moves between its own hunks; crossing into another file is the
 * owner's, which knows the order.
 */
export type ReviewNav = {
  file: {
    index: number;
    total: number;
    onPrev: () => void;
    onNext: () => void;
  };
  /** A change exists in the next or previous file when this one runs out. */
  moreAfter: boolean;
  moreBefore: boolean;
  /** `J` past the last hunk or `K` before the first. */
  onCross: (delta: 1 | -1) => void;
  /** A move to a change in a file that was not shown; the patch scrolls to it once it is. */
  land: { path: string; hunk: Landing; token: number } | null;
  onLanded: (token: number) => void;
  /** Where this file was left, for a return by the file list. */
  restore: (ViewState & { path: string }) | null;
  onViewState: (path: string, state: ViewState) => void;
  /** The current change and how many there are, for the footer. */
  onPosition: (hunk: number, count: number) => void;
  /** Says where a move crossing files ended up. */
  notice: string | null;
};

/** A line, or a range ending at it, a comment goes on. */
export type LineTarget = {
  side: DiffSide;
  line: number;
  start_line: number | null;
};

/**
 * Pull request review over a patch: what to show under lines (threads,
 * drafts, the comment box), and the gutter button and `C` that start a
 * comment on the selected line or range.
 */
export type Annotate = {
  render: (line: DiffLine) => ReactNode | null;
  /** Absent when commenting is off; `note` then says why. */
  onComment?: (target: LineTarget) => void;
  note?: string;
};

type Props = {
  diff: DiffResult | null;
  /** A request is in flight; the previous patch stays visible under a progress line. */
  loading: boolean;
  empty: string;
  onOpenInEditor: (path: string, line?: number) => void;
  /** Commit patches open the current working-tree file, so they say so and skip the line. */
  historical?: boolean;
  /** Short context after the stats, such as "vs parent 65f2666". */
  meta?: ReactNode;
  /** Optional controls at the start of the second header row, such as the comparison switch. */
  extra?: ReactNode;
  stepper?: Stepper;
  ignoreWhitespace: boolean;
  onIgnoreWhitespace: (on: boolean) => void;
  /** Why whitespace cannot be ignored for this patch (a provider's diff); the button is then off. */
  ignoreWhitespaceNote?: string;
  annotate?: Annotate;
  /** Pull request review: replaces `stepper` and the N P [ ] keys. */
  review?: ReviewNav;
};

/** Must match `.diff-row` in index.css. */
const ROW_HEIGHT = 20;

type Layout = "unified" | "split";

export function diffStats(hunks: Hunk[]): { add: number; del: number } {
  let add = 0;
  let del = 0;
  for (const h of hunks)
    for (const l of h.lines) {
      if (l.kind === "add") add++;
      else if (l.kind === "delete") del++;
    }
  return { add, del };
}

/** First new-side line number that changed, for "Open at line N". */
export function firstChangedLine(hunks: Hunk[]): number | undefined {
  for (const h of hunks)
    for (const l of h.lines)
      if (l.kind !== "context" && l.new_no !== null) return l.new_no;
  return hunks[0]?.new_start || undefined;
}

export default function DiffView({
  diff,
  loading,
  empty,
  onOpenInEditor,
  historical = false,
  meta,
  extra,
  stepper,
  ignoreWhitespace,
  onIgnoreWhitespace,
  ignoreWhitespaceNote,
  annotate,
  review,
}: Props) {
  const [wrap, setWrap] = usePref("brainiac.diff.wrap", false);
  const [layout, setLayout] = usePref<Layout>(
    "brainiac.diff.layout",
    "unified",
  );
  const [hunkIndex, setHunkIndex] = useState(0);
  const [viewOpen, setViewOpen] = useState(false);
  // Split needs two readable sides: below 900px of pane it shows unified.
  const [measure, tooNarrowToSplit] = useNarrowerThan(900);
  const jumpRef = useRef<((i: number) => void) | null>(null);
  const reviewRef = useRef(review);
  reviewRef.current = review;
  const hunks = diff?.content.kind === "text" ? diff.content.hunks : null;
  const rowCount = useMemo(
    () => (hunks ? hunks.reduce((n, h) => n + 1 + h.lines.length, 0) : 0),
    [hunks],
  );
  const hunkCount = hunks?.length ?? 0;

  const jump = useCallback(
    (delta: number) => {
      if (!hunkCount) return;
      const next = Math.min(hunkCount - 1, Math.max(0, hunkIndex + delta));
      jumpRef.current?.(next);
    },
    [hunkCount, hunkIndex],
  );
  /** The next or previous change: in this file, else the first or last of the one beyond. */
  const change = useCallback(
    (delta: 1 | -1) => {
      if (!review || loading) return;
      const to = hunkIndex + delta;
      if (to >= 0 && to < hunkCount) jumpRef.current?.(to);
      else review.onCross(delta);
    },
    [review, loading, hunkIndex, hunkCount],
  );
  // In a pull request review, J and K move by change and Shift+J / Shift+K
  // by file; elsewhere N and P move by hunk and [ and ] by file.
  useKeys(
    review
      ? {
          j: () => change(1),
          k: () => change(-1),
          "shift+j": () => review.file.onNext(),
          "shift+k": () => review.file.onPrev(),
        }
      : {
          n: () => jump(1),
          p: () => jump(-1),
          "[": () => stepper?.onPrev(),
          "]": () => stepper?.onNext(),
        },
  );
  const onPosition = review?.onPosition;
  useEffect(() => {
    onPosition?.(hunkIndex, hunkCount);
  }, [onPosition, hunkIndex, hunkCount]);

  // A move into this file from another: scroll to the change it asked for
  // once the patch is here. A file with no changes to land on is the
  // owner's to step past.
  const land = review?.land;
  const shownPath = diff?.selector.path;
  useEffect(() => {
    if (!land || loading || !shownPath || shownPath !== land.path || !hunkCount)
      return;
    const i = resolveLanding(land.hunk, hunkCount);
    if (i === null) return;
    jumpRef.current?.(i);
    reviewRef.current?.onLanded(land.token);
  }, [land, loading, shownPath, hunkCount]);

  if (!diff)
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        {(extra || stepper || review) && (
          <div className="flex items-center gap-3 border-b px-4 py-2.5">
            {review ? (
              <ReviewToolbar
                review={review}
                hunkIndex={0}
                hunkCount={0}
                onChange={change}
              />
            ) : (
              stepper && <StepperView stepper={stepper} />
            )}
            {extra}
          </div>
        )}
        <div className="p-6 text-muted">
          {loading ? <DiffSkeleton /> : empty}
        </div>
      </div>
    );

  const path = diff.selector.path;
  const { dir, name } = splitPath(path);
  const copy = (text: string) => void navigator.clipboard?.writeText(text);
  const stats = hunks ? diffStats(hunks) : null;
  const line = !historical && hunks ? firstChangedLine(hunks) : undefined;
  // Windowing needs fixed row heights, so very long patches cannot wrap.
  const canWrap = rowCount <= VIRTUALIZE_ABOVE;
  const wrapped = wrap && canWrap;
  const oldPath =
    diff.content.kind === "text" &&
    diff.content.old_path &&
    diff.content.old_path !== path
      ? diff.content.old_path
      : null;

  const shownLayout: Layout = tooNarrowToSplit ? "unified" : layout;
  const splitTitle = tooNarrowToSplit
    ? "Split needs a pane at least 900 pixels wide"
    : undefined;
  const openLabel = historical
    ? "Open current file"
    : line
      ? `Open at line ${line}`
      : "Open in editor";
  const header = (
    <div ref={measure} className="@container flex shrink-0 flex-col border-b">
      <div className="flex min-w-0 items-center gap-2.5 px-3 pt-2 pb-1.5">
        {review ? (
          <ReviewToolbar
            review={review}
            hunkIndex={hunkIndex}
            hunkCount={hunkCount}
            onChange={change}
          />
        ) : (
          stepper && <StepperView stepper={stepper} />
        )}
        <span className="mono selectable min-w-0 truncate pl-1 text-[13px]">
          {oldPath && <span className="text-muted">{oldPath} → </span>}
          <span className="text-muted">{dir}</span>
          <span className="font-semibold">{name}</span>
        </span>
        <button
          type="button"
          className="btn btn-sm btn-ghost px-1.5"
          aria-label="Copy path"
          title="Copy path"
          onClick={() => copy(path)}
        >
          <CopyIcon />
        </button>
        {stats && (
          <span className="shrink-0 text-[12px]">
            <span className="text-add">+{stats.add}</span>{" "}
            <span className="text-del">−{stats.del}</span>
          </span>
        )}
        {meta && (
          <span className="shrink-0 truncate text-[12px] text-muted">
            {meta}
          </span>
        )}
        <span className="flex-1" />
        <button
          type="button"
          className="btn btn-sm shrink-0"
          aria-label={openLabel}
          title={openLabel}
          onClick={() => onOpenInEditor(path, line)}
        >
          <ExternalIcon size={12} className="hidden @max-[700px]:inline" />
          <span className="@max-[700px]:hidden">{openLabel}</span>
        </button>
      </div>
      <div className="flex min-h-[34px] min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5 px-4 pb-2">
        {extra}
        {annotate?.note && !annotate.onComment && (
          // A disabled button shows no tooltip, so why commenting is off is
          // said here, once, rather than on each line's "+".
          <span
            className="min-w-0 truncate text-[12px] text-muted"
            title={annotate.note}
          >
            Comments off: {annotate.note}
          </span>
        )}
        <span className="flex-1" />
        {!review && hunkCount > 1 && (
          <span className="flex items-center gap-1.5 text-[12px] text-muted">
            <span className="tabular" aria-live="polite">
              <span className="@max-[760px]:hidden">
                Hunk {hunkIndex + 1} of {hunkCount}
              </span>
              <span className="hidden @max-[760px]:inline">
                {hunkIndex + 1}/{hunkCount}
              </span>
            </span>
            <button
              type="button"
              className="btn btn-sm w-6 px-0"
              aria-label="Previous hunk (P)"
              title="Previous hunk (P)"
              disabled={hunkIndex === 0}
              onClick={() => jump(-1)}
            >
              ↑
            </button>
            <button
              type="button"
              className="btn btn-sm w-6 px-0"
              aria-label="Next hunk (N)"
              title="Next hunk (N)"
              disabled={hunkIndex >= hunkCount - 1}
              onClick={() => jump(1)}
            >
              ↓
            </button>
          </span>
        )}
        {/* Below 760px these go into the View menu. */}
        <span className="flex items-center gap-3 @max-[760px]:hidden">
          {hunks && (
            <div role="tablist" aria-label="Diff layout" className="seg seg-sm">
              <button
                type="button"
                role="tab"
                aria-selected={shownLayout === "unified"}
                onClick={() => setLayout("unified")}
              >
                Unified
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={shownLayout === "split"}
                disabled={tooNarrowToSplit}
                title={splitTitle}
                onClick={() => setLayout("split")}
              >
                Split
              </button>
            </div>
          )}
          {hunks && (
            <button
              type="button"
              className="btn btn-sm"
              aria-pressed={wrapped}
              disabled={!canWrap}
              title={
                canWrap ? "Wrap long lines" : "This patch is too long to wrap"
              }
              onClick={() => setWrap(!wrap)}
            >
              Wrap
            </button>
          )}
          {diff.selector.kind !== "untracked_preview" && (
            <button
              type="button"
              className="btn btn-sm"
              aria-pressed={ignoreWhitespace && !ignoreWhitespaceNote}
              disabled={!!ignoreWhitespaceNote}
              title={
                ignoreWhitespaceNote ??
                "Compare lines ignoring whitespace (git diff -w)"
              }
              onClick={() => onIgnoreWhitespace(!ignoreWhitespace)}
            >
              Ignore whitespace
            </button>
          )}
        </span>
        <div className="relative hidden @max-[760px]:block">
          <button
            type="button"
            className="btn btn-sm"
            aria-haspopup="menu"
            aria-expanded={viewOpen}
            onClick={() => setViewOpen(!viewOpen)}
          >
            View
            <ChevronDown size={10} className="text-muted" />
          </button>
          {viewOpen && (
            <Popover align="right" onClose={() => setViewOpen(false)}>
              {hunks && (
                <>
                  <ViewItem
                    checked={shownLayout === "unified"}
                    onClick={() => setLayout("unified")}
                  >
                    Unified
                  </ViewItem>
                  <ViewItem
                    checked={shownLayout === "split"}
                    disabled={tooNarrowToSplit}
                    title={splitTitle}
                    onClick={() => setLayout("split")}
                  >
                    Split
                  </ViewItem>
                  <div className="menu-sep" />
                  <ViewItem
                    checked={wrapped}
                    disabled={!canWrap}
                    onClick={() => setWrap(!wrap)}
                  >
                    Wrap long lines
                  </ViewItem>
                </>
              )}
              {diff.selector.kind !== "untracked_preview" && (
                <ViewItem
                  checked={ignoreWhitespace && !ignoreWhitespaceNote}
                  disabled={!!ignoreWhitespaceNote}
                  title={ignoreWhitespaceNote ?? undefined}
                  onClick={() => onIgnoreWhitespace(!ignoreWhitespace)}
                >
                  Ignore whitespace
                </ViewItem>
              )}
            </Popover>
          )}
        </div>
      </div>
    </div>
  );

  if (diff.content.kind === "non_text") {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        {header}
        <div className="relative p-6">
          {loading && <div className="progress-line" />}
          <div className="font-medium">{reasonLabel(diff.content.reason)}</div>
          <div className="selectable text-muted">{diff.content.summary}</div>
        </div>
      </div>
    );
  }

  const { truncated, total_lines } = diff.content;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {header}
      <div className="relative flex min-h-0 flex-1 flex-col">
        {loading && <div className="progress-line" />}
        {diff.content.hunks.length === 0 ? (
          <div className="p-6 text-muted">
            {oldPath
              ? "Renamed without content changes."
              : ignoreWhitespace
                ? "Only whitespace differs."
                : "No textual differences."}
          </div>
        ) : (
          <PatchRows
            key={`${JSON.stringify(diff.selector)}:${shownLayout}`}
            hunks={diff.content.hunks}
            layout={shownLayout}
            wrap={wrapped}
            onHunk={setHunkIndex}
            jumpRef={jumpRef}
            annotate={annotate}
            restore={
              review?.restore && review.restore.path === path
                ? review.restore
                : null
            }
            onViewState={
              review ? (state) => review.onViewState(path, state) : undefined
            }
            footer={
              truncated && (
                <div className="m-3 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 font-sans text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
                  Output truncated
                  {total_lines !== null
                    ? ` (${total_lines} lines in total)`
                    : ""}
                  . Open the file in your editor for the full content.
                </div>
              )
            }
          />
        )}
      </div>
    </div>
  );
}

function StepperView({ stepper }: { stepper: Stepper }) {
  return (
    <span className="flex shrink-0 items-center rounded-md border border-control-line">
      <button
        type="button"
        className="h-6 w-6 rounded-l-md text-fg-2 hover:bg-control disabled:opacity-40"
        aria-label="Previous file ([)"
        title="Previous file ([)"
        disabled={stepper.index <= 0}
        onClick={stepper.onPrev}
      >
        ‹
      </button>
      <span className="tabular px-1 text-[12px] text-fg-2">
        {stepper.index + 1} / {stepper.total}
      </span>
      <button
        type="button"
        className="h-6 w-6 rounded-r-md text-fg-2 hover:bg-control disabled:opacity-40"
        aria-label="Next file (])"
        title="Next file (])"
        disabled={stepper.index >= stepper.total - 1}
        onClick={stepper.onNext}
      >
        ›
      </button>
    </span>
  );
}

/** "‹ File 1 of 42 ›   ↑ Change 1 of 2 ↓": the review's two kinds of step, side by side. */
function ReviewToolbar({
  review,
  hunkIndex,
  hunkCount,
  onChange,
}: {
  review: ReviewNav;
  hunkIndex: number;
  hunkCount: number;
  onChange?: (delta: 1 | -1) => void;
}) {
  const { file } = review;
  const canPrevChange = hunkIndex > 0 || review.moreBefore;
  const canNextChange = hunkIndex < hunkCount - 1 || review.moreAfter;
  const step =
    "h-6 w-6 text-fg-2 hover:bg-control disabled:opacity-40 disabled:hover:bg-transparent";
  return (
    <span
      role="toolbar"
      aria-label="Files and changes"
      className="flex shrink-0 items-center gap-2"
    >
      <span className="flex items-center rounded-md border border-control-line">
        <button
          type="button"
          className={`${step} rounded-l-md`}
          aria-label="Previous file (Shift+K)"
          title="Previous file (Shift+K)"
          disabled={file.index <= 0}
          onClick={file.onPrev}
        >
          ‹
        </button>
        <span className="tabular px-1 text-[12px] text-fg-2">
          File {file.index + 1} of {file.total}
        </span>
        <button
          type="button"
          className={`${step} rounded-r-md`}
          aria-label="Next file (Shift+J)"
          title="Next file (Shift+J)"
          disabled={file.index >= file.total - 1}
          onClick={file.onNext}
        >
          ›
        </button>
      </span>
      <span
        className="flex items-center rounded-md border border-control-line"
        title="A change is one hunk of the diff: a run of changed lines with its context"
      >
        <button
          type="button"
          className={`${step} rounded-l-md`}
          aria-label="Previous change (K)"
          title="Previous change (K)"
          disabled={!canPrevChange}
          onClick={() => onChange?.(-1)}
        >
          ↑
        </button>
        <span className="tabular px-1 text-[12px] text-fg-2" aria-live="polite">
          {hunkCount > 0
            ? `Change ${hunkIndex + 1} of ${hunkCount}`
            : "No changes to step"}
        </span>
        <button
          type="button"
          className={`${step} rounded-r-md`}
          aria-label="Next change (J)"
          title="Next change (J)"
          disabled={!canNextChange}
          onClick={() => onChange?.(1)}
        >
          ↓
        </button>
      </span>
      {review.notice && (
        <span
          className="mono max-w-[260px] truncate text-[12px] text-accent"
          role="status"
        >
          {review.notice}
        </span>
      )}
    </span>
  );
}

export function DiffSkeleton() {
  return (
    <div
      className="flex flex-col gap-2.5"
      role="status"
      aria-label="Loading diff"
    >
      {[70, 45, 82, 60, 38].map((w) => (
        <div key={w} className="skeleton" style={{ width: `${w}%` }} />
      ))}
    </div>
  );
}

/** Lines of one side selected in the gutter, for a comment on them. */
export type Selection = { side: DiffSide; from: number; to: number };

function sideNo(l: DiffLine, side: DiffSide): number | null {
  return side === "old" ? l.old_no : l.new_no;
}

function inSelection(l: DiffLine, sel: Selection | null): boolean {
  if (!sel) return false;
  const n = sideNo(l, sel.side);
  return n !== null && n >= sel.from && n <= sel.to;
}

function targetOf(sel: Selection): LineTarget {
  return {
    side: sel.side,
    line: sel.to,
    start_line: sel.from < sel.to ? sel.from : null,
  };
}

/**
 * Scrollable patch body. Only rows near the viewport are in the DOM for long
 * diffs. Tracks which hunk is at the top, pins its header, and exposes a
 * jump function for hunk navigation. Annotated patches (threads under lines)
 * have rows of varying height, so they render whole and are measured.
 */
function PatchRows({
  hunks,
  layout,
  wrap,
  onHunk,
  jumpRef,
  annotate,
  footer,
  restore,
  onViewState,
}: {
  restore: ViewState | null;
  onViewState?: (state: ViewState) => void;
  hunks: Hunk[];
  layout: Layout;
  wrap: boolean;
  onHunk: (index: number) => void;
  jumpRef: React.RefObject<((i: number) => void) | null>;
  annotate?: Annotate;
  footer: ReactNode;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(800);
  const [pinned, setPinned] = useState<number | null>(null);
  const [sel, setSel] = useState<Selection | null>(restore?.sel ?? null);
  // The change an explicit jump landed on, which stays the current one while
  // the scroll position is where the jump put it: the last change of a short
  // file cannot always reach the top of the view.
  const forced = useRef<{ index: number; top: number } | null>(null);
  const hunkNow = useRef(restore?.hunk ?? 0);
  const [flash, setFlash] = useState<number | null>(null);
  const rows = useMemo<Array<UnifiedRow | SplitRow>>(
    () => (layout === "split" ? splitRows(hunks) : unifiedRows(hunks)),
    [hunks, layout],
  );
  const words = useMemo(() => wordHighlighter(hunks), [hunks]);
  const hunkRows = useMemo(() => hunkRowIndexes(rows), [rows]);
  const rowsRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(800);
  const longest = useMemo(() => longestLines(hunks), [hunks]);
  // Rows are measured, not counted, when their heights vary.
  const measured = wrap || !!annotate;

  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const measure = () => {
      setViewport(el.clientHeight);
      setWidth(el.clientWidth);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  /** Top offset of hunk `i`'s header row inside the scroller. */
  const hunkTop = useCallback(
    (i: number) => {
      if (!measured) return (hunkRows[i] ?? 0) * ROW_HEIGHT;
      // Only the rows themselves, never the pinned copy above them.
      const el = rowsRef.current?.querySelector<HTMLElement>(
        `[data-hunk="${i}"]`,
      );
      return el ? el.offsetTop : 0;
    },
    [measured, hunkRows],
  );

  const track = useCallback(
    (top: number) => {
      setScrollTop(top);
      let current = 0;
      if (!measured) current = hunkAtRow(rows, Math.floor(top / ROW_HEIGHT));
      else
        for (let i = 0; i < hunkRows.length; i++)
          if (hunkTop(i) <= top + 1) current = i;
      const held = forced.current;
      if (held && Math.abs(top - held.top) <= 1) {
        hunkNow.current = held.index;
        onHunk(held.index);
        setPinned(null);
        return;
      }
      // The reviewer scrolled on their own: the position decides again.
      forced.current = null;
      hunkNow.current = current;
      onHunk(current);
      setPinned(top > hunkTop(current) + 1 ? current : null);
    },
    [measured, rows, hunkRows, hunkTop, onHunk],
  );

  // A reloaded patch (same file, new content) keeps its scroll position;
  // recompute which hunk that is, so the counter never points past the end.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs when the patch's hunks change.
  useEffect(() => {
    track(scroller.current?.scrollTop ?? 0);
  }, [hunks]);

  useEffect(() => {
    jumpRef.current = (i: number) => {
      const el = scroller.current;
      if (!el) return;
      el.scrollTop = hunkTop(i);
      forced.current = { index: i, top: el.scrollTop };
      track(el.scrollTop);
      setFlash(i);
    };
    return () => {
      jumpRef.current = null;
    };
  }, [jumpRef, hunkTop, track]);
  useEffect(() => {
    if (flash === null) return;
    const timer = setTimeout(() => setFlash(null), 1400);
    return () => clearTimeout(timer);
  }, [flash]);

  // Where the reviewer was, kept by the owner for a return to this file. The
  // first run restores it; the scroll then reports itself.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only on mount, when the patch was opened.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || !restore) return;
    if (restore.scrollTop > 0) el.scrollTop = restore.scrollTop;
    // The change that was current stays so, even where the scroll cannot show it.
    if (restore.hunk > 0)
      forced.current = { index: restore.hunk, top: el.scrollTop };
  }, []);
  useEffect(() => {
    onViewState?.({
      scrollTop: scroller.current?.scrollTop ?? 0,
      hunk: hunkNow.current,
      sel,
    });
  }, [sel, onViewState]);

  /** A click in the gutter selects the line; with Shift, the range up to it. */
  const pick = useCallback((side: DiffSide, line: number, extend: boolean) => {
    setSel((current) => {
      if (extend && current && current.side === side)
        return {
          side,
          from: Math.min(current.from, line),
          to: Math.max(current.to, line),
        };
      if (current && current.from === line && current.to === line && !extend)
        return null;
      return { side, from: line, to: line };
    });
  }, []);
  const onComment = annotate?.onComment;
  /** The gutter button: on the selection when the line is in it, else that line. */
  const comment = useCallback(
    (l: DiffLine, side: DiffSide) => {
      if (!onComment) return;
      const n = sideNo(l, side);
      if (n === null) return;
      const target: Selection = inSelection(l, sel)
        ? (sel as Selection)
        : { side, from: n, to: n };
      setSel(target);
      onComment(targetOf(target));
    },
    [onComment, sel],
  );
  useKeys({ c: () => sel && onComment?.(targetOf(sel)) }, !!onComment && !!sel);

  const windowed = measured
    ? { start: 0, end: rows.length }
    : visibleRange(rows.length, ROW_HEIGHT, scrollTop, viewport);
  const { start, end } = windowed;
  const pinnedHunk = pinned === null ? null : hunks[pinned];
  const extra = (l: DiffLine | null) => {
    if (!l || !annotate) return null;
    const node = annotate.render(l);
    return node ? <div className="diff-extra">{node}</div> : null;
  };
  const lineProps = {
    sel,
    onPick: pick,
    onComment: onComment ? comment : null,
    note: annotate?.note,
  };
  return (
    <div
      ref={scroller}
      tabIndex={-1}
      data-own-arrows
      className={`diff outline-none selectable relative min-h-0 flex-1 overflow-auto ${wrap ? "diff-wrap" : ""} ${layout === "split" ? "diff-split" : ""}`}
      onScroll={(e) => {
        const top = e.currentTarget.scrollTop;
        track(top);
        onViewState?.({ scrollTop: top, hunk: hunkNow.current, sel });
      }}
    >
      {pinnedHunk && (
        // Overlays the first visible row instead of pushing rows down.
        <div className="diff-pinned -mb-5" aria-hidden="true">
          <HunkRow hunk={pinnedHunk} />
        </div>
      )}
      <div
        ref={rowsRef}
        className={wrap ? "" : "min-w-max"}
        style={measured ? undefined : { height: rows.length * ROW_HEIGHT }}
      >
        <div
          style={
            measured
              ? undefined
              : { transform: `translateY(${start * ROW_HEIGHT}px)` }
          }
        >
          {rows.slice(start, end).map((row, i) =>
            row.kind === "hunk" ? (
              <HunkRow
                // biome-ignore lint/suspicious/noArrayIndexKey: rows have no identity beyond their position in one patch.
                key={start + i}
                hunk={row.hunk}
                index={row.index}
                flash={flash === row.index}
              />
            ) : row.kind === "line" ? (
              // biome-ignore lint/suspicious/noArrayIndexKey: see above.
              <Fragment key={start + i}>
                <LineRow
                  line={row.line}
                  words={words(row.line)}
                  {...lineProps}
                />
                {extra(row.line)}
              </Fragment>
            ) : (
              // biome-ignore lint/suspicious/noArrayIndexKey: see above.
              <Fragment key={start + i}>
                <PairRow
                  left={row.left}
                  right={row.right}
                  words={words}
                  widths={
                    wrap
                      ? null
                      : [
                          halfWidth(width, longest.old),
                          halfWidth(width, longest.new),
                        ]
                  }
                  {...lineProps}
                />
                {extra(row.left)}
                {row.right !== row.left && extra(row.right)}
              </Fragment>
            ),
          )}
        </div>
      </div>
      {footer}
    </div>
  );
}

/** A split column at least half the view wide and as wide as its longest line. */
function halfWidth(view: number, chars: number): string {
  // Gutter (44px), sign (20px), and the text's right padding (16px).
  return `max(${Math.floor(view / 2)}px, calc(80px + ${chars}ch))`;
}

/** `index` marks a header in the patch, which hunk navigation looks up. */
function HunkRow({
  hunk: h,
  index,
  flash = false,
}: {
  hunk: Hunk;
  index?: number;
  flash?: boolean;
}) {
  return (
    <div
      className="diff-row diff-hunk"
      data-hunk={index}
      data-flash={flash || undefined}
    >
      <span className="diff-gutter" />
      <span className="diff-gutter" />
      <span className="diff-sign" />
      <span className="diff-text">
        @@ -{h.old_start},{h.old_lines} +{h.new_start},{h.new_lines} @@{" "}
        {h.header}
      </span>
    </div>
  );
}

function lineClass(l: DiffLine): string {
  return l.kind === "add" ? "diff-add" : l.kind === "delete" ? "diff-del" : "";
}

function Text({ line, words }: { line: DiffLine; words?: Segment[] }) {
  if (!words) return <span className="diff-text">{line.text}</span>;
  return (
    <span className="diff-text">
      {words.map((s, i) =>
        s.changed ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: segments are positional.
          <span key={i} className="word">
            {s.text}
          </span>
        ) : (
          s.text
        ),
      )}
    </span>
  );
}

/** What a row needs to select lines and start comments; inert without a reviewer. */
type LineProps = {
  sel: Selection | null;
  onPick: (side: DiffSide, line: number, extend: boolean) => void;
  onComment: ((l: DiffLine, side: DiffSide) => void) | null;
  note?: string;
};

/** A line number: a button that selects the line when selection is on. */
function Gutter({
  line: l,
  side,
  onPick,
  onComment,
}: {
  line: DiffLine;
  side: DiffSide;
  onPick: LineProps["onPick"];
  onComment: LineProps["onComment"];
}) {
  const n = sideNo(l, side);
  if (n === null || !onComment)
    return <span className="diff-gutter">{n ?? ""}</span>;
  return (
    <button
      type="button"
      className="diff-gutter diff-gutter-btn"
      title="Select the line; Shift-click for a range"
      onClick={(e) => onPick(side, n, e.shiftKey)}
    >
      {n}
    </button>
  );
}

/** The "+" that starts a comment on a line, shown on hover. */
function CommentButton({
  line: l,
  side,
  onComment,
  note,
}: {
  line: DiffLine;
  side: DiffSide;
  onComment: LineProps["onComment"];
  note?: string;
}) {
  const n = sideNo(l, side);
  if (n === null) return null;
  const where = side === "old" ? `old line ${n}` : `line ${n}`;
  return (
    <button
      type="button"
      className="diff-comment-btn"
      aria-label={`Comment on ${where}`}
      title={note ?? `Comment on ${where} (C on the selected line)`}
      disabled={!onComment}
      onClick={() => onComment?.(l, side)}
    >
      +
    </button>
  );
}

function LineRow({
  line: l,
  words,
  sel,
  onPick,
  onComment,
  note,
}: { line: DiffLine; words?: Segment[] } & LineProps) {
  // A context line is on both sides; the new side is where comments go.
  const side: DiffSide = l.kind === "delete" ? "old" : "new";
  const active = !!onComment || !!note;
  return (
    <div
      className={`diff-row ${lineClass(l)}`}
      data-selected={inSelection(l, sel) || undefined}
    >
      <Gutter line={l} side="old" onPick={onPick} onComment={onComment} />
      <Gutter line={l} side="new" onPick={onPick} onComment={onComment} />
      <span className="diff-sign">
        {active && (
          <CommentButton
            line={l}
            side={side}
            onComment={onComment}
            note={note}
          />
        )}
        {l.kind === "add" ? "+" : l.kind === "delete" ? "−" : ""}
      </span>
      <Text line={l} words={words} />
    </div>
  );
}

function PairRow({
  left,
  right,
  words,
  widths,
  sel,
  onPick,
  onComment,
  note,
}: {
  left: DiffLine | null;
  right: DiffLine | null;
  words: (line: DiffLine) => Segment[] | undefined;
  /** Fixed column widths without wrapping, so the columns line up across rows. */
  widths: [string, string] | null;
} & LineProps) {
  const active = !!onComment || !!note;
  const half = (l: DiffLine | null, side: DiffSide) => {
    const style = widths
      ? { flex: "none", width: widths[side === "old" ? 0 : 1] }
      : undefined;
    return l ? (
      <span
        className={`half ${l.kind === "context" ? "" : lineClass(l)}`}
        style={style}
        data-selected={(inSelection(l, sel) && sel?.side === side) || undefined}
      >
        <Gutter line={l} side={side} onPick={onPick} onComment={onComment} />
        <span className="diff-sign">
          {active && (
            <CommentButton
              line={l}
              side={side}
              onComment={onComment}
              note={note}
            />
          )}
          {l.kind === "add" ? "+" : l.kind === "delete" ? "−" : ""}
        </span>
        <Text line={l} words={words(l)} />
      </span>
    ) : (
      <span className="half empty" style={style} />
    );
  };
  return (
    <div className="diff-row">
      {half(left, "old")}
      {half(right, "new")}
    </div>
  );
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

/** A View menu item: a check while it is on. */
function ViewItem({
  checked,
  disabled,
  title,
  onClick,
  children,
}: {
  checked: boolean;
  disabled?: boolean;
  title?: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitemcheckbox"
      aria-checked={checked}
      className="menu-item"
      disabled={disabled}
      title={title}
      onClick={onClick}
    >
      <span className="w-3.5 shrink-0 text-accent">{checked ? "✓" : ""}</span>
      {children}
    </button>
  );
}
