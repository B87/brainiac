import { useEffect, useMemo, useRef, useState } from "react";
import { ENV_LABEL, queryMatches } from "../lib/databases";
import { shortPath } from "../lib/format";
import {
  type AppSnapshot,
  type DbConnection,
  errorMessage,
  ipc,
  type RepositorySummary,
  type SavedQuery,
  type SearchHit,
  type SearchResults,
} from "../lib/ipc";
import { repoTone } from "../lib/repo";
import { createLatest } from "../lib/stale";
import {
  DatabaseIcon,
  FetchIcon,
  FolderIcon,
  GridIcon,
  NoteIcon,
  PlayIcon,
  PlusIcon,
  SearchIcon,
  TaskIcon,
  TodayIcon,
} from "./icons";
import { Parts } from "./NoteTree";
import type { View } from "./Sidebar";

type Props = {
  snapshot: AppSnapshot;
  onClose: () => void;
  onView: (view: View) => void;
  /** Databases (v0.4): saved queries run at once; a connection opens a new query. */
  dbConnections: DbConnection[];
  dbQueries: SavedQuery[];
  onDatabase: (
    request:
      | { kind: "run_query"; queryId: string }
      | { kind: "new_query"; connectionId: string },
  ) => void;
  onOpenRepository: () => void;
  onNewWorkspace: () => void;
  onFetch: () => void;
  /** The open repository, which Locate Folder… applies to. */
  current: RepositorySummary | null;
  onLocate: (repositoryId: string) => void;
  onOpenNote: (noteId: string) => void;
  onOpenTask: (taskId: string) => void;
  onNewNote: () => void;
  onNewTask: () => void;
  onNewRun: () => void;
};

/** The palette's scope buttons (SPEC.md, Search). */
type Scope = "all" | "repositories" | "notes" | "tasks";

type Item = {
  id: string;
  label: React.ReactNode;
  hint: string;
  snippet?: React.ReactNode;
  icon: React.ReactNode;
  action: () => void;
};

type Group = {
  title: string;
  items: Item[];
  more?: { total: number; scope: Scope };
};

/** How many results each group shows before Show all. */
const FIRST = 6;

/**
 * ⌘K (SPEC.md, Search): repositories, workspaces, notes, and tasks
 * together, grouped by kind with repositories first. Notes and tasks are
 * found by keyword; input is literal text and never an error.
 */
export default function CommandPalette({
  snapshot,
  onClose,
  onView,
  onOpenRepository,
  onNewWorkspace,
  onFetch,
  current,
  onLocate,
  onOpenNote,
  onOpenTask,
  onNewNote,
  onNewTask,
  onNewRun,
  dbConnections,
  dbQueries,
  onDatabase,
}: Props) {
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<Scope>("all");
  const [index, setIndex] = useState(0);
  const [results, setResults] = useState<SearchResults | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const latest = useMemo(() => createLatest(), []);
  const q = query.trim();

  // Results follow the query; an older query's results never replace a newer one's.
  useEffect(() => {
    if (!q || scope === "repositories") {
      latest.cancel();
      setResults(null);
      return;
    }
    const kinds =
      scope === "notes"
        ? (["note"] as const)
        : scope === "tasks"
          ? (["task"] as const)
          : null;
    const t = setTimeout(() => {
      void latest.run(
        () =>
          ipc.search({
            query: q,
            kinds: kinds ? [...kinds] : null,
            limit: scope === "all" ? FIRST : 200,
          }),
        (r) => {
          setResults(r);
          setSearchError(null);
        },
        (e) => setSearchError(errorMessage(e)),
      );
    }, 90);
    return () => clearTimeout(t);
  }, [q, scope, latest]);

  // Results of an earlier query are not shown, so Enter never opens one of them.
  const currentResults = results && results.query === q ? results : null;
  const groups = useMemo<Group[]>(() => {
    const lower = q.toLowerCase();
    const match = (...fields: string[]) =>
      !lower || fields.some((f) => f.toLowerCase().includes(lower));
    const out: Group[] = [];
    if (scope === "all" || scope === "repositories") {
      const repos: Item[] = snapshot.repositories
        .filter((r) => match(r.name, r.display_path))
        .map((r) => ({
          id: `r:${r.id}`,
          label: r.name,
          hint: shortPath(r.display_path),
          icon: <span className="dot" data-state={repoTone(r)} />,
          action: () => onView({ kind: "repository", id: r.id }),
        }));
      const workspaces: Item[] = snapshot.workspaces
        .filter((w) => match(w.name, w.discovery_root ?? ""))
        .map((w) => ({
          id: `w:${w.id}`,
          label: w.name,
          hint: `workspace · ${w.members.length}`,
          icon: <GridIcon size={13} />,
          action: () => onView({ kind: "workspace", id: w.id }),
        }));
      const limit = scope === "all" && q ? FIRST : Number.POSITIVE_INFINITY;
      if (repos.length)
        out.push({
          title: "Repositories",
          items: repos.slice(0, limit),
          more:
            repos.length > limit
              ? { total: repos.length, scope: "repositories" }
              : undefined,
        });
      if (workspaces.length)
        out.push({ title: "Workspaces", items: workspaces.slice(0, limit) });
    }
    const hitItem = (h: SearchHit): Item => ({
      id: `${h.kind}:${h.id}`,
      label: <Parts parts={h.title} />,
      hint: h.detail,
      snippet: h.snippet.length ? <Parts parts={h.snippet} /> : undefined,
      icon: h.kind === "note" ? <NoteIcon size={13} /> : <TaskIcon size={13} />,
      action: () => (h.kind === "note" ? onOpenNote(h.id) : onOpenTask(h.id)),
    });
    if (
      currentResults &&
      (scope === "all" || scope === "notes") &&
      currentResults.notes.hits.length
    )
      out.push({
        title: "Notes",
        items: currentResults.notes.hits.map(hitItem),
        more:
          currentResults.notes.total > currentResults.notes.hits.length
            ? { total: Number(currentResults.notes.total), scope: "notes" }
            : undefined,
      });
    if (
      currentResults &&
      (scope === "all" || scope === "tasks") &&
      currentResults.tasks.hits.length
    )
      out.push({
        title: "Tasks",
        items: currentResults.tasks.hits.map(hitItem),
        more:
          currentResults.tasks.total > currentResults.tasks.hits.length
            ? { total: Number(currentResults.tasks.total), scope: "tasks" }
            : undefined,
      });
    if (scope === "all") {
      const nameOf = (id: string | null) =>
        dbConnections.find((c) => c.id === id)?.name ?? "no connection";
      const saved: Item[] = dbQueries
        .filter((query) => q && queryMatches(query, q) && query.connection_id)
        .slice(0, FIRST)
        .map((query) => ({
          id: `q:${query.id}`,
          label: query.name,
          hint: `Run Saved Query · ${nameOf(query.connection_id)}`,
          snippet: query.description || undefined,
          icon: <DatabaseIcon size={13} />,
          action: () => onDatabase({ kind: "run_query", queryId: query.id }),
        }));
      if (saved.length) out.push({ title: "Saved queries", items: saved });
      const databases: Item[] = dbConnections
        .filter((c) => q && match(c.name))
        .slice(0, FIRST)
        .map((c) => ({
          id: `db:${c.id}`,
          label: `New Query on ${c.name}`,
          hint: ENV_LABEL[c.environment],
          icon: <DatabaseIcon size={13} />,
          action: () => onDatabase({ kind: "new_query", connectionId: c.id }),
        }));
      if (databases.length) out.push({ title: "Databases", items: databases });
      const actions: Item[] = [
        {
          id: "today",
          label: "Today",
          hint: "",
          icon: <TodayIcon size={13} />,
          action: () => onView({ kind: "today" }),
        },
        {
          id: "tasks",
          label: "Tasks",
          hint: "",
          icon: <TaskIcon size={13} />,
          action: () => onView({ kind: "tasks" }),
        },
        {
          id: "notes",
          label: "Notes",
          hint: "",
          icon: <NoteIcon size={13} />,
          action: () => onView({ kind: "notes" }),
        },
        {
          id: "databases",
          label: "Databases",
          hint: "",
          icon: <DatabaseIcon size={13} />,
          action: () => onView({ kind: "databases" }),
        },
        {
          id: "runs",
          label: "Runs",
          hint: "",
          icon: <PlayIcon size={13} />,
          action: () => onView({ kind: "runs" }),
        },
        {
          id: "new-run",
          label: "New Run…",
          hint: "⌥⌘N",
          icon: <PlusIcon size={13} />,
          action: onNewRun,
        },
        {
          id: "new-note",
          label: "New Note",
          hint: "⌘N",
          icon: <PlusIcon size={13} />,
          action: onNewNote,
        },
        {
          id: "new-task",
          label: "New Task",
          hint: "⇧⌘N",
          icon: <PlusIcon size={13} />,
          action: onNewTask,
        },
        {
          id: "all",
          label: "All repositories",
          hint: "",
          icon: <GridIcon size={13} />,
          action: () => onView({ kind: "all" }),
        },
        {
          id: "open",
          label: "Open Repository…",
          hint: "⌘O",
          icon: <FolderIcon size={13} />,
          action: onOpenRepository,
        },
        {
          id: "new",
          label: "New Workspace…",
          hint: "",
          icon: <PlusIcon size={13} />,
          action: onNewWorkspace,
        },
        {
          id: "fetch",
          label: "Fetch Now",
          hint: "remote-tracking refs only",
          icon: <FetchIcon size={13} />,
          action: onFetch,
        },
        ...(current
          ? [
              {
                id: "locate",
                label: "Locate Folder…",
                hint: current.name,
                icon: <FolderIcon size={13} />,
                action: () => onLocate(current.id),
              },
            ]
          : []),
      ].filter((a) => match(String(a.label)));
      if (actions.length) out.push({ title: "Actions", items: actions });
    }
    return out;
  }, [
    snapshot,
    q,
    scope,
    currentResults,
    onView,
    onOpenRepository,
    onNewWorkspace,
    onFetch,
    current,
    onLocate,
    onOpenNote,
    onOpenTask,
    onNewNote,
    onNewTask,
    onNewRun,
    dbConnections,
    dbQueries,
    onDatabase,
  ]);

  const flat = groups.flatMap((g) => g.items);
  useEffect(() => {
    inputRef.current?.focus();
  }, []);
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new query or scope starts at the top.
  useEffect(() => setIndex(0), [q, scope]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") onClose();
    else if (e.key === "ArrowDown")
      setIndex((i) => Math.min(i + 1, flat.length - 1));
    else if (e.key === "ArrowUp") setIndex((i) => Math.max(i - 1, 0));
    else if (e.key === "Enter") flat[index]?.action();
    else return;
    e.preventDefault();
  };

  const index_ = results?.index;
  const searchesNotes = scope === "all" || scope === "notes";
  const nothing =
    q && flat.length === 0 && (currentResults || scope === "repositories");
  const quoted = q.includes('"');
  let position = -1;

  return (
    <div className="absolute inset-0 z-30 flex items-start justify-center bg-black/25 pt-24">
      <button
        type="button"
        aria-label="Close command palette"
        className="absolute inset-0"
        onMouseDown={onClose}
        onClick={onClose}
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Search and switch"
        tabIndex={-1}
        className="relative w-[620px] rounded-xl border border-control-line bg-header shadow-2xl"
        onKeyDown={onKeyDown}
      >
        <div className="flex items-center gap-2 border-b px-3.5 text-muted">
          <SearchIcon size={14} />
          <input
            ref={inputRef}
            className="selectable h-11 flex-1 bg-transparent text-[14px] outline-none"
            placeholder="Search notes, tasks, and repositories…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <div className="flex items-center gap-1.5 border-b px-3 py-1.5">
          {(
            [
              ["all", "All"],
              ["repositories", "Repositories"],
              ["notes", "Notes"],
              ["tasks", "Tasks"],
            ] as Array<[Scope, string]>
          ).map(([s, label]) => (
            <button
              key={s}
              type="button"
              className="chip h-6"
              aria-pressed={scope === s}
              onClick={() => {
                setScope(s);
                inputRef.current?.focus();
              }}
            >
              {label}
            </button>
          ))}
          <span className="ml-auto text-[11.5px] text-muted" role="status">
            {searchesNotes && index_?.state === "indexing"
              ? `Indexing notes: ${index_.done} of ${index_.total}. Results may be incomplete.`
              : searchesNotes && index_?.state === "no_vault"
                ? "No vault chosen: notes are not searched."
                : ""}
          </span>
        </div>
        {searchesNotes && index_?.state === "unavailable" && (
          <div className="flex items-center gap-2 border-b px-3.5 py-2 text-[12.5px]">
            <span className="flex-1">
              Search unavailable. Repository names still match.
            </span>
            <button
              type="button"
              className="btn btn-sm"
              onClick={() =>
                void ipc
                  .rebuildSearch()
                  .catch((e) => setSearchError(errorMessage(e)))
              }
            >
              Rebuild Index
            </button>
          </div>
        )}
        <div className="max-h-[440px] overflow-y-auto p-1.5">
          {searchError && (
            <div className="px-2 py-1 text-conflict">{searchError}</div>
          )}
          {nothing && (
            <div className="flex items-center gap-2 px-2 py-1 text-muted">
              No matches
              {quoted && (
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => setQuery(query.replaceAll('"', ""))}
                >
                  Search without quotes
                </button>
              )}
            </div>
          )}
          {groups.map((g) => (
            <div key={g.title} className="mb-1">
              <div className="section-label px-2 pt-1.5 pb-0.5">{g.title}</div>
              {g.items.map((it) => {
                position++;
                const at = position;
                return (
                  <button
                    type="button"
                    key={it.id}
                    className="menu-item h-auto min-h-8 py-1"
                    aria-current={at === index}
                    onMouseEnter={() => setIndex(at)}
                    onClick={it.action}
                  >
                    <span className="flex w-4 shrink-0 justify-center">
                      {it.icon}
                    </span>
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span className="flex min-w-0 items-center gap-2">
                        <span className="truncate">{it.label}</span>
                        <span className="muted-in-menu ml-auto shrink-0 truncate text-[11px] text-muted">
                          {it.hint}
                        </span>
                      </span>
                      {it.snippet && (
                        <span className="muted-in-menu truncate text-[11.5px] text-muted">
                          {it.snippet}
                        </span>
                      )}
                    </span>
                  </button>
                );
              })}
              {g.more && (
                <button
                  type="button"
                  className="btn btn-sm btn-ghost ml-6 text-link"
                  onClick={() => g.more && setScope(g.more.scope)}
                >
                  Show all {g.more.total}
                </button>
              )}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
