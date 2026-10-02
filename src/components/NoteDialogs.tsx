/** Dialogs of the note editor: rename or move, version history, and compare. */
import { useEffect, useMemo, useState } from "react";
import { absoluteTime } from "../lib/format";
import {
  errorMessage,
  ipc,
  type NoteRevision,
  type NoteSummary,
  type RenamePreview,
} from "../lib/ipc";
import { plural } from "../lib/repo";
import { diffLines } from "../lib/textDiff";
import Dialog from "./Dialog";

/**
 * Rename or move a note inside the vault. Updating the links in other notes
 * is offered with the files it would change, and is off by default because
 * it edits them (SPEC.md, Links between notes).
 */
export function RenameDialog({
  note,
  onClose,
  onDone,
}: {
  note: NoteSummary;
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const [path, setPath] = useState(note.relative_path);
  const [preview, setPreview] = useState<RenamePreview | null>(null);
  const [update, setUpdate] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const target = path.trim().endsWith(".md")
    ? path.trim()
    : `${path.trim()}.md`;

  useEffect(() => {
    let alive = true;
    void ipc
      .previewRename(note.id, target)
      .then((p) => alive && setPreview(p))
      .catch(() => alive && setPreview(null));
    return () => {
      alive = false;
    };
  }, [note.id, target]);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const result = await ipc.renameNote({
        note_id: note.id,
        new_path: target,
        update_links: update,
      });
      const parts = [`Moved to ${result.note.relative_path}`];
      if (result.updated.length)
        parts.push(`links updated in ${plural(result.updated.length, "note")}`);
      if (result.failed.length)
        parts.push(`not updated in ${result.failed.join(", ")}`);
      onDone(parts.join("; "));
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  const linking = preview?.linking_notes ?? [];
  return (
    <Dialog
      title="Rename or Move Note"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || target === note.relative_path || !path.trim()}
            onClick={() => void submit()}
          >
            Rename
          </button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1">
          <span className="section-label">Path in the vault</span>
          <input
            className="text-input mono h-8"
            value={path}
            autoFocus
            onChange={(e) => setPath(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void submit()}
          />
          <span className="text-[12px] text-muted">
            Folders are created as needed. The note's title does not change.
          </span>
        </label>
        {linking.length > 0 && (
          <div className="flex flex-col gap-1.5">
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={update}
                onChange={(e) => setUpdate(e.target.checked)}
              />
              Update links in {plural(linking.length, "other note")}
            </label>
            <ul className="m-0 max-h-32 list-none overflow-y-auto rounded-md border p-2 text-[12px]">
              {linking.map((n) => (
                <li key={n.id} className="mono truncate text-fg-2">
                  {n.relative_path}
                </li>
              ))}
            </ul>
          </div>
        )}
        {error && (
          <div role="alert" className="text-conflict">
            {error}
          </div>
        )}
      </div>
    </Dialog>
  );
}

const REASON: Record<NoteRevision["reason"], string> = {
  app_save: "Saved in Brainiac",
  external_change: "Changed outside Brainiac",
  restore: "Before restoring a version",
};

/** Earlier versions of a note, kept outside the vault, and restoring one. */
export function HistoryDialog({
  note,
  version,
  onClose,
  onRestored,
}: {
  note: NoteSummary;
  /** The version open now, which a restore must name. */
  version: string;
  onClose: () => void;
  onRestored: () => void;
}) {
  const [revisions, setRevisions] = useState<NoteRevision[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void ipc
      .listRevisions(note.id)
      .then((r) => {
        setRevisions(r);
        setSelected(r[0]?.id ?? null);
      })
      .catch((e) => setError(errorMessage(e)));
  }, [note.id]);

  useEffect(() => {
    setText(null);
    if (!selected) return;
    let alive = true;
    void ipc
      .readRevision(selected)
      .then((t) => alive && setText(t))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [selected]);

  const restore = async () => {
    if (!selected) return;
    try {
      await ipc.restoreRevision(note.id, selected, version);
      onRestored();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <Dialog
      title={`Versions of “${note.title}”`}
      onClose={onClose}
      width={820}
      footer={
        <>
          <span className="mr-auto text-[12px] text-muted">
            Up to 20 versions per note are kept for 30 days, outside the vault.
          </span>
          <button type="button" className="btn" onClick={onClose}>
            Close
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!selected || note.missing}
            onClick={() => void restore()}
          >
            Restore This Version
          </button>
        </>
      }
    >
      {error && (
        <div role="alert" className="mb-2 text-conflict">
          {error}
        </div>
      )}
      {revisions && revisions.length === 0 ? (
        <p className="m-0 text-muted">
          No earlier versions yet. Brainiac keeps one when you edit the note or
          when it changes outside Brainiac.
        </p>
      ) : (
        <div className="flex h-[420px] gap-3">
          <div className="w-[230px] shrink-0 overflow-y-auto rounded-md border">
            {revisions?.map((r) => (
              <button
                key={r.id}
                type="button"
                className="list-row flex-col px-2.5 py-1.5"
                aria-current={r.id === selected}
                onClick={() => setSelected(r.id)}
              >
                <span>{absoluteTime(r.created_at)}</span>
                <span className="meta text-[11.5px] text-muted">
                  {REASON[r.reason]}
                </span>
              </button>
            ))}
          </div>
          <pre className="selectable mono m-0 min-w-0 flex-1 overflow-auto rounded-md border bg-app p-3 text-[12px] whitespace-pre-wrap">
            {text ?? ""}
          </pre>
        </div>
      )}
    </Dialog>
  );
}

/** The file on disk next to the unsaved draft, after a save conflict. */
export function CompareDialog({
  disk,
  mine,
  onClose,
  onKeepMine,
  onUseDisk,
}: {
  disk: string;
  mine: string;
  onClose: () => void;
  onKeepMine: () => void;
  onUseDisk: () => void;
}) {
  const rows = useMemo(() => diffLines(disk, mine), [disk, mine]);
  return (
    <Dialog
      title="Compare with the Version on Disk"
      onClose={onClose}
      width={820}
      footer={
        <>
          <span className="mr-auto text-[12px] text-muted">
            <span className="text-del">− on disk</span> ·{" "}
            <span className="text-add">+ your draft</span>
          </span>
          <button type="button" className="btn" onClick={onUseDisk}>
            Use the Version on Disk
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={onKeepMine}
          >
            Keep My Draft
          </button>
        </>
      }
    >
      <div className="diff diff-wrap selectable max-h-[440px] overflow-auto rounded-md border">
        {rows.map((r, i) => (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: rows have no identity besides their position.
            key={i}
            className={`diff-row ${r.kind === "added" ? "diff-add" : r.kind === "removed" ? "diff-del" : ""}`}
          >
            <span className="diff-sign">
              {r.kind === "added" ? "+" : r.kind === "removed" ? "−" : ""}
            </span>
            <span className="diff-text">{r.text}</span>
          </div>
        ))}
      </div>
    </Dialog>
  );
}
