import { ask } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  errorMessage,
  type FolderEntry,
  type FolderListing,
  type IndexStatus,
  ipc,
  isAppError,
  type NoteLists,
  type NoteSummary,
  onNoteChanged,
  type SearchHit,
  subscribe,
  type TextPart,
  type TrashedNote,
  type VaultInfo,
} from "../lib/ipc";
import { useKeys } from "../lib/keys";
import { createLatest } from "../lib/stale";
import Dialog from "./Dialog";
import {
  ChevronDown,
  ChevronRight,
  FolderIcon,
  NoteIcon,
  PinIcon,
  PlusIcon,
  SearchIcon,
  TrashIcon,
} from "./icons";

const EXPANDED_KEY = "brainiac.notes.expanded";

function readExpanded(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(EXPANDED_KEY) ?? "[]"));
  } catch {
    return new Set();
  }
}

/** Highlighted parts of a search result's title. */
export function Parts({ parts }: { parts: TextPart[] }) {
  return (
    <>
      {parts.map((p, i) =>
        p.highlight ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: parts have no identity besides their position.
          <mark key={i} className="hit">
            {p.text}
          </mark>
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: as above.
          <span key={i}>{p.text}</span>
        ),
      )}
    </>
  );
}

/**
 * The vault's tree (SPEC.md, Notes view): pinned and recent notes, then the
 * folders as they are on disk, with a filter (`/`) that searches notes.
 */
export default function NoteTree({
  vault,
  index,
  selectedId,
  pinnedIds,
  onOpen,
  onNew,
  onError,
}: {
  vault: VaultInfo;
  index: IndexStatus;
  selectedId: string | null;
  pinnedIds: Set<string>;
  onOpen: (noteId: string) => void;
  /** New note in this folder ("" for the top). */
  onNew: (folder: string) => void;
  onError: (message: string | null) => void;
}) {
  const [listings, setListings] = useState<Map<string, FolderListing>>(
    new Map(),
  );
  const [expanded, setExpanded] = useState(readExpanded);
  const [lists, setLists] = useState<NoteLists | null>(null);
  const [filter, setFilter] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [trashOpen, setTrashOpen] = useState(false);
  const filterRef = useRef<HTMLInputElement>(null);
  const searchLatest = useMemo(() => createLatest(), []);

  const loadFolder = useCallback(
    async (folder: string) => {
      try {
        const listing = await ipc.listFolder(folder || undefined);
        setListings((prev) => new Map(prev).set(folder, listing));
      } catch (e) {
        // A folder renamed or removed meanwhile simply disappears.
        if (!(isAppError(e) && e.code === "NOT_FOUND"))
          onError(errorMessage(e));
        setListings((prev) => {
          const next = new Map(prev);
          next.delete(folder);
          return next;
        });
      }
    },
    [onError],
  );

  const reloadAll = useCallback(() => {
    void loadFolder("");
    for (const f of expanded) void loadFolder(f);
    void ipc
      .getNoteLists()
      .then(setLists)
      .catch(() => {});
  }, [expanded, loadFolder]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: reload when the vault or pins change.
  useEffect(reloadAll, [vault.id, pinnedIds]);
  useEffect(() => {
    let t: ReturnType<typeof setTimeout> | undefined;
    return subscribe(
      onNoteChanged(() => {
        clearTimeout(t);
        t = setTimeout(reloadAll, 300);
      }),
    );
  }, [reloadAll]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: a finished scan lists new notes.
  useEffect(() => {
    if (index.state === "ready") reloadAll();
  }, [index.state]);

  useEffect(() => {
    const q = filter.trim();
    if (!q) {
      searchLatest.cancel();
      setHits(null);
      return;
    }
    const t = setTimeout(() => {
      void searchLatest.run(
        () => ipc.search({ query: q, kinds: ["note"], limit: 50 }),
        (r) => setHits(r.notes.hits),
        (e) => onError(errorMessage(e)),
      );
    }, 120);
    return () => clearTimeout(t);
  }, [filter, searchLatest, onError]);

  useKeys({ "/": () => filterRef.current?.focus() });

  const toggle = (folder: string) => {
    const next = new Set(expanded);
    if (next.has(folder)) next.delete(folder);
    else {
      next.add(folder);
      void loadFolder(folder);
    }
    setExpanded(next);
    try {
      localStorage.setItem(EXPANDED_KEY, JSON.stringify([...next]));
    } catch {
      // Preference only.
    }
  };

  const noteRow = (n: NoteSummary, key: string, indent = 8) => (
    <button
      key={key}
      type="button"
      className={`side-row ${n.missing ? "text-muted" : ""}`}
      style={{ paddingLeft: indent }}
      aria-current={n.id === selectedId}
      title={n.relative_path}
      onClick={() => onOpen(n.id)}
    >
      <NoteIcon size={13} className="shrink-0 text-muted" />
      <span className={`flex-1 truncate ${n.missing ? "line-through" : ""}`}>
        {n.title}
      </span>
      {n.id_conflict && (
        <span className="text-[11px] text-dirty" title="Duplicate ID">
          !
        </span>
      )}
    </button>
  );

  const entryRows = (folder: string, depth: number): React.ReactNode[] => {
    const listing = listings.get(folder);
    if (!listing) return [];
    const indent = 8 + depth * 14;
    return listing.entries.flatMap((e: FolderEntry) => {
      if (e.kind === "folder") {
        const open = expanded.has(e.relative_path);
        return [
          <button
            key={`d:${e.relative_path}`}
            type="button"
            className="side-row"
            style={{ paddingLeft: indent - 2 }}
            aria-expanded={open}
            onClick={() => toggle(e.relative_path)}
          >
            {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
            <FolderIcon size={13} className="shrink-0 text-muted" />
            <span className="flex-1 truncate">{e.name}</span>
          </button>,
          ...(open ? entryRows(e.relative_path, depth + 1) : []),
        ];
      }
      if (e.kind === "note" && e.note)
        return [noteRow(e.note, `n:${e.relative_path}`, indent + 12)];
      return [
        <button
          key={`f:${e.relative_path}`}
          type="button"
          className="side-row text-muted"
          style={{ paddingLeft: indent + 12 }}
          title={
            e.kind === "note"
              ? "Not indexed yet"
              : `${e.name}: not a note. Click to open it in its default app.`
          }
          onClick={() =>
            void ipc
              .openVaultFile(e.relative_path)
              .catch((err) => onError(errorMessage(err)))
          }
        >
          <span className="w-[13px] shrink-0" />
          <span className="flex-1 truncate">{e.name}</span>
        </button>,
      ];
    });
  };

  const pinned = lists?.pinned ?? [];
  const recent = (lists?.recent ?? [])
    .filter((n) => !pinnedIds.has(n.id))
    .slice(0, 6);
  return (
    <nav
      aria-label="Notes"
      className="flex w-[250px] shrink-0 flex-col border-r bg-sidebar"
    >
      <div className="flex items-center gap-2 px-3 pt-3 pb-2">
        <span className="flex-1 truncate font-semibold" title={vault.root_path}>
          {vault.name}
        </span>
        <button
          type="button"
          className="btn btn-sm btn-ghost px-1.5"
          aria-label="New note"
          title="New note (⌘N)"
          onClick={() => onNew("")}
        >
          <PlusIcon size={13} />
        </button>
      </div>
      <div className="px-3 pb-2">
        <div className="search">
          <SearchIcon />
          <input
            ref={filterRef}
            aria-label="Filter notes"
            placeholder="Filter notes  /"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setFilter("");
                e.currentTarget.blur();
              } else if (e.key === "Enter" && hits?.[0]) onOpen(hits[0].id);
            }}
          />
        </div>
        {index.state === "indexing" && (
          <p className="m-0 mt-1.5 text-[11.5px] text-muted" role="status">
            Indexing notes: {index.done} of {index.total}. Results may be
            incomplete.
          </p>
        )}
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2 pb-2">
        {hits ? (
          <>
            {hits.length === 0 && (
              <p className="m-0 px-2 py-1 text-[12px] text-muted">No matches</p>
            )}
            {hits.map((h) => (
              <button
                key={h.id}
                type="button"
                className="side-row h-auto flex-col items-start gap-0 py-1"
                aria-current={h.id === selectedId}
                onClick={() => onOpen(h.id)}
              >
                <span className="max-w-full truncate">
                  <Parts parts={h.title} />
                </span>
                <span className="mono max-w-full truncate text-[11px] text-muted">
                  {h.detail}
                </span>
              </button>
            ))}
          </>
        ) : (
          <>
            {pinned.length > 0 && (
              <div className="section-label flex items-center gap-1 px-2 pt-2 pb-1">
                <PinIcon size={11} /> Pinned
              </div>
            )}
            {pinned.map((n) => noteRow(n, `p:${n.id}`))}
            {recent.length > 0 && (
              <div className="section-label px-2 pt-3 pb-1">Recent</div>
            )}
            {recent.map((n) => noteRow(n, `r:${n.id}`))}
            <div className="section-label px-2 pt-3 pb-1">Folders</div>
            {entryRows("", 0)}
            {listings.get("")?.entries.length === 0 && (
              <p className="m-0 px-2 py-1 text-[12px] text-muted">
                The vault is empty. Create a note with ⌘N.
              </p>
            )}
          </>
        )}
      </div>
      <div className="border-t p-2">
        <button
          type="button"
          className="side-row text-fg-2"
          onClick={() => setTrashOpen(true)}
        >
          <TrashIcon className="shrink-0" />
          <span className="flex-1">Trash</span>
        </button>
      </div>
      {trashOpen && (
        <TrashDialog
          onClose={() => setTrashOpen(false)}
          onOpen={(id) => {
            setTrashOpen(false);
            onOpen(id);
          }}
          onError={onError}
        />
      )}
    </nav>
  );
}

/** Notes in Brainiac's trash, which lives in its data folder, never the vault. */
function TrashDialog({
  onClose,
  onOpen,
  onError,
}: {
  onClose: () => void;
  onOpen: (noteId: string) => void;
  onError: (message: string | null) => void;
}) {
  const [items, setItems] = useState<TrashedNote[] | null>(null);
  const load = useCallback(() => {
    void ipc
      .listTrash()
      .then(setItems)
      .catch((e) => onError(errorMessage(e)));
  }, [onError]);
  useEffect(load, [load]);

  const restore = async (item: TrashedNote) => {
    try {
      await ipc.restoreNote(item.note.id, false);
    } catch (e) {
      if (!(isAppError(e) && e.code === "CONFLICT")) {
        onError(errorMessage(e));
        return;
      }
      const ok = await ask(
        `${errorMessage(e)} Restore over it? The note there moves to the trash.`,
        { title: "Restore Note", kind: "warning", okLabel: "Restore" },
      );
      if (!ok) return;
      await ipc
        .restoreNote(item.note.id, true)
        .catch((e2) => onError(errorMessage(e2)));
    }
    load();
  };

  return (
    <Dialog title="Trash" onClose={onClose} width={560}>
      <p className="mt-0 text-[12px] text-muted">
        Deleted notes are kept in Brainiac's data folder, outside the vault,
        with their tasks and links. Brainiac never deletes a note for good on
        its own.
      </p>
      {items?.length === 0 && <p className="text-muted">The trash is empty.</p>}
      <div className="flex flex-col">
        {items?.map((item) => (
          <div
            key={item.note.id}
            className="flex items-center gap-2 border-b py-1.5"
          >
            <button
              type="button"
              className="flex min-w-0 flex-1 flex-col items-start border-0 bg-transparent p-0 text-left font-[inherit] text-[inherit]"
              onClick={() => onOpen(item.note.id)}
            >
              <span className="max-w-full truncate">{item.note.title}</span>
              <span className="mono max-w-full truncate text-[11px] text-muted">
                {item.note.relative_path}
              </span>
            </button>
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => void restore(item)}
            >
              Restore
            </button>
          </div>
        ))}
      </div>
    </Dialog>
  );
}
