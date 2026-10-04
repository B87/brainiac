import { ask } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { addCloseGuard } from "../lib/closeGuard";
import {
  byFolder,
  connectionPlace,
  defaultMode,
  ENV_SHORT,
  formatBytes,
  kindLabel,
  queryMarkdown,
  queryMatches,
  untitled,
} from "../lib/databases";
import {
  type DbConnection,
  errorMessage,
  ipc,
  type QueryTab,
  type SavedQuery,
  type TransactionInfo,
} from "../lib/ipc";
import { useKeys } from "../lib/keys";
import DbConnectionDialog from "./DbConnectionDialog";
import { SaveQueryDialog } from "./DbDialogs";
import DbHealth from "./DbHealth";
import DbQueryTab, { type SchemaState } from "./DbQueryTab";
import {
  CloseIcon,
  DatabaseIcon,
  HomeIcon,
  MoreIcon,
  PlusIcon,
  PulseIcon,
  SearchIcon,
} from "./icons";
import Popover from "./Popover";

type HealthTab = { kind: "health"; id: string; connection_id: string };
type Tab = ({ kind: "query" } & QueryTab) | HealthTab;

/** What another view asks Databases to open, such as from ⌘K. */
export type DatabasesRequest =
  | { kind: "run_query"; queryId: string; tick: number }
  | { kind: "open_query"; queryId: string; tick: number }
  | { kind: "new_query"; connectionId: string; tick: number };

type Props = {
  visible: boolean;
  /** Bumped by ⌘S while Databases is shown. */
  saveTick: number;
  request: DatabasesRequest | null;
  onNotice: (text: string) => void;
  onError: (text: string) => void;
  /** Connections or saved queries changed, for ⌘K. */
  onLists: (connections: DbConnection[], queries: SavedQuery[]) => void;
};

/** The query tabs as they are kept for the next run. */
function savedTabs(tabs: Tab[], connections: DbConnection[]): QueryTab[] {
  return tabs.flatMap((t) =>
    t.kind === "query"
      ? [
          {
            id: t.id,
            connection_id: t.connection_id,
            saved_query_id: t.saved_query_id,
            title: t.title,
            text: t.text,
            saved_version: t.saved_version,
            dirty: t.dirty,
            // Read and write on production lasts for the tab only, not across runs.
            mode:
              connections.find((c) => c.id === t.connection_id)?.environment ===
              "production"
                ? "read_only"
                : t.mode,
          },
        ]
      : [],
  );
}

const newId = () =>
  globalThis.crypto?.randomUUID?.() ?? `tab-${Date.now()}-${Math.random()}`;

/** Databases (SPEC.md, section 11): Home and the query tabs. */
export default function DatabasesView(props: Props) {
  const [connections, setConnections] = useState<DbConnection[]>([]);
  const [queries, setQueries] = useState<SavedQuery[]>([]);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [active, setActive] = useState<string>("home");
  const strip = useRef<HTMLDivElement>(null);

  // The strip has no scrollbar, so the selected tab scrolls itself into view.
  // biome-ignore lint/correctness/useExhaustiveDependencies: a newly selected tab is the signal.
  useEffect(() => {
    strip.current
      ?.querySelector('[aria-selected="true"]')
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [active]);
  const [loaded, setLoaded] = useState(false);
  // Tabs that could not be restored are not overwritten by an empty list.
  const restoreFailed = useRef(false);
  const latest = useRef({ tabs, connections });
  latest.current = { tabs, connections };
  const [schemas, setSchemas] = useState<Map<string, SchemaState>>(new Map());
  const [editing, setEditing] = useState<DbConnection | "new" | null>(null);
  const [saving, setSaving] = useState<string | null>(null);
  const [autoRun, setAutoRun] = useState<string | null>(null);
  const transactions = useRef(new Map<string, TransactionInfo>());

  const reload = useCallback(async () => {
    try {
      const [c, q] = await Promise.all([
        ipc.listDbConnections(),
        ipc.listSavedQueries(),
      ]);
      setConnections(c);
      setQueries(q);
    } catch (e) {
      props.onError(errorMessage(e));
    }
  }, [props.onError]);

  // Tabs and their text come back from the last run (history.db).
  // biome-ignore lint/correctness/useExhaustiveDependencies: once.
  useEffect(() => {
    void reload();
    void ipc
      .listQueryTabs()
      .then((saved) => {
        setTabs(saved.map((t) => ({ ...t, kind: "query" as const })));
        setLoaded(true);
      })
      .catch(() => {
        restoreFailed.current = true;
        setLoaded(true);
      });
  }, []);

  useEffect(() => {
    props.onLists(connections, queries);
  }, [connections, queries, props.onLists]);

  // Tabs are saved a moment after each change.
  useEffect(() => {
    if (!loaded || restoreFailed.current) return;
    const t = setTimeout(() => {
      void ipc.saveQueryTabs(savedTabs(tabs, connections)).catch(() => {});
    }, 600);
    return () => clearTimeout(t);
  }, [tabs, loaded, connections]);

  // Closing the window, which quits (also ⌘Q), asks first when a tab has a
  // transaction open; the default is Roll Back. The backend's list counts,
  // since it knows of a transaction a running statement just opened.
  useEffect(
    () =>
      addCloseGuard(async () => {
        const known = await ipc.openDbTransactions().catch(() => []);
        const open = [...new Set([...transactions.current.keys(), ...known])];
        if (open.length > 0) {
          const sure = await ask(
            `${open.length === 1 ? "A query tab has a transaction" : `${open.length} query tabs have transactions`} open. Quitting rolls ${open.length === 1 ? "it" : "them"} back.`,
            {
              title: "Roll Back and Quit",
              kind: "warning",
              okLabel: "Roll Back",
            },
          );
          if (!sure) return false;
          for (const id of open)
            await ipc.endTransaction(id, false).catch(() => {});
          transactions.current.clear();
        }
        // The last edits are kept, not left to the save a moment later.
        if (!restoreFailed.current) {
          const { tabs, connections } = latest.current;
          await ipc.saveQueryTabs(savedTabs(tabs, connections)).catch(() => {});
        }
        return true;
      }),
    [],
  );

  // Connections whose schema is being read: tabs that ask at once share it.
  const schemaLoads = useRef(new Set<string>());
  const loadSchema = useCallback(
    async (connectionId: string, refresh: boolean) => {
      if (!refresh && schemaLoads.current.has(connectionId)) return;
      schemaLoads.current.add(connectionId);
      setSchemas((m) => {
        const next = new Map(m);
        const before = m.get(connectionId);
        next.set(connectionId, { schema: before?.schema ?? null, error: null });
        return next;
      });
      try {
        const schema = await ipc.getDbSchema(connectionId, refresh);
        setSchemas((m) =>
          new Map(m).set(connectionId, { schema, error: null }),
        );
      } catch (e) {
        setSchemas((m) =>
          new Map(m).set(connectionId, {
            schema: null,
            error: errorMessage(e),
          }),
        );
      } finally {
        schemaLoads.current.delete(connectionId);
      }
    },
    [],
  );

  const patchTab = useCallback((id: string, patch: Partial<QueryTab>) => {
    setTabs((ts) =>
      ts.map((t) =>
        t.id === id && t.kind === "query" ? { ...t, ...patch } : t,
      ),
    );
  }, []);

  const newTab = useCallback(
    (fields: Partial<QueryTab> = {}, run = false) => {
      const connectionId =
        fields.connection_id ??
        (() => {
          const current = tabs.find((t) => t.id === active);
          return current?.kind === "query"
            ? current.connection_id
            : (connections[0]?.id ?? null);
        })();
      const connection = connections.find((c) => c.id === connectionId) ?? null;
      const tab: Tab = {
        kind: "query",
        id: newId(),
        connection_id: connectionId,
        saved_query_id: null,
        title: untitled(tabs.filter((t) => t.kind === "query") as QueryTab[]),
        text: "",
        saved_version: null,
        dirty: false,
        mode: defaultMode(connection),
        ...fields,
      };
      setTabs((ts) => [...ts, tab]);
      setActive(tab.id);
      if (run) setAutoRun(tab.id);
      return tab.id;
    },
    [tabs, active, connections],
  );

  const openQuery = useCallback(
    (q: SavedQuery, run = false) => {
      const already = tabs.find(
        (t) => t.kind === "query" && t.saved_query_id === q.id,
      );
      if (already) {
        setActive(already.id);
        if (run) setAutoRun(already.id);
        return;
      }
      newTab(
        {
          connection_id: q.connection_id,
          saved_query_id: q.id,
          title: q.name,
          text: q.sql,
          saved_version: q.version,
          dirty: false,
        },
        run,
      );
    },
    [tabs, newTab],
  );

  const openHealth = (connectionId: string) => {
    const already = tabs.find(
      (t) => t.kind === "health" && t.connection_id === connectionId,
    );
    if (already) {
      setActive(already.id);
      return;
    }
    const tab: HealthTab = {
      kind: "health",
      id: newId(),
      connection_id: connectionId,
    };
    setTabs((ts) => [...ts, tab]);
    setActive(tab.id);
  };

  const closeTab = async (id: string) => {
    const tab = tabs.find((t) => t.id === id);
    if (!tab) return;
    if (tab.kind === "query") {
      if (transactions.current.has(id)) {
        const sure = await ask(
          "This tab has a transaction open. Closing the tab rolls it back.",
          {
            title: "Roll Back and Close",
            kind: "warning",
            okLabel: "Roll Back",
          },
        );
        if (!sure) return;
        await ipc.endTransaction(id, false).catch(() => {});
        transactions.current.delete(id);
      }
      const saved = queries.find((q) => q.id === tab.saved_query_id);
      if (tab.dirty && tab.text.trim() && !saved) {
        const sure = await ask(
          `“${tab.title}” has text that is not saved as a query. Close it anyway?`,
          { title: "Close Tab", kind: "warning", okLabel: "Close" },
        );
        if (!sure) return;
      }
      void ipc.closeDbSession(id).catch(() => {});
    }
    const index = tabs.findIndex((t) => t.id === id);
    const rest = tabs.filter((t) => t.id !== id);
    setTabs(rest);
    if (active === id)
      setActive(rest[Math.min(index, rest.length - 1)]?.id ?? "home");
  };

  // ⌘T opens a tab; ⌘1 is Home and ⌘2…⌘9 the tabs after it.
  const keys = useMemo(() => {
    if (!props.visible) return {};
    const map: Record<string, () => void> = { "mod+t": () => newTab() };
    for (let n = 1; n <= 9; n += 1)
      map[`mod+${n}`] = () =>
        setActive(n === 1 ? "home" : (tabs[n - 2]?.id ?? active));
    return map;
  }, [props.visible, newTab, tabs, active]);
  useKeys(keys);

  // ⌘S saves the active tab's query.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only a new tick saves.
  useEffect(() => {
    if (!props.saveTick || !props.visible) return;
    const tab = tabs.find((t) => t.id === active);
    if (tab?.kind === "query") setSaving(tab.id);
  }, [props.saveTick]);

  // Requests from ⌘K.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only a new request acts.
  useEffect(() => {
    const r = props.request;
    if (!r) return;
    if (r.kind === "new_query") {
      newTab({ connection_id: r.connectionId });
      return;
    }
    const q = queries.find((x) => x.id === r.queryId);
    if (q) openQuery(q, r.kind === "run_query");
  }, [props.request?.tick]);

  const savingTab = tabs.find((t) => t.id === saving);

  return (
    <div
      className="min-h-0 flex-1 flex-col"
      style={{ display: props.visible ? "flex" : "none" }}
    >
      <div
        role="tablist"
        aria-label="Query tabs"
        className="flex h-12 shrink-0 items-end gap-0.5 border-b bg-header px-2.5"
        data-tauri-drag-region
      >
        <button
          type="button"
          role="tab"
          aria-selected={active === "home"}
          className="db-tab"
          title="Databases home (⌘1)"
          onClick={() => setActive("home")}
        >
          <HomeIcon size={13} />
          Home
        </button>
        <div
          ref={strip}
          className="db-tabs"
          onWheel={(e) => {
            // A mouse wheel scrolls the strip sideways; it has no scrollbar.
            if (e.deltaX === 0) e.currentTarget.scrollLeft += e.deltaY;
          }}
        >
          {tabs.map((t) => {
            const connection = connections.find(
              (c) => c.id === t.connection_id,
            );
            const title =
              t.kind === "health"
                ? `${connection?.name ?? "?"} health`
                : t.title;
            return (
              <div key={t.id} className="flex">
                <button
                  type="button"
                  role="tab"
                  aria-selected={active === t.id}
                  className="db-tab"
                  title={
                    connection
                      ? `${title} · ${connection.name} · ${ENV_SHORT[connection.environment]}`
                      : title
                  }
                  onClick={() => setActive(t.id)}
                  onAuxClick={(e) => {
                    if (e.button === 1) void closeTab(t.id);
                  }}
                >
                  {t.kind === "health" ? (
                    <PulseIcon size={12} />
                  ) : (
                    <span
                      className="env-dot"
                      data-env={connection?.environment}
                      title={
                        connection
                          ? ENV_SHORT[connection.environment]
                          : "No connection"
                      }
                    />
                  )}
                  <span className="truncate">{title}</span>
                  {t.kind === "query" && t.dirty && (
                    <span className="unsaved-dot" title="Unsaved edits" />
                  )}
                  {/* biome-ignore lint/a11y/useSemanticElements: a close control inside the tab. */}
                  <span
                    role="button"
                    tabIndex={-1}
                    className="close"
                    aria-label={`Close ${title}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      void closeTab(t.id);
                    }}
                    onKeyDown={() => {}}
                  >
                    <CloseIcon size={10} />
                  </span>
                </button>
              </div>
            );
          })}
        </div>
        <button
          type="button"
          className="btn btn-ghost icon-btn mb-1"
          aria-label="New tab"
          title="New tab (⌘T)"
          onClick={() => newTab()}
        >
          <PlusIcon size={13} />
        </button>
        <div className="h-full flex-1" data-tauri-drag-region />
      </div>
      {active === "home" && (
        <Home
          connections={connections}
          queries={queries}
          onNewConnection={() => setEditing("new")}
          onEdit={setEditing}
          onDeleted={() => void reload()}
          onNewQuery={(c) => newTab({ connection_id: c.id })}
          onHealth={openHealth}
          onOpenQuery={(q, run) => openQuery(q, run)}
          onQueriesChanged={() => void reload()}
          onNotice={props.onNotice}
          onError={props.onError}
        />
      )}
      {tabs.map((t) => {
        if (t.kind === "health") {
          const connection = connections.find((c) => c.id === t.connection_id);
          if (!connection) return null;
          return (
            <DbHealth
              key={t.id}
              connection={connection}
              visible={props.visible && active === t.id}
              onEditConnection={() => setEditing(connection)}
              onOpenSql={(sql, title) =>
                newTab({
                  connection_id: connection.id,
                  text: sql,
                  title:
                    title ??
                    untitled(
                      tabs.filter((x) => x.kind === "query") as QueryTab[],
                    ),
                  dirty: true,
                })
              }
              onNotice={props.onNotice}
            />
          );
        }
        return (
          <DbQueryTab
            key={t.id}
            tab={t}
            visible={props.visible && active === t.id}
            connections={connections}
            queries={queries}
            schema={t.connection_id ? schemas.get(t.connection_id) : undefined}
            onSchema={(id, refresh) => void loadSchema(id, refresh)}
            onChange={(patch) => patchTab(t.id, patch)}
            onConnectionChanged={(c) =>
              setConnections((cs) => cs.map((x) => (x.id === c.id ? c : x)))
            }
            onOpenSql={(sql, title) =>
              newTab({
                connection_id: t.connection_id,
                text: sql,
                title:
                  title ??
                  untitled(
                    tabs.filter((x) => x.kind === "query") as QueryTab[],
                  ),
                dirty: true,
              })
            }
            onOpenQuery={(q) => openQuery(q)}
            onSave={() => setSaving(t.id)}
            onNewConnection={() => setEditing("new")}
            onTransaction={(info) => {
              if (info) transactions.current.set(t.id, info);
              else transactions.current.delete(t.id);
            }}
            onNotice={props.onNotice}
            autoRun={autoRun === t.id}
            onAutoRan={() => setAutoRun(null)}
          />
        );
      })}
      {editing && (
        <DbConnectionDialog
          connection={editing === "new" ? undefined : editing}
          onClose={() => setEditing(null)}
          onSaved={(c) => {
            setEditing(null);
            void reload();
            setSchemas((m) => {
              const next = new Map(m);
              next.delete(c.id);
              return next;
            });
            props.onNotice(`Saved ${c.name}`);
          }}
        />
      )}
      {savingTab && savingTab.kind === "query" && (
        <SaveQueryDialog
          sql={savingTab.text}
          existing={
            queries.find((q) => q.id === savingTab.saved_query_id) ?? null
          }
          expectedVersion={savingTab.saved_version}
          connections={connections}
          connectionId={savingTab.connection_id}
          folders={[...new Set(queries.map((q) => q.folder).filter(Boolean))]}
          onClose={() => setSaving(null)}
          onSaved={(q) => {
            setSaving(null);
            patchTab(savingTab.id, {
              saved_query_id: q.id,
              title: q.name,
              saved_version: q.version,
              dirty: false,
            });
            void reload();
            props.onNotice(`Saved ${q.name}`);
          }}
        />
      )}
    </div>
  );
}

function Home({
  connections,
  queries,
  onNewConnection,
  onEdit,
  onDeleted,
  onNewQuery,
  onHealth,
  onOpenQuery,
  onQueriesChanged,
  onNotice,
  onError,
}: {
  connections: DbConnection[];
  queries: SavedQuery[];
  onNewConnection: () => void;
  onEdit: (c: DbConnection) => void;
  onDeleted: () => void;
  onNewQuery: (c: DbConnection) => void;
  onHealth: (connectionId: string) => void;
  onOpenQuery: (q: SavedQuery, run: boolean) => void;
  onQueriesChanged: () => void;
  onNotice: (text: string) => void;
  onError: (text: string) => void;
}) {
  const [filter, setFilter] = useState("");
  const [menu, setMenu] = useState<string | null>(null);
  const lower = filter.trim().toLowerCase();
  const shownConnections = connections.filter(
    (c) =>
      !lower ||
      c.name.toLowerCase().includes(lower) ||
      connectionPlace(c).toLowerCase().includes(lower),
  );
  const groups = byFolder(queries.filter((q) => queryMatches(q, filter)));
  const nameOf = (id: string | null) =>
    connections.find((c) => c.id === id)?.name ?? null;

  const remove = async (c: DbConnection) => {
    const used = queries.filter((q) => q.connection_id === c.id).length;
    const sure = await ask(
      `${c.name} is removed from Brainiac${c.password.kind === "store" ? ", with its password in the Keychain" : ""}.${used ? ` ${used} saved quer${used === 1 ? "y" : "ies"} will have no connection.` : ""} The database itself is not touched.`,
      { title: "Delete Connection", kind: "warning", okLabel: "Delete" },
    );
    if (!sure) return;
    try {
      await ipc.deleteDbConnection(c.id, c.version);
      onDeleted();
    } catch (e) {
      onError(errorMessage(e));
    }
  };

  const removeQuery = async (q: SavedQuery) => {
    const sure = await ask(`The saved query “${q.name}” is deleted.`, {
      title: "Delete Saved Query",
      kind: "warning",
      okLabel: "Delete",
    });
    if (!sure) return;
    try {
      await ipc.deleteQuery(q.id, q.version);
      onQueriesChanged();
    } catch (e) {
      onError(errorMessage(e));
    }
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-270 flex-col gap-6 px-6 py-6">
        <div className="flex items-center gap-3">
          <DatabaseIcon size={18} className="text-fg-2" />
          <h1 className="m-0 text-[18px] font-semibold">Databases</h1>
          <div className="ml-4 flex max-w-90 flex-1 items-center gap-1.5 rounded-md border border-control-line bg-field px-2 py-1 text-muted">
            <SearchIcon size={12} />
            <input
              className="min-w-0 flex-1 bg-transparent text-[12.5px] text-fg outline-none"
              placeholder="Find a connection or saved query"
              aria-label="Find a connection or saved query"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
          </div>
          <span className="flex-1" />
          <button
            type="button"
            className="btn btn-primary"
            onClick={onNewConnection}
          >
            <PlusIcon size={12} />
            New Connection…
          </button>
        </div>

        <section className="flex flex-col gap-2">
          <h2 className="section-label m-0">Connections</h2>
          {connections.length === 0 ? (
            <div className="flex flex-col items-start gap-2 rounded-[10px] border border-dashed border-control-line p-5">
              <span className="text-[13px] font-medium">
                No connections yet
              </span>
              <span className="text-[12.5px] text-muted">
                Add a SQLite file or a PostgreSQL server. Every connection is
                read only unless you allow writes, and passwords stay in the
                Keychain or wherever you already keep them.
              </span>
              <button
                type="button"
                className="btn mt-1"
                onClick={onNewConnection}
              >
                New Connection…
              </button>
            </div>
          ) : (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-3">
              {shownConnections.map((c) => (
                <div
                  key={c.id}
                  className="flex flex-col gap-2 rounded-[10px] border bg-app p-3.5"
                  data-env={c.environment}
                  style={{ borderLeft: "3px solid var(--env)" }}
                >
                  <div className="flex items-center gap-2">
                    <span className="font-semibold">{c.name}</span>
                    <span className="env-badge" data-env={c.environment}>
                      {ENV_SHORT[c.environment]}
                    </span>
                    <span className="flex-1" />
                    <div className="relative">
                      <button
                        type="button"
                        className="btn btn-sm btn-ghost px-1"
                        aria-label={`More for ${c.name}`}
                        onClick={() => setMenu(menu === c.id ? null : c.id)}
                      >
                        <MoreIcon size={13} />
                      </button>
                      {menu === c.id && (
                        <Popover align="right" onClose={() => setMenu(null)}>
                          <button
                            type="button"
                            role="menuitem"
                            className="menu-item"
                            onClick={() => {
                              setMenu(null);
                              onEdit(c);
                            }}
                          >
                            Edit Connection…
                          </button>
                          <button
                            type="button"
                            role="menuitem"
                            className="menu-item"
                            onClick={() => {
                              setMenu(null);
                              void remove(c);
                            }}
                          >
                            Delete Connection…
                          </button>
                        </Popover>
                      )}
                    </div>
                  </div>
                  <div
                    className="mono truncate text-[12px] text-fg-2"
                    title={connectionPlace(c)}
                  >
                    {kindLabel(c)} · {connectionPlace(c)}
                  </div>
                  <div className="text-[11.5px] text-muted">
                    {c.access === "read_write" ? "Read and write" : "Read only"}
                    {c.kind === "sqlite"
                      ? c.file_size !== null
                        ? ` · ${formatBytes(c.file_size)}`
                        : " · file missing"
                      : c.credential.needs_approval
                        ? " · password source to allow in Settings → Secrets"
                        : c.credential.pending === "save"
                          ? " · last save did not finish"
                          : c.password.kind === "ask" && !c.password_ready
                            ? " · asks for its password"
                            : ""}
                  </div>
                  <div className="mt-1 flex gap-2">
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => onNewQuery(c)}
                    >
                      New Query
                    </button>
                    {c.kind === "postgres" && (
                      <button
                        type="button"
                        className="btn btn-sm btn-ghost"
                        title="Connections, sessions, locks, and the machine it runs on"
                        onClick={() => onHealth(c.id)}
                      >
                        <PulseIcon size={12} />
                        Health
                      </button>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </section>

        <section className="flex flex-col gap-2">
          <h2 className="section-label m-0">Saved queries</h2>
          {queries.length === 0 ? (
            <p className="m-0 text-[12.5px] text-muted">
              ⌘S in a query tab saves its text with a name. Saved queries are
              here, in the side panel, and in ⌘K.
            </p>
          ) : (
            <div className="overflow-hidden rounded-[10px] border">
              {groups.map((g) => (
                <div key={g.folder}>
                  {g.folder && (
                    <div className="border-b bg-header px-3 py-1.5 text-[12px] font-semibold text-fg-2">
                      {g.folder}
                    </div>
                  )}
                  {g.queries.map((q) => (
                    <div
                      key={q.id}
                      className="flex items-center gap-3 border-b border-line-soft px-3 py-2 last:border-b-0"
                    >
                      <div className="flex min-w-0 flex-1 flex-col">
                        <span className="text-[13px]">{q.name}</span>
                        <span className="truncate text-[11.5px] text-muted">
                          {q.description ||
                            q.sql.replace(/\s+/g, " ").slice(0, 120)}
                        </span>
                      </div>
                      <span className="text-[12px] text-fg-2">
                        {nameOf(q.connection_id) ?? (
                          <span className="text-muted">no connection</span>
                        )}
                      </span>
                      <button
                        type="button"
                        className="btn btn-sm"
                        disabled={!q.connection_id}
                        onClick={() => onOpenQuery(q, true)}
                      >
                        Run
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm btn-ghost"
                        onClick={() => onOpenQuery(q, false)}
                      >
                        Open
                      </button>
                      <div className="relative">
                        <button
                          type="button"
                          className="btn btn-sm btn-ghost px-1"
                          aria-label={`More for ${q.name}`}
                          onClick={() => setMenu(menu === q.id ? null : q.id)}
                        >
                          <MoreIcon size={13} />
                        </button>
                        {menu === q.id && (
                          <Popover align="right" onClose={() => setMenu(null)}>
                            <button
                              type="button"
                              role="menuitem"
                              className="menu-item"
                              onClick={() => {
                                setMenu(null);
                                void navigator.clipboard.writeText(
                                  queryMarkdown(q, nameOf(q.connection_id)),
                                );
                                onNotice("Copied as Markdown");
                              }}
                            >
                              Copy as Markdown
                            </button>
                            <button
                              type="button"
                              role="menuitem"
                              className="menu-item"
                              onClick={() => {
                                setMenu(null);
                                void removeQuery(q);
                              }}
                            >
                              Delete…
                            </button>
                          </Popover>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
