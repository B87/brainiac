import { ask } from "@tauri-apps/plugin-dialog";
import { type ReactNode, useEffect, useRef, useState } from "react";
import {
  checksLine,
  depthLabel,
  formatDuration,
  knownLeftOutLine,
  type PlacedNote,
  readingOrder,
  sameSubject,
  stepLabel,
  subjectLabel,
  usageText,
} from "../lib/explain";
import { relativeTime } from "../lib/format";
import {
  type CitedQuote,
  type ConceptKind,
  type ExplainStep,
  type ExplainSubject,
  type ExplanationRecord,
  errorMessage,
  ipc,
} from "../lib/ipc";
import { usePref } from "../lib/prefs";
import { plural } from "../lib/repo";
import { requestSettings } from "../lib/settings";
import type { ExplanationState } from "../lib/useExplanation";
import { CloseIcon, MoreIcon } from "./icons";
import Popover from "./Popover";

type Tab = "concepts" | "questions";

/** The panel's width: dragged by its left edge, remembered, within bounds. */
const PANEL_WIDTH = 360;
const PANEL_MIN = 280;
function panelMax(): number {
  return Math.max(
    PANEL_MIN,
    Math.min(960, Math.round(window.innerWidth * 0.6)),
  );
}
function clampWidth(w: number): number {
  return Math.round(Math.min(panelMax(), Math.max(PANEL_MIN, w)));
}

/**
 * The panel's left edge: drag to resize, arrow keys when focused, double
 * click to go back to the default width.
 */
function ResizeEdge({
  width,
  onLive,
  onDone,
}: {
  width: number;
  /** The width while dragging, not yet remembered. */
  onLive: (w: number | null) => void;
  onDone: (w: number) => void;
}) {
  const drag = useRef<{ x: number; w: number } | null>(null);
  return (
    // biome-ignore lint/a11y/useSemanticElements: an <hr> cannot be dragged or focused; this is a focusable splitter.
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the explanation"
      aria-valuenow={width}
      aria-valuemin={PANEL_MIN}
      aria-valuemax={panelMax()}
      tabIndex={0}
      title="Drag to resize; double-click for the default width"
      className="absolute top-0 bottom-0 -left-[3px] z-10 w-[6px] cursor-col-resize hover:bg-accent/40 focus-visible:bg-accent/60 focus-visible:outline-none"
      onPointerDown={(e) => {
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { x: e.clientX, w: width };
      }}
      onPointerMove={(e) => {
        if (!drag.current) return;
        // The panel is on the right, so dragging left makes it wider.
        onLive(clampWidth(drag.current.w + drag.current.x - e.clientX));
      }}
      onPointerUp={(e) => {
        if (!drag.current) return;
        const w = clampWidth(drag.current.w + drag.current.x - e.clientX);
        drag.current = null;
        onLive(null);
        onDone(w);
      }}
      onDoubleClick={() => onDone(PANEL_WIDTH)}
      onKeyDown={(e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        e.preventDefault();
        e.stopPropagation();
        onDone(clampWidth(width + (e.key === "ArrowLeft" ? 24 : -24)));
      }}
    />
  );
}

const STEPS: ExplainStep[] = ["copying", "starting", "reading", "checking"];

const KIND_WORD: Record<ConceptKind, string> = {
  language: "Language",
  library: "Library",
  system: "System tool",
  project_pattern: "Project pattern",
};

/** Lines of a source, as `path:12–18`. */
export function sourceLabel(q: CitedQuote): string {
  return `${q.path}:${q.start === q.end ? q.start : `${q.start}–${q.end}`}`;
}

/**
 * A note after the lines it explains (SPEC.md, The explanation, Notes in
 * the patch), with its sources and, on a moved branch, whether it is out
 * of date.
 */
export function NoteCard({
  placed,
  position,
  onSource,
  privateMark = false,
}: {
  placed: PlacedNote;
  /** Its place among all the notes, in reading order: "Note 4 of 14". */
  position?: { n: number; total: number };
  onSource: (path: string) => void;
  /** Among a pull request's threads and drafts: say it is never posted. */
  privateMark?: boolean;
}) {
  const { note, outOfDate, start, end } = placed;
  const name = privateMark ? "Explanation note" : "Note";
  return (
    <div
      id={`explain-note-${placed.index}`}
      className="mx-3 my-1.5 rounded-md border border-info-line bg-info-bg px-3 py-2 font-sans text-[12.5px] leading-relaxed whitespace-normal"
    >
      <div className="mb-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px]">
        <span className="font-medium text-link">
          {position ? `${name} ${position.n} of ${position.total}` : name}
        </span>
        <span className="text-muted">
          {start === end ? `on line ${start}` : `on lines ${start}–${end}`}
        </span>
        {outOfDate && (
          <span
            className="state-pill"
            data-tone="amber"
            title="These lines changed since it was explained."
          >
            Out of date
          </span>
        )}
        {privateMark && (
          <span
            className="ml-auto text-muted"
            title="Explanation notes are kept in Brainiac on this Mac. Nothing sends them to the provider."
          >
            Only on this Mac
          </span>
        )}
      </div>
      <div className="selectable whitespace-pre-wrap text-fg">{note.text}</div>
      <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11.5px] text-muted">
        {note.sources.map((s) => (
          <button
            key={`${s.path}:${s.start}:${s.quote}`}
            type="button"
            className="mono text-link hover:underline"
            title={s.quote}
            onClick={() => onSource(s.path)}
          >
            {sourceLabel(s)}
            {s.moved && (
              <span
                className="ml-1 font-sans text-muted"
                title="The quote was found at other lines than the agent gave."
              >
                moved
              </span>
            )}
          </button>
        ))}
        {note.sources_dropped > 0 && (
          <span title="Its claim may have rested on a source Brainiac could not find.">
            {note.sources_dropped === 1
              ? "1 source not found"
              : `${note.sources_dropped} sources not found`}
          </span>
        )}
      </div>
    </div>
  );
}

/**
 * The panel beside the patch (SPEC.md, section 14, The explanation): while
 * an explanation works, its steps and the files the agent opens; when it
 * failed, why; when it is ready, the summary, what the checks did, the
 * selected file's place in the tour (the file list is the tour, numbered),
 * and the Concepts and Questions tabs, with the disagreements after.
 */
export default function ExplanationPanel({
  repositoryId,
  subject,
  state,
  selectedPath,
  onSelectFile,
  onExplain,
  onOpenRun,
  onClose,
  onNotice,
}: {
  repositoryId: string;
  /** The subject on screen; the explanation shown may be of another with
   * the same changes (a branch and its pull request). */
  subject: ExplainSubject;
  state: ExplanationState;
  selectedPath: string | null;
  onSelectFile: (path: string) => void;
  /** Open the Explain dialog. */
  onExplain: () => void;
  /** How it was written: the explain run's conversation, read only. */
  onOpenRun: (runId: string) => void;
  onClose: () => void;
  onNotice: (message: string) => void;
}) {
  const { current: record, placement, records, choose } = state;
  const [tab, setTab] = useState<Tab>("concepts");
  const [savedWidth, setSavedWidth] = usePref(
    "brainiac.explain.width",
    PANEL_WIDTH,
  );
  const [liveWidth, setLiveWidth] = useState<number | null>(null);
  // A narrower window than when the width was saved still leaves the patch room.
  const width = liveWidth ?? clampWidth(savedWidth);
  const [error, setError] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);

  // J and K move through the panel's items while it has focus; Space does
  // the focused item's action, as on any button.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key !== "j" && e.key !== "k") return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const target = e.target as HTMLElement;
    if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") return;
    const items = Array.from(
      box.current?.querySelectorAll<HTMLElement>("[data-explain-item]") ?? [],
    );
    if (!items.length) return;
    e.preventDefault();
    e.stopPropagation();
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next =
      e.key === "j"
        ? Math.min(items.length - 1, at + 1)
        : Math.max(0, at < 0 ? 0 : at - 1);
    items[next]?.focus();
  };

  const act = async (what: () => Promise<unknown>) => {
    setError(null);
    try {
      await what();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  // Try again and Re-explain explain the subject on screen, with the same
  // agent and depth.
  const again = (r: ExplanationRecord) =>
    act(() =>
      ipc.startExplanation({
        repository_id: r.repository_id,
        subject,
        profile_id: r.profile_id,
        host_id: r.host_id,
        depth: r.depth,
        questions: r.questions,
      }),
    );

  return (
    <aside
      ref={box}
      aria-label="Explanation"
      className="relative flex shrink-0 flex-col border-l bg-panel-2"
      style={{ width }}
      onKeyDown={onKeyDown}
    >
      <ResizeEdge width={width} onLive={setLiveWidth} onDone={setSavedWidth} />
      <div className="flex h-9 shrink-0 items-center gap-2 border-b pr-1.5 pl-3 text-[12px]">
        <span className="font-semibold">Explanation</span>
        {records.length > 1 && (
          <select
            className="field h-6 py-0 text-[12px]"
            aria-label="Explanation shown"
            value={record?.id ?? ""}
            onChange={(e) => choose(e.target.value)}
          >
            {records.map((r) => (
              <option key={r.id} value={r.id}>
                {depthLabel(r.depth)} · {r.model || r.agent}
                {sameSubject(r.subject, subject)
                  ? ""
                  : ` · from ${subjectLabel(r.subject)}`}
              </option>
            ))}
          </select>
        )}
        <span className="flex-1" />
        {record?.state === "ready" && record.explanation && (
          <ReadyActions
            record={record}
            onOpenRun={onOpenRun}
            onExplain={onExplain}
            act={act}
            onNotice={onNotice}
          />
        )}
        <button
          type="button"
          className="btn btn-sm btn-ghost w-6 px-0"
          aria-label="Hide the explanation (⇧⌘B or Shift+E)"
          title="Hide the explanation (⇧⌘B or Shift+E)"
          onClick={onClose}
        >
          <CloseIcon size={12} />
        </button>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-3.5 py-3 text-[12.5px] leading-relaxed">
        {(error || state.error) && (
          <div role="alert" className="text-conflict">
            {error ?? state.error}
          </div>
        )}
        {!record && (
          <div className="flex flex-col items-start gap-2 text-fg-2">
            <span className="font-semibold text-fg">Not explained yet</span>
            <p className="m-0">
              An agent can explain why this change exists, the files in reading
              order, notes beside the lines, and the ideas it relies on. Every
              note and quote is checked against the change before you see it.
            </p>
            <button type="button" className="btn btn-sm" onClick={onExplain}>
              Explain… <span className="kbd">E</span>
            </button>
            <p className="m-0 text-[12px] text-muted">
              Nothing is sent until you press Explain; the first time in a
              repository, it asks whether its code may go to the provider.
            </p>
          </div>
        )}
        {record?.state === "working" && (
          <Working
            record={record}
            onCancel={() => act(() => ipc.cancelExplanation(record.id))}
          />
        )}
        {record &&
          (record.state === "failed" || record.state === "cancelled") && (
            <Ended
              record={record}
              onTryAgain={() => again(record)}
              onOpenRun={onOpenRun}
              onDelete={() => act(() => ipc.deleteExplanation(record.id))}
            />
          )}
        {record?.state === "ready" && record.explanation && (
          <Ready
            repositoryId={repositoryId}
            subject={subject}
            record={record}
            placementMoved={placement?.moved ?? false}
            placementGone={placement?.gone ?? false}
            placementTip={placement?.tip ?? record.tip}
            outOfDate={
              placement?.notes.filter((n) => n.out_of_date).length ?? 0
            }
            uncovered={placement?.uncovered ?? []}
            tab={tab}
            setTab={setTab}
            selectedPath={selectedPath}
            onSelectFile={onSelectFile}
            onReexplain={() => again(record)}
            act={act}
            onNotice={onNotice}
          />
        )}
      </div>
    </aside>
  );
}

/** A branch or pull request with nothing left to compare. */
function goneText(kind: ExplainSubject["kind"]): string {
  return kind === "pull_request"
    ? "The pull request has no changes against its target now; the explanation is kept as written."
    : "The branch is gone or has no changes against the default branch; the explanation is kept as written.";
}

/** "3 notes out of date and 2 files not covered; the other 11 notes still match their lines." */
function movedCounts(
  total: number,
  outOfDate: number,
  uncovered: number,
): string {
  const parts = [`${plural(outOfDate, "note")} out of date`];
  if (uncovered) parts.push(`${plural(uncovered, "file")} not covered`);
  const rest = total - outOfDate;
  const still = rest
    ? `; the other ${plural(rest, "note")} still ${rest === 1 ? "matches its" : "match their"} lines`
    : "";
  return `${parts.join(" and ")}${still}.`;
}

function elapsed(since: string): string {
  const secs = Math.max(
    0,
    Math.round((Date.now() - new Date(since).getTime()) / 1000),
  );
  return formatDuration(secs);
}

function Working({
  record,
  onCancel,
}: {
  record: ExplanationRecord;
  onCancel: () => void;
}) {
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, []);
  const steps: ExplainStep[] =
    record.step === "follow_up" ? [...STEPS, "follow_up"] : STEPS;
  const at = steps.indexOf(record.step ?? "copying");
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <span className="font-medium">Explaining…</span>
        <span className="text-muted">
          {elapsed(record.created_at)} of {record.time_limit_minutes} min
        </span>
        <span className="flex-1" />
        <button type="button" className="btn btn-sm" onClick={onCancel}>
          Cancel
        </button>
      </div>
      <ol className="m-0 flex flex-col gap-1 pl-0">
        {steps.map((s, i) => (
          <li
            key={s}
            className={`flex items-center gap-2 ${i === at ? "font-medium text-fg" : i < at ? "text-fg-2" : "text-muted"}`}
          >
            <span aria-hidden="true">
              {i < at ? "✓" : i === at ? "•" : "○"}
            </span>
            {stepLabel(s)}
          </li>
        ))}
      </ol>
      {record.files_read.length > 0 && (
        <div className="flex flex-col gap-0.5">
          <span className="text-[11.5px] font-medium text-fg-2">
            Files the agent opened
          </span>
          {record.files_read.slice(-12).map((f) => (
            <span key={f} className="mono truncate text-[11.5px] text-muted">
              {f}
            </span>
          ))}
        </div>
      )}
      <span className="text-[12px] text-muted">
        Leaving this view does not stop it; Brainiac says when it is done.
      </span>
    </div>
  );
}

function Ended({
  record,
  onTryAgain,
  onOpenRun,
  onDelete,
}: {
  record: ExplanationRecord;
  onTryAgain: () => void;
  onOpenRun: (runId: string) => void;
  onDelete: () => void;
}) {
  const usage = usageText(record);
  return (
    <div className="flex flex-col gap-2">
      <span className="font-medium">
        {record.state === "cancelled" ? "Cancelled" : "The explanation failed"}
      </span>
      {record.error && <span className="text-fg-2">{record.error}</span>}
      {record.errors.length > 0 && (
        <details>
          <summary className="cursor-default text-link">
            What the checks found
          </summary>
          <ul className="m-0 pl-5 text-[12px] text-fg-3">
            {record.errors.map((e) => (
              <li key={e}>{e}</li>
            ))}
          </ul>
        </details>
      )}
      {usage && <span className="text-muted">It took {usage}.</span>}
      <span className="text-[12px] text-muted">Nothing partial is shown.</span>
      <div className="flex flex-wrap gap-2">
        <button type="button" className="btn btn-sm" onClick={onTryAgain}>
          Try again
        </button>
        {record.run_id && (
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => onOpenRun(record.run_id as string)}
          >
            How it was written
          </button>
        )}
        <button
          type="button"
          className="btn btn-sm btn-ghost"
          onClick={onDelete}
        >
          Delete
        </button>
      </div>
      <span className="text-[12px] text-muted">
        Try again runs the agent again, at its cost.
      </span>
    </div>
  );
}

function Ready({
  repositoryId,
  subject,
  record,
  placementMoved,
  placementGone,
  placementTip,
  outOfDate,
  uncovered,
  tab,
  setTab,
  selectedPath,
  onSelectFile,
  onReexplain,
  act,
  onNotice,
}: {
  repositoryId: string;
  subject: ExplainSubject;
  record: ExplanationRecord;
  placementMoved: boolean;
  placementGone: boolean;
  /** The branch's tip or the pull request's head now. */
  placementTip: string;
  outOfDate: number;
  uncovered: string[];
  tab: Tab;
  setTab: (t: Tab) => void;
  selectedPath: string | null;
  onSelectFile: (path: string) => void;
  onReexplain: () => void;
  act: (what: () => Promise<unknown>) => Promise<void>;
  onNotice: (message: string) => void;
}) {
  const e = record.explanation;
  const [checksOpen, setChecksOpen] = useState(false);
  const [metaOpen, setMetaOpen] = useState(false);
  const [known, setKnown] = useState<Record<string, string>>({});
  const [leftOutOpen, setLeftOutOpen] = useState(false);
  // Ledger ids forgotten from the left-out list; later explanations explain them.
  const [forgotten, setForgotten] = useState<Set<string>>(new Set());
  const [revealed, setRevealed] = useState<Set<number>>(new Set());
  const [showHidden, setShowHidden] = useState(false);
  if (!e) return null;
  const checks = checksLine(e);
  const leftOut = knownLeftOutLine(e);
  const usage = usageText(record);
  const tour = readingOrder(e.tour, e);
  const step = selectedPath
    ? tour.findIndex((s) => s.path === selectedPath)
    : -1;
  const disagreements = e.disagreements
    .map((d, i) => ({ d, i }))
    .filter(({ i }) => showHidden || !record.hidden.includes(i));
  const hiddenCount = record.hidden.length;

  const learn = (name: string, kind: ConceptKind) =>
    act(async () => {
      const id = await ipc.learnConcept(repositoryId, kind, name, record.id);
      setKnown((k) => ({ ...k, [`${kind}:${name}`]: id }));
    });
  const forget = (id: string) =>
    act(async () => {
      await ipc.forgetConcept(id);
      setForgotten((f) => new Set(f).add(id));
    });
  const unlearn = (name: string, kind: ConceptKind) =>
    act(async () => {
      const id = known[`${kind}:${name}`];
      if (id) await ipc.forgetConcept(id);
      setKnown((k) => {
        const next = { ...k };
        delete next[`${kind}:${name}`];
        return next;
      });
    });

  return (
    <div className="flex flex-col gap-3">
      {!sameSubject(record.subject, subject) && (
        <span className="text-[12px] text-fg-2">
          The same changes as {subjectLabel(record.subject)}; explained there.
        </span>
      )}
      {placementMoved && (
        <div
          role="status"
          className="flex flex-col gap-1.5 rounded-md border border-amber-300 bg-amber-50 px-2.5 py-2 text-[12px] text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100"
        >
          {placementGone ? (
            goneText(record.subject.kind)
          ) : (
            <>
              <span className="font-semibold">
                The{" "}
                {record.subject.kind === "pull_request"
                  ? "pull request"
                  : "branch"}{" "}
                moved since this explanation
              </span>
              <span>
                Explained at{" "}
                <span className="mono">{record.tip.slice(0, 7)}</span>,{" "}
                {relativeTime(record.created_at)}; now at{" "}
                <span className="mono">{placementTip.slice(0, 7)}</span>.{" "}
                {movedCounts(e.notes.length, outOfDate, uncovered.length)}
              </span>
              <span className="flex items-center gap-2">
                <button
                  type="button"
                  className="btn btn-sm btn-primary"
                  onClick={onReexplain}
                >
                  Re-explain
                </button>
                <span className="text-[11.5px] opacity-80">
                  {depthLabel(record.depth)} · the whole{" "}
                  {record.subject.kind === "pull_request"
                    ? "pull request"
                    : "branch"}{" "}
                  at its tip, at its cost
                </span>
              </span>
            </>
          )}
        </div>
      )}
      <p className="selectable m-0 text-[13px] text-fg">{e.summary}</p>
      {checks && (
        <div className="text-[12px] text-muted">
          <button
            type="button"
            className="text-left hover:underline"
            aria-expanded={checksOpen}
            onClick={() => setChecksOpen(!checksOpen)}
          >
            Matched to the change: {checks}
          </button>
          {checksOpen && (
            <ul className="m-0 mt-1 pl-5">
              {e.checks.left_out.map((l) => (
                <li key={l}>{l}</li>
              ))}
            </ul>
          )}
        </div>
      )}
      {leftOut && (
        <div className="text-[12px] text-muted">
          <button
            type="button"
            className="text-left hover:underline"
            aria-expanded={leftOutOpen}
            title="Concepts you marked as known, which this explanation does not teach again"
            onClick={() => setLeftOutOpen(!leftOutOpen)}
          >
            {leftOut}
          </button>
          {leftOutOpen && (
            <ul className="m-0 mt-1 list-none pl-0">
              {e.known_left_out.map((c) => (
                <li key={c.id} className="flex items-baseline gap-2 py-0.5">
                  <span className="text-fg-2">{c.name}</span>
                  <span className="text-[11.5px]">{KIND_WORD[c.kind]}</span>
                  <span className="flex-1" />
                  {forgotten.has(c.id) ? (
                    <span>Forgotten: later explanations will explain it</span>
                  ) : (
                    <button
                      type="button"
                      className="text-link hover:underline"
                      onClick={() => void forget(c.id)}
                    >
                      Undo
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      <div className="text-[12px] text-muted">
        <button
          type="button"
          className="text-left hover:underline"
          aria-expanded={metaOpen}
          onClick={() => setMetaOpen(!metaOpen)}
        >
          {depthLabel(record.depth)} · {record.model || record.agent} ·{" "}
          {relativeTime(record.ended_at ?? record.created_at)}
          {usage ? ` · ${usage}` : ""}
        </button>
        {metaOpen && (
          <div className="mt-1 flex flex-col gap-0.5">
            <span>
              {record.agent === "claude_code" ? "Claude Code" : "OpenCode"} on{" "}
              {record.host_name}, model {record.model || "the agent's default"}
            </span>
            {e.sources_read.length > 0 && (
              <span>Read: {e.sources_read.join(", ")}</span>
            )}
          </div>
        )}
      </div>

      {step >= 0 && (
        <div className="flex flex-col gap-0.5">
          <span className="text-[11.5px] font-medium text-fg-2">
            This file · step {step + 1} of {tour.length}
          </span>
          {tour[step].role && (
            <span className="text-fg-2">{tour[step].role}</span>
          )}
        </div>
      )}
      {step < 0 && selectedPath && uncovered.includes(selectedPath) && (
        <span className="text-[12px] text-fg-2">
          This file is not in this explanation: it changed after it was
          explained.
        </span>
      )}

      <div
        role="tablist"
        aria-label="Explanation"
        className="flex gap-1 border-b"
      >
        <TabButton tab="concepts" current={tab} onClick={setTab}>
          Concepts {e.concepts.length > 0 && `(${e.concepts.length})`}
        </TabButton>
        {e.questions.length > 0 && (
          <TabButton tab="questions" current={tab} onClick={setTab}>
            Questions
          </TabButton>
        )}
      </div>

      {tab === "concepts" && (
        <div className="flex flex-col gap-3">
          {e.concepts.length === 0 && (
            <span className="text-muted">
              No new concepts: the change relies on what you know.
            </span>
          )}
          {e.concepts.map((c) => {
            const isKnown = `${c.kind}:${c.name}` in known;
            return (
              <div key={`${c.kind}:${c.name}`} className="flex flex-col gap-1">
                <div className="flex items-baseline gap-2">
                  <span className="font-medium">{c.name}</span>
                  <span className="text-[11.5px] text-muted">
                    {KIND_WORD[c.kind]}
                  </span>
                  <span className="flex-1" />
                  {isKnown ? (
                    <span className="flex items-center gap-1.5 text-[12px] text-muted">
                      Known
                      <button
                        type="button"
                        data-explain-item
                        className="text-link hover:underline"
                        onClick={() => void unlearn(c.name, c.kind)}
                      >
                        Undo
                      </button>
                    </span>
                  ) : (
                    <button
                      type="button"
                      data-explain-item
                      className="btn btn-sm"
                      title="Later explanations leave it out unless a change uses it in a new way"
                      onClick={() => void learn(c.name, c.kind)}
                    >
                      I know this
                    </button>
                  )}
                </div>
                <span className="selectable text-fg-2">{c.explanation}</span>
                {c.appears.length > 0 && (
                  <span className="flex flex-wrap gap-x-2 text-[11.5px]">
                    {c.appears.map((p) => (
                      <button
                        key={`${p.path}:${p.line}`}
                        type="button"
                        className="mono text-link hover:underline"
                        onClick={() => onSelectFile(p.path)}
                      >
                        {p.path}:{p.line}
                      </button>
                    ))}
                  </span>
                )}
              </div>
            );
          })}
        </div>
      )}

      {tab === "questions" && (
        <ol className="m-0 flex flex-col gap-2.5 pl-5">
          {e.questions.map((q, i) => (
            <li key={q.question}>
              <span className="text-fg">{q.question}</span>
              {revealed.has(i) ? (
                <p className="selectable m-0 mt-1 text-fg-2">{q.answer}</p>
              ) : (
                <button
                  type="button"
                  data-explain-item
                  className="btn btn-sm mt-1"
                  onClick={() => setRevealed(new Set(revealed).add(i))}
                >
                  Reveal answer
                </button>
              )}
            </li>
          ))}
        </ol>
      )}

      {(disagreements.length > 0 || hiddenCount > 0) && (
        <div className="flex flex-col gap-2 border-t pt-3">
          <span className="font-medium">Code and docs disagree</span>
          {disagreements.map(({ d, i }) => (
            <div
              key={d.claim}
              className="flex flex-col gap-1.5 rounded-md bg-panel px-2.5 py-2"
            >
              <span className="text-fg">{d.claim}</span>
              <Quote label="Code" q={d.code} onSelectFile={onSelectFile} />
              <Quote label="Docs" q={d.doc} onSelectFile={onSelectFile} />
              <div className="flex gap-2">
                <button
                  type="button"
                  data-explain-item
                  className="btn btn-sm"
                  onClick={() =>
                    void act(async () => {
                      await ipc.createTask({
                        title: `Code and docs disagree: ${d.claim}`.slice(
                          0,
                          200,
                        ),
                        description: `${sourceLabel(d.code)} and ${sourceLabel(d.doc)}`,
                        status: "todo",
                        planned_date: null,
                        due_date: null,
                        note_id: null,
                        repository_id: repositoryId,
                        sorted: false,
                      });
                      onNotice("Task added.");
                    })
                  }
                >
                  Add a task
                </button>
                <button
                  type="button"
                  data-explain-item
                  className="btn btn-sm btn-ghost"
                  onClick={() =>
                    void act(() =>
                      ipc.setDisagreementHidden(
                        record.id,
                        i,
                        !record.hidden.includes(i),
                      ),
                    )
                  }
                >
                  {record.hidden.includes(i) ? "Show" : "Not a problem"}
                </button>
              </div>
            </div>
          ))}
          {hiddenCount > 0 && !showHidden && (
            <button
              type="button"
              className="self-start text-[12px] text-link hover:underline"
              onClick={() => setShowHidden(true)}
            >
              {hiddenCount} hidden · Show
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * The panel header's actions on a ready explanation: Save as note, and a
 * menu with How it was written, Explain again, Settings, and Delete.
 */
function ReadyActions({
  record,
  onOpenRun,
  onExplain,
  act,
  onNotice,
}: {
  record: ExplanationRecord;
  onOpenRun: (runId: string) => void;
  onExplain: () => void;
  act: (what: () => Promise<unknown>) => Promise<void>;
  onNotice: (message: string) => void;
}) {
  const [menu, setMenu] = useState(false);
  const item = (label: string, onClick: () => void, danger = false) => (
    <button
      type="button"
      role="menuitem"
      className={`menu-item ${danger ? "text-conflict" : ""}`}
      onClick={() => {
        setMenu(false);
        onClick();
      }}
    >
      {label}
    </button>
  );
  return (
    <span className="relative flex items-center gap-1">
      <button
        type="button"
        className="btn btn-sm"
        onClick={() =>
          void act(async () => {
            const note = await ipc.saveExplanationAsNote(record.id);
            onNotice(`Saved as ${note.relative_path}.`);
          })
        }
      >
        Save as note
      </button>
      <button
        type="button"
        className="btn btn-sm btn-ghost w-6 px-0"
        aria-label="More: How it was written, Explain again, Delete"
        title="How it was written · Explain again… · Delete…"
        aria-haspopup="menu"
        aria-expanded={menu}
        onClick={() => setMenu(!menu)}
      >
        <MoreIcon size={14} />
      </button>
      {menu && (
        <Popover align="right" onClose={() => setMenu(false)}>
          {record.run_id &&
            item("How it was written", () =>
              onOpenRun(record.run_id as string),
            )}
          {item("Explain again or at another depth…", onExplain)}
          {item("Settings → Explanations", () =>
            requestSettings("explain-agent"),
          )}
          {item(
            "Delete explanation…",
            () =>
              void (async () => {
                const sure = await ask(
                  "Delete this explanation? Its run's conversation, How it was written, goes with it.",
                  {
                    title: "Delete Explanation",
                    kind: "warning",
                    okLabel: "Delete Explanation",
                  },
                );
                if (sure) await act(() => ipc.deleteExplanation(record.id));
              })(),
            true,
          )}
        </Popover>
      )}
    </span>
  );
}

function TabButton({
  tab,
  current,
  onClick,
  children,
}: {
  tab: Tab;
  current: Tab;
  onClick: (t: Tab) => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="tab"
      className="tab"
      aria-selected={tab === current}
      onClick={() => onClick(tab)}
    >
      {children}
    </button>
  );
}

function Quote({
  label,
  q,
  onSelectFile,
}: {
  label: string;
  q: CitedQuote;
  onSelectFile: (path: string) => void;
}) {
  return (
    <div className="flex flex-col gap-0.5">
      <button
        type="button"
        className="mono self-start text-[11.5px] text-link hover:underline"
        onClick={() => onSelectFile(q.path)}
      >
        {label}, {sourceLabel(q)}
        {q.moved && <span className="ml-1 font-sans text-muted">moved</span>}
      </button>
      <pre className="selectable m-0 overflow-x-auto rounded bg-panel-2 px-2 py-1 text-[11.5px] whitespace-pre-wrap">
        {q.quote}
      </pre>
    </div>
  );
}
