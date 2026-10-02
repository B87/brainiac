import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useRef, useState } from "react";
import type { NoteLink } from "../lib/editor/livePreview";
import {
  errorMessage,
  ipc,
  isAppError,
  type NoteContent,
  type NoteSummary,
  onNoteChanged,
  onNoteMissing,
  subscribe,
  type VaultInfo,
} from "../lib/ipc";
import {
  editedLabel,
  folderOf,
  insideVault,
  stem,
  vaultImageUrl,
} from "../lib/notes";
import { plural } from "../lib/repo";
import {
  AlertIcon,
  ExternalIcon,
  MoreIcon,
  NoteIcon,
  PanelRightIcon,
} from "./icons";
import { CompareDialog, HistoryDialog, RenameDialog } from "./NoteDialogs";
import { NoteEditor } from "./NoteEditor";
import Popover from "./Popover";

/** Notes save by themselves after this long without typing (SPEC.md, Editing). */
const AUTOSAVE_MS = 750;

type SaveState = "saved" | "dirty" | "saving" | "failed" | "conflict";
type Mode = "loading" | "editing" | "readonly" | "missing" | "trashed";

type Props = {
  noteId: string;
  vault: VaultInfo;
  livePreview: boolean;
  onLivePreview: (on: boolean) => void;
  /** Bumped by ⌘S. */
  saveTick: number;
  contextOpen: boolean;
  onToggleContext: () => void;
  /** Shown in the header while the context panel is hidden. */
  contextCounts: { repositories: number; tasks: number } | null;
  pinned: boolean;
  onTogglePin: () => void;
  onOpenNote: (noteId: string) => void;
  onNotice: (message: string) => void;
  onError: (message: string | null) => void;
  /** Receives the function that saves unsaved edits now. */
  flushRef?: React.MutableRefObject<(() => Promise<void>) | null>;
};

/**
 * One note: its header, its editor, and what to do when it changed on disk,
 * is gone, or is not editable. A save names the version it read and never
 * overwrites one it has not seen (docs/architecture.md, Save contract).
 */
export default function NotePane(props: Props) {
  const { noteId, vault, onError, onNotice, onOpenNote } = props;
  const [content, setContent] = useState<NoteContent | null>(null);
  const [mode, setMode] = useState<Mode>("loading");
  const [text, setText] = useState("");
  const [saveState, setSaveStateRaw] = useState<SaveState>("saved");
  const [searchPending, setSearchPending] = useState(false);
  const [disk, setDisk] = useState<{ text: string; version: string } | null>(
    null,
  );
  const [menuOpen, setMenuOpen] = useState(false);
  const [dialog, setDialog] = useState<
    "rename" | "match" | "history" | "compare" | null
  >(null);
  const [, setClock] = useState(0);

  // The editor's text, the text of the version it was read or saved as, and that version.
  const current = useRef("");
  const savedText = useRef("");
  const base = useRef("");
  const state = useRef<SaveState>("saved");
  const modeRef = useRef<Mode>("loading");
  const loaded = useRef(false);
  /** The save in flight; others wait for it. */
  const inflight = useRef<Promise<void> | null>(null);
  /** A version an outside change announced while a save was in flight. */
  const pendingOutside = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  /** The note as last read or saved, for code that runs after a save. */
  const noteRef = useRef<NoteSummary | null>(null);
  /** The title the file name matched when the note was read, while it
   * follows the title (SPEC.md, Note identity); null when it does not. */
  const followFrom = useRef<string | null>(null);
  /** The cursor's line, and its line when a save last changed the title:
   * the file is renamed once the cursor has left that line. */
  const cursorLine = useRef(1);
  const titleLine = useRef<number | null>(null);
  /** The title a rename was last tried for, so a refusal is not repeated. */
  const triedTitle = useRef<string | null>(null);

  const setSaveState = useCallback((s: SaveState) => {
    state.current = s;
    setSaveStateRaw(s);
  }, []);
  const setModeBoth = useCallback((m: Mode) => {
    modeRef.current = m;
    setMode(m);
  }, []);

  /** Edits not on disk: typed since the last save, or kept through a conflict. */
  const unsaved = useCallback(
    () =>
      loaded.current &&
      (current.current !== savedText.current || state.current === "conflict"),
    [],
  );

  /** Keep edits that cannot be saved now as the note's draft (SPEC.md, Saving). */
  const keepDraft = useCallback(async () => {
    if (!unsaved()) return;
    await ipc
      .saveDraft(noteId, base.current, current.current)
      .catch((e) => onError(errorMessage(e)));
  }, [noteId, onError, unsaved]);

  const fetchDisk = useCallback(() => {
    void ipc
      .readNote(noteId)
      .then(
        (c) => c.text !== null && setDisk({ text: c.text, version: c.version }),
      );
  }, [noteId]);

  /** Show what `read_note` returned. A reload never replaces unsaved edits:
   * they become a conflict with the version on disk. */
  const apply = useCallback(
    (c: NoteContent, reload: boolean) => {
      setContent(c);
      noteRef.current = c.note;
      const keepMine = reload && unsaved();
      if (c.note.trashed || c.note.missing) {
        const shown = keepMine
          ? current.current
          : (c.draft?.text ?? c.text ?? current.current);
        current.current = shown;
        setText(shown);
        setModeBoth(c.note.trashed ? "trashed" : "missing");
        if (keepMine) void keepDraft();
        return;
      }
      if (c.text === null) {
        setModeBoth("readonly");
        return;
      }
      setModeBoth("editing");
      if (keepMine) {
        // Typed while the reload was on its way: keep it against the old version.
        setDisk({ text: c.text, version: c.version });
        setSaveState("conflict");
        void keepDraft();
        return;
      }
      base.current = c.version;
      savedText.current = c.text;
      loaded.current = true;
      followFrom.current =
        c.note.title_file_name === null ? c.note.title : null;
      setDisk(null);
      if (c.draft && c.draft.text !== c.text) {
        current.current = c.draft.text;
        setText(c.draft.text);
        if (c.draft.base_version === c.version) {
          setSaveState("dirty");
          timer.current = setTimeout(() => void saveRef.current(), AUTOSAVE_MS);
        } else {
          // Edits made against a version that changed on disk since.
          base.current = c.draft.base_version;
          setDisk({ text: c.text, version: c.version });
          setSaveState("conflict");
        }
      } else {
        current.current = c.text;
        setText(c.text);
        setSaveState("saved");
      }
    },
    [setModeBoth, setSaveState, keepDraft, unsaved],
  );

  const load = useCallback(
    async (reload = true) => {
      try {
        apply(await ipc.readNote(noteId), reload);
      } catch (e) {
        onError(errorMessage(e));
      }
    },
    [noteId, apply, onError],
  );

  /** A change to the note from outside Brainiac, announced with its new version. */
  const outsideChange = useCallback(
    (version: string | null) => {
      if (version === null) {
        void load();
        return;
      }
      if (version === base.current) {
        // Renamed or new context; the text is the same.
        void ipc.readNote(noteId).then((c) => {
          noteRef.current = c.note;
          setContent(c);
        });
        return;
      }
      if (!unsaved()) {
        void load();
        return;
      }
      setSaveState("conflict");
      fetchDisk();
      void keepDraft();
    },
    [noteId, load, setSaveState, fetchDisk, keepDraft, unsaved],
  );

  /** One attempt to write the editor's text over the version it was read as. */
  const saveOnce = useCallback(async () => {
    const sent = current.current;
    pendingOutside.current = null;
    setSaveState("saving");
    try {
      const result = await ipc.saveNote({
        note_id: noteId,
        expected_version: base.current,
        text: sent,
      });
      base.current = result.version;
      savedText.current = sent;
      if (result.note.title !== noteRef.current?.title)
        titleLine.current = cursorLine.current;
      noteRef.current = result.note;
      setSearchPending(result.search_pending);
      setContent((c) =>
        c ? { ...c, note: result.note, version: result.version } : c,
      );
      setSaveState(current.current === sent ? "saved" : "dirty");
    } catch (e) {
      if (isAppError(e) && e.code === "CONFLICT") {
        setSaveState("conflict");
        fetchDisk();
        // The backend kept what was sent; keep what was typed since too.
        void keepDraft();
      } else if (isAppError(e) && e.code === "NOT_FOUND") {
        setSaveState("dirty");
        void load();
      } else {
        setSaveState("failed");
        onError(errorMessage(e));
      }
    }
    const outside = pendingOutside.current;
    pendingOutside.current = null;
    if (outside !== null && outside !== base.current) outsideChange(outside);
    else void followRef.current();
  }, [
    noteId,
    onError,
    setSaveState,
    fetchDisk,
    keepDraft,
    load,
    outsideChange,
  ]);

  /**
   * Save the editor's text, waiting for a save in flight first, until what
   * is on disk is what was typed. While the note cannot be saved (changed on
   * disk, gone) the edits are kept as its draft instead.
   */
  const save = useCallback(async (): Promise<void> => {
    clearTimeout(timer.current);
    while (inflight.current) await inflight.current;
    if (modeRef.current !== "editing" || state.current === "conflict") {
      await keepDraft();
      return;
    }
    if (current.current === savedText.current) {
      if (state.current !== "failed") setSaveState("saved");
      return;
    }
    const run = saveOnce();
    inflight.current = run;
    try {
      await run;
    } finally {
      inflight.current = null;
    }
    if (state.current === "dirty" && modeRef.current === "editing")
      await save();
  }, [keepDraft, saveOnce, setSaveState]);
  const saveRef = useRef(save);
  saveRef.current = save;

  /**
   * Rename the file after a changed title once the cursor has left the
   * title's line, or at once when `leaving` the note. The backend decides
   * whether it may (SPEC.md, Note identity).
   */
  const followTitle = useCallback(
    async (leaving = false) => {
      const note = noteRef.current;
      const from = followFrom.current;
      if (
        !note ||
        from === null ||
        modeRef.current !== "editing" ||
        state.current !== "saved" ||
        note.title === from ||
        note.title_file_name === null ||
        triedTitle.current === note.title ||
        (!leaving && cursorLine.current === titleLine.current)
      )
        return;
      triedTitle.current = note.title;
      try {
        const next = await ipc.followNoteTitle(noteId, from);
        if (next.relative_path !== note.relative_path)
          followFrom.current = next.title;
        noteRef.current = next;
        setContent((c) => (c ? { ...c, note: next } : c));
      } catch (e) {
        if (!leaving) onError(errorMessage(e));
      }
    },
    [noteId, onError],
  );
  const followRef = useRef(followTitle);
  followRef.current = followTitle;

  // Others save first before changing the note's file (linking writes its ID).
  useEffect(() => {
    if (!props.flushRef) return;
    props.flushRef.current = save;
    const ref = props.flushRef;
    return () => {
      if (ref.current === save) ref.current = null;
    };
  }, [props.flushRef, save]);

  // Closing the window saves unsaved edits first.
  useEffect(() => {
    const off = getCurrentWindow().onCloseRequested(async () => {
      await save();
      await followRef.current(true);
    });
    return () => {
      void off.then((unlisten) => unlisten());
    };
  }, [save]);

  // Load the note; mark it recent. Unsaved edits are saved when leaving it.
  useEffect(() => {
    void load(false);
    void ipc.markNoteOpened(noteId).catch(() => {});
    return () => {
      clearTimeout(timer.current);
      const saved = unsaved() ? saveRef.current() : Promise.resolve();
      void saved.then(() => followRef.current(true));
    };
  }, [noteId, load, unsaved]);

  // ⌘S saves at once.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only a new tick saves.
  useEffect(() => {
    if (props.saveTick) void save();
  }, [props.saveTick]);

  // Changes from outside: reload when there is nothing unsaved, otherwise
  // say the note changed on disk; a vanished file keeps the draft. An event
  // during a save is handled once the save is done.
  useEffect(
    () =>
      subscribe(
        onNoteChanged((e) => {
          if (e.note_id !== noteId) return;
          if (inflight.current) {
            if (e.version !== null) pendingOutside.current = e.version;
            else void inflight.current.then(() => outsideChange(null));
            return;
          }
          outsideChange(e.version);
        }),
        onNoteMissing((e) => {
          if (e.note_id === noteId) void load();
        }),
      ),
    [noteId, load, outsideChange],
  );

  // Keep "edited 2 minutes ago" current.
  useEffect(() => {
    const t = setInterval(() => setClock((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);

  const onChange = (next: string) => {
    current.current = next;
    setText(next);
    clearTimeout(timer.current);
    if (state.current !== "conflict") {
      if (next === savedText.current) {
        if (!inflight.current) setSaveState("saved");
        return;
      }
      setSaveState("dirty");
    }
    // In a conflict this keeps the draft current instead of saving.
    timer.current = setTimeout(() => void save(), AUTOSAVE_MS);
  };

  const note = content?.note ?? null;
  // Offered while the file name differs from the title and is not about to
  // follow it by itself.
  const titleName =
    mode === "editing" &&
    note?.title_file_name &&
    (followFrom.current === null || triedTitle.current === note.title)
      ? note.title_file_name
      : null;

  const openLink = async (link: NoteLink) => {
    try {
      if (link.kind === "url") {
        if (/^(https?|mailto):/i.test(link.target)) await openUrl(link.target);
        return;
      }
      const resolved = await ipc.resolveLink(noteId, link.target, !!link.wiki);
      if (resolved.note) {
        onOpenNote(resolved.note.id);
        return;
      }
      const create = await ask(
        `“${link.target}” does not exist yet. Create ${resolved.suggested_path}?`,
        { title: "Create Note", kind: "info", okLabel: "Create" },
      );
      if (!create) return;
      const created = await ipc.createNote({
        folder: folderOf(resolved.suggested_path) || null,
        title: stem(resolved.suggested_path),
      });
      onOpenNote(created.id);
    } catch (e) {
      onError(errorMessage(e));
    }
  };

  // Conflict actions.
  const reloadFromDisk = async () => {
    clearTimeout(timer.current);
    await ipc.discardDraft(noteId).catch(() => {});
    setSaveState("saved");
    current.current = savedText.current;
    setDialog(null);
    await load(false);
  };
  const keepMine = async () => {
    if (!disk) return;
    setDialog(null);
    base.current = disk.version;
    savedText.current = disk.text;
    setSaveState("dirty");
    setDisk(null);
    await save();
  };
  const saveCopy = async () => {
    try {
      const copy = await ipc.saveDraftAsCopy(noteId, current.current);
      await reloadFromDisk();
      onNotice(`Your draft was saved as ${copy.relative_path}`);
      onOpenNote(copy.id);
    } catch (e) {
      onError(errorMessage(e));
    }
  };

  // Missing and trashed notes.
  const restoreAsNew = async () => {
    try {
      const restored = await ipc.recreateNote(noteId, current.current);
      onNotice(`Restored as ${restored.relative_path}`);
      await load(false);
    } catch (e) {
      onError(errorMessage(e));
    }
  };
  const relink = async () => {
    try {
      const chosen = await open({
        multiple: false,
        directory: false,
        title: "Relink to a File",
        defaultPath: vault.root_path,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (typeof chosen !== "string") return;
      const rel = insideVault(vault.root_path, chosen);
      if (!rel) {
        onError("Choose a Markdown file inside the vault.");
        return;
      }
      await ipc.relinkNote(noteId, rel);
      await load(false);
    } catch (e) {
      onError(errorMessage(e));
    }
  };
  const restoreFromTrash = async () => {
    try {
      await ipc.restoreNote(noteId, false);
    } catch (e) {
      if (!(isAppError(e) && e.code === "CONFLICT")) {
        onError(errorMessage(e));
        return;
      }
      const overwrite = await ask(
        `${errorMessage(e)} Restore over it? The note there moves to the trash.`,
        { title: "Restore Note", kind: "warning", okLabel: "Restore" },
      );
      if (!overwrite) return;
      try {
        await ipc.restoreNote(noteId, true);
      } catch (e2) {
        onError(errorMessage(e2));
        return;
      }
    }
    await load(false);
  };
  const trash = async () => {
    setMenuOpen(false);
    await save();
    try {
      await ipc.trashNote(noteId);
      onNotice(`Moved “${note?.title ?? "the note"}” to the trash`);
    } catch (e) {
      onError(errorMessage(e));
    }
  };

  const label = (() => {
    if (mode === "missing") return "File gone";
    if (mode === "trashed") return "In the trash";
    if (mode === "readonly") return "Not editable here";
    switch (saveState) {
      case "saving":
        return "Saving…";
      case "dirty":
        return "Unsaved changes";
      case "failed":
        return "Save failed";
      case "conflict":
        return "Changed on disk";
      default:
        return searchPending
          ? "Saved · search update pending"
          : note
            ? `Saved · ${editedLabel(note.modified_at)}`
            : "Saved";
    }
  })();

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center gap-3 border-b bg-header px-4"
      >
        <div className="flex min-w-0 flex-col">
          <span className="truncate font-semibold">{note?.title ?? "…"}</span>
          <span className="flex min-w-0 items-baseline gap-2 text-[11px]">
            <span className="mono truncate text-muted">
              {note?.relative_path}
            </span>
            {titleName && (
              <button
                type="button"
                className="shrink-0 text-link hover:underline"
                title={`Rename the file to ${titleName}`}
                onClick={() => void save().then(() => setDialog("match"))}
              >
                Rename File to Match Title…
              </button>
            )}
          </span>
        </div>
        {note?.id_conflict && (
          <span
            className="flex shrink-0 items-center gap-1 text-[12px] text-dirty"
            title="Another note carries the same brainiac_id, probably a copy. Brainiac keeps them as separate notes."
          >
            <AlertIcon size={12} /> Duplicate ID
          </span>
        )}
        <div data-tauri-drag-region className="h-full flex-1" />
        <span
          role="status"
          className={`shrink-0 text-[12px] ${saveState === "failed" || saveState === "conflict" || mode === "missing" ? "font-medium text-conflict" : "text-muted"}`}
        >
          {label}
        </span>
        {saveState === "failed" && (
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => void save()}
          >
            Retry
          </button>
        )}
        {mode === "editing" && (
          <div role="tablist" aria-label="Editor mode" className="seg seg-sm">
            <button
              type="button"
              role="tab"
              aria-selected={props.livePreview}
              title="Live Preview (⇧⌘E)"
              onClick={() => props.onLivePreview(true)}
            >
              Live Preview
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={!props.livePreview}
              title="Source (⇧⌘E)"
              onClick={() => props.onLivePreview(false)}
            >
              Source
            </button>
          </div>
        )}
        {!props.contextOpen && props.contextCounts && (
          <span className="shrink-0 text-[12px] text-muted">
            {plural(
              props.contextCounts.repositories,
              "repository",
              "repositories",
            )}{" "}
            · {plural(props.contextCounts.tasks, "task")}
          </span>
        )}
        <button
          type="button"
          className="btn btn-ghost icon-btn"
          aria-label={
            props.contextOpen ? "Hide context panel" : "Show context panel"
          }
          aria-pressed={props.contextOpen}
          title="Context panel (⌥⌘0)"
          onClick={props.onToggleContext}
        >
          <PanelRightIcon />
        </button>
        <div className="relative">
          <button
            type="button"
            className="btn btn-ghost icon-btn"
            aria-label="Note actions"
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            disabled={!note}
            onClick={() => setMenuOpen(!menuOpen)}
          >
            <MoreIcon />
          </button>
          {menuOpen && note && (
            <Popover align="right" onClose={() => setMenuOpen(false)}>
              <MenuItem
                label={props.pinned ? "Unpin" : "Pin"}
                onClick={() => {
                  setMenuOpen(false);
                  props.onTogglePin();
                }}
              />
              {!note.missing && (
                <MenuItem
                  label="Rename or Move…"
                  onClick={() => {
                    setMenuOpen(false);
                    void save().then(() => setDialog("rename"));
                  }}
                />
              )}
              <MenuItem
                label="Version History…"
                onClick={() => {
                  setMenuOpen(false);
                  void save().then(() => setDialog("history"));
                }}
              />
              {!note.missing && (
                <>
                  <MenuItem
                    label="Reveal in Finder"
                    onClick={() => {
                      setMenuOpen(false);
                      void ipc
                        .revealVaultPath(note.relative_path)
                        .catch((e) => onError(errorMessage(e)));
                    }}
                  />
                  <MenuItem
                    label="Open in Default App"
                    onClick={() => {
                      setMenuOpen(false);
                      void ipc
                        .openVaultFile(note.relative_path)
                        .catch((e) => onError(errorMessage(e)));
                    }}
                  />
                  <div className="menu-sep" />
                  <MenuItem
                    label="Move to Trash"
                    onClick={() => void trash()}
                  />
                </>
              )}
            </Popover>
          )}
        </div>
      </header>

      {saveState === "conflict" && mode === "editing" && (
        <div className="banner" data-tone="warn" role="alert">
          <AlertIcon className="shrink-0 text-dirty" />
          <span className="flex-1">
            This note changed on disk since you opened it. Your edits are kept
            as a draft.
          </span>
          <button
            type="button"
            className="btn btn-sm"
            disabled={!disk}
            onClick={() => setDialog("compare")}
          >
            Compare…
          </button>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => void reloadFromDisk()}
          >
            Reload from Disk
          </button>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => void saveCopy()}
          >
            Save Draft as Copy
          </button>
        </div>
      )}
      {mode === "missing" && (
        <div className="banner" data-tone="warn" role="alert">
          <AlertIcon className="shrink-0 text-dirty" />
          <span className="flex-1">
            This note's file is gone. Its tasks and links are kept
            {text ? ", and so is its last text" : ""}.
          </span>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => void restoreAsNew()}
          >
            Restore as New File
          </button>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => void relink()}
          >
            Relink to a File…
          </button>
        </div>
      )}
      {mode === "trashed" && (
        <div className="banner" role="status">
          <span className="flex-1">
            This note is in Brainiac's trash. Its tasks and links are kept.
          </span>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => void restoreFromTrash()}
          >
            Restore
          </button>
        </div>
      )}

      <div className="relative min-h-0 flex-1 bg-app">
        {mode === "loading" && <div className="progress-line" />}
        {mode === "readonly" && note && (
          <div className="flex flex-col items-center gap-3 px-6 py-16 text-center text-muted">
            <NoteIcon size={28} />
            <p className="m-0 max-w-[420px]">
              {note.text_state === "too_large"
                ? "This note is over 5 MiB. Brainiac finds it by its name; edit it in another editor."
                : "This note is not UTF-8 text. Brainiac finds it by its name; open it in another editor."}
            </p>
            <button
              type="button"
              className="btn"
              onClick={() =>
                void ipc
                  .openVaultFile(note.relative_path)
                  .catch((e) => onError(errorMessage(e)))
              }
            >
              <ExternalIcon size={13} />
              Open Externally
            </button>
          </div>
        )}
        {(mode === "editing" || mode === "missing" || mode === "trashed") && (
          <NoteEditor
            key={mode === "editing" ? "edit" : "view"}
            text={text}
            livePreview={props.livePreview}
            readOnly={mode !== "editing"}
            onChange={onChange}
            onCursorLine={(line) => {
              cursorLine.current = line;
              void followTitle();
            }}
            onOpenLink={(l) => void openLink(l)}
            resolveImage={(src) =>
              note ? vaultImageUrl(note.relative_path, src) : null
            }
            className="note-editor"
          />
        )}
      </div>

      {(dialog === "rename" || dialog === "match") && note && (
        <RenameDialog
          note={note}
          initialPath={
            dialog === "match" && note.title_file_name
              ? [folderOf(note.relative_path), note.title_file_name]
                  .filter(Boolean)
                  .join("/")
              : undefined
          }
          onClose={() => setDialog(null)}
          onDone={(message) => {
            setDialog(null);
            onNotice(message);
            void ipc.readNote(noteId).then((c) => {
              noteRef.current = c.note;
              // A file named after its title follows it from here on; one
              // named otherwise keeps the name it was given.
              followFrom.current =
                c.note.title_file_name === null ? c.note.title : null;
              setContent(c);
            });
          }}
        />
      )}
      {dialog === "history" && note && (
        <HistoryDialog
          note={note}
          version={base.current}
          onClose={() => setDialog(null)}
          onRestored={() => {
            setDialog(null);
            void load(false);
          }}
        />
      )}
      {dialog === "compare" && disk && (
        <CompareDialog
          disk={disk.text}
          mine={current.current}
          onClose={() => setDialog(null)}
          onKeepMine={() => void keepMine()}
          onUseDisk={() => void reloadFromDisk()}
        />
      )}
    </div>
  );
}

function MenuItem({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      role="menuitem"
      className="menu-item"
      onClick={onClick}
    >
      {label}
    </button>
  );
}

export type { NoteSummary };
