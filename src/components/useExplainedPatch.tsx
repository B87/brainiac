import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { noteSequence, notesIn, readingOrder } from "../lib/explain";
import type { DiffLine, ExplainSubject } from "../lib/ipc";
import { useKeys } from "../lib/keys";
import { usePref } from "../lib/prefs";
import { useExplanation } from "../lib/useExplanation";
import {
  requestNotice,
  requestRun,
  TOGGLE_EXPLANATION_EVENT,
} from "../lib/windowEvents";
import type { Annotate } from "./DiffView";
import ExplainDialog from "./ExplainDialog";
import ExplanationPanel, { NoteCard } from "./ExplanationPanel";
import { BulbIcon } from "./icons";

export type FileOrder = "reading" | "path";

/**
 * A patch view with its explanation (SPEC.md, section 14): **Explain** and
 * `E`, and **Explanation** (`Shift+E`) for the panel beside the patch, both
 * on the subject's header; notes after their lines (**Notes**, in the
 * patch's toolbar), `Shift+N` / `Shift+P` between notes, and the file list
 * in reading order. The view gives its subject, its files, and its selection.
 */
export function useExplainedPatch<T extends { path: string }>({
  repositoryId,
  view,
  subject,
  files,
  selectedPath,
  onSelectFile,
  onNotice = requestNotice,
  version = null,
  review = false,
}: {
  repositoryId: string;
  /** Which kind of view this is; the panel is shown or hidden per kind, so
   * opening it in a pull request does not open it in History. */
  view: ExplainSubject["kind"];
  subject: ExplainSubject | null;
  files: T[];
  selectedPath: string | null;
  onSelectFile: (path: string) => void;
  onNotice?: (message: string) => void;
  /** Changes when the subject moves (a pull request's head). */
  version?: string | null;
  /** A pull request's Files Changed: notes sit among review threads and
   * drafts, say they stay on this Mac, and are shown or hidden apart. */
  review?: boolean;
}): {
  /** Files in the order the list shows them, for the list and `[` `]`. */
  ordered: T[];
  order: FileOrder;
  setOrder: (o: FileOrder) => void;
  /** Each toured file's step, from 1, while the list is in reading order;
   * the numbered list is the tour. Files the tour does not list have none. */
  steps: Map<string, number>;
  hasExplanation: boolean;
  /** Explain and the panel toggle, for the subject's header: they act on
   * the whole subject, not on the file shown. */
  header: ReactNode;
  /** The notes toggle, for the patch's toolbar. */
  toolbar: ReactNode;
  annotate: Annotate | undefined;
  panel: ReactNode;
  /** Whether the panel is on screen, for a view that makes room for it. */
  panelShown: boolean;
  dialog: ReactNode;
  /** Open the Explain dialog, as `E` does. */
  openDialog: () => void;
} {
  const state = useExplanation(repositoryId, subject, version);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [panelOpen, setPanelOpen] = usePref(
    `brainiac.explain.panel.${view}`,
    true,
  );
  const [notesOn, setNotesOn] = usePref(
    review ? "brainiac.explain.pr.notes" : "brainiac.explain.notes",
    true,
  );
  const [order, setOrder] = usePref<FileOrder>(
    "brainiac.explain.order",
    "reading",
  );
  const record = state.current;
  const ready = record?.state === "ready" ? record.explanation : null;

  const ordered = useMemo(
    () => (ready && order === "reading" ? readingOrder(files, ready) : files),
    [files, ready, order],
  );

  const steps = useMemo(
    () =>
      new Map(
        ready && order === "reading"
          ? readingOrder(ready.tour, ready).map((s, i) => [s.path, i + 1])
          : [],
      ),
    [ready, order],
  );

  const sequence = useMemo(
    () => (ready ? noteSequence(ready, state.placement) : []),
    [ready, state.placement],
  );
  const goNote = useCallback(
    (delta: number) => {
      if (!sequence.length) return;
      const here = sequence.findIndex((n) => n.note.path === selectedPath);
      const next =
        sequence[
          Math.max(
            0,
            Math.min(sequence.length - 1, here < 0 ? 0 : here + delta),
          )
        ];
      if (!next) return;
      if (next.note.path !== selectedPath) onSelectFile(next.note.path);
      // The patch draws the note once its file is shown.
      setTimeout(
        () =>
          document
            .getElementById(`explain-note-${next.index}`)
            ?.scrollIntoView({ block: "center" }),
        150,
      );
    },
    [sequence, selectedPath, onSelectFile],
  );

  useKeys(
    {
      e: () => setDialogOpen(true),
      "shift+e": () => setPanelOpen(!panelOpen),
    },
    !!subject,
  );
  // View → Show or Hide Explanation (⇧⌘B), a menu item, so it arrives as a
  // window event rather than a key.
  useEffect(() => {
    if (!subject) return;
    const toggle = () => setPanelOpen(!panelOpen);
    window.addEventListener(TOGGLE_EXPLANATION_EVENT, toggle);
    return () => window.removeEventListener(TOGGLE_EXPLANATION_EVENT, toggle);
  }, [subject, panelOpen, setPanelOpen]);
  // Only with notes to move between: otherwise a shifted N or P still moves
  // by hunk, as it did before explanations.
  useKeys(
    {
      "shift+n": () => goNote(1),
      "shift+p": () => goNote(-1),
    },
    sequence.length > 0,
  );

  const annotate: Annotate | undefined = useMemo(() => {
    if (!ready || !notesOn || !selectedPath) return undefined;
    const notes = notesIn(ready, state.placement, selectedPath);
    if (!notes.length) return undefined;
    return {
      render: (line: DiffLine) => {
        if (line.kind === "delete" || line.new_no === null) return null;
        const here = notes.filter((n) => n.end === line.new_no);
        if (!here.length) return null;
        return (
          <>
            {here.map((n) => (
              <NoteCard
                key={n.index}
                placed={n}
                onSource={onSelectFile}
                privateMark={review}
              />
            ))}
          </>
        );
      },
    };
  }, [ready, notesOn, selectedPath, state.placement, onSelectFile, review]);

  const header = subject ? (
    <span className="flex shrink-0 items-center gap-1.5">
      <button
        type="button"
        className="btn btn-sm"
        title="Explain this change (E)"
        onClick={() => setDialogOpen(true)}
      >
        <BulbIcon size={13} />
        {record?.state === "working" ? "Explaining…" : "Explain"}
      </button>
      {record && (
        <button
          type="button"
          className="btn btn-sm"
          aria-pressed={panelOpen}
          title="Show or hide the explanation (⇧⌘B or Shift+E)"
          onClick={() => setPanelOpen(!panelOpen)}
        >
          Explanation
        </button>
      )}
    </span>
  ) : null;

  const toolbar =
    ready && ready.notes.length > 0 ? (
      <button
        type="button"
        className="btn btn-sm"
        aria-pressed={notesOn}
        title="Show or hide the notes in the patch (Shift+N and Shift+P move between them)"
        onClick={() => setNotesOn(!notesOn)}
      >
        Notes
      </button>
    ) : null;

  const panel =
    subject && panelOpen && (record || dialogOpen) ? (
      <ExplanationPanel
        repositoryId={repositoryId}
        subject={subject}
        state={state}
        selectedPath={selectedPath}
        onSelectFile={onSelectFile}
        onExplain={() => setDialogOpen(true)}
        onOpenRun={requestRun}
        onClose={() => setPanelOpen(false)}
        onNotice={onNotice}
      />
    ) : null;

  const dialog =
    dialogOpen && subject ? (
      <ExplainDialog
        repositoryId={repositoryId}
        subject={subject}
        onClose={() => setDialogOpen(false)}
        onStarted={(r) => {
          setDialogOpen(false);
          setPanelOpen(true);
          state.choose(r.id);
          state.reload();
        }}
        onOpen={(id) => {
          setDialogOpen(false);
          setPanelOpen(true);
          state.choose(id);
        }}
      />
    ) : null;

  return {
    ordered,
    order,
    setOrder,
    steps,
    hasExplanation: !!ready,
    header,
    toolbar,
    annotate,
    panel,
    panelShown: panel !== null,
    dialog,
    openDialog: () => setDialogOpen(true),
  };
}

/**
 * Two annotations on one diff: a pull request's threads and drafts, and the
 * explanation's notes before them. The first one's comment box stays.
 */
export function composeAnnotate(
  first: Annotate | undefined,
  notes: Annotate | undefined,
): Annotate | undefined {
  if (!first || !notes) return first ?? notes;
  return {
    ...first,
    render: (line: DiffLine) => {
      const a = notes.render(line);
      const b = first.render(line);
      if (!a && !b) return null;
      return (
        <>
          {a}
          {b}
        </>
      );
    },
  };
}

/** The file list's switch between reading order and path. */
export function OrderSwitch({
  order,
  setOrder,
}: {
  order: FileOrder;
  setOrder: (o: FileOrder) => void;
}) {
  return (
    <div role="tablist" aria-label="File order" className="seg seg-sm">
      <button
        type="button"
        role="tab"
        aria-selected={order === "reading"}
        title="The explanation's tour; [ and ] follow it"
        onClick={() => setOrder("reading")}
      >
        Reading order
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={order === "path"}
        onClick={() => setOrder("path")}
      >
        Path
      </button>
    </div>
  );
}
