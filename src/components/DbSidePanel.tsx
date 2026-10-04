import { ask } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { formatDuration, rowCount, selectRows } from "../lib/databases";
import {
  type DbConnection,
  type DbRelation,
  type DbSchema,
  errorMessage,
  type HistoryEntry,
  ipc,
  type SavedQuery,
} from "../lib/ipc";
import { ChevronDown, ChevronRight, RefreshIcon, SearchIcon } from "./icons";

export type SidePanelSection = "schema" | "saved" | "history";

type Props = {
  connection: DbConnection;
  schema: DbSchema | null;
  schemaError: string | null;
  section: SidePanelSection;
  onSection: (section: SidePanelSection) => void;
  onRefreshSchema: () => void;
  queries: SavedQuery[];
  /** Bumped after each run, so History shows it. */
  runTick: number;
  onOpenSql: (sql: string, title?: string) => void;
  onOpenQuery: (query: SavedQuery) => void;
};

/** The side panel of a query tab: its connection's schema, saved queries, and history. */
export default function DbSidePanel(props: Props) {
  const { connection, section } = props;
  return (
    <aside
      aria-label={`Schema, saved queries, and history for ${connection.name}`}
      className="flex w-[264px] shrink-0 flex-col border-l bg-panel"
    >
      <div className="flex items-center gap-2 border-b px-2.5 py-2">
        <div
          className="seg seg-sm flex-1"
          role="tablist"
          aria-label="Side panel"
        >
          {(["schema", "saved", "history"] as const).map((s) => (
            <button
              key={s}
              type="button"
              role="tab"
              className="flex-1"
              aria-selected={section === s}
              onClick={() => props.onSection(s)}
            >
              {s === "schema" ? "Schema" : s === "saved" ? "Saved" : "History"}
            </button>
          ))}
        </div>
      </div>
      {section === "schema" ? (
        <SchemaTree {...props} />
      ) : section === "saved" ? (
        <SavedList {...props} />
      ) : (
        <HistoryList {...props} />
      )}
    </aside>
  );
}

function Filter({
  value,
  onChange,
  label,
}: {
  value: string;
  onChange: (v: string) => void;
  label: string;
}) {
  return (
    <div className="flex items-center gap-1.5 border-b px-2.5 py-1.5 text-muted">
      <SearchIcon size={12} />
      <input
        className="min-w-0 flex-1 bg-transparent text-[12.5px] text-fg outline-none"
        placeholder={label}
        aria-label={label}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}

function SchemaTree({
  connection,
  schema,
  schemaError,
  onRefreshSchema,
  onOpenSql,
}: Props) {
  const [filter, setFilter] = useState("");
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [closedGroups, setClosedGroups] = useState<Set<string>>(new Set());
  const toggle = (set: Set<string>, key: string) => {
    const next = new Set(set);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next;
  };
  const lower = filter.trim().toLowerCase();
  const matches = (r: DbRelation) =>
    !lower ||
    r.name.toLowerCase().includes(lower) ||
    r.columns.some((c) => c.name.toLowerCase().includes(lower));
  return (
    <>
      <div className="flex items-center gap-1 border-b pr-1.5">
        <div className="flex-1">
          <Filter
            value={filter}
            onChange={setFilter}
            label={`Filter ${connection.name}`}
          />
        </div>
        <button
          type="button"
          className="btn btn-sm btn-ghost px-1.5"
          aria-label="Refresh Schema"
          title="Refresh Schema"
          onClick={onRefreshSchema}
        >
          <RefreshIcon size={12} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto py-1 text-[12.5px]">
        {schemaError && (
          <div className="selectable px-3 py-2 text-[12px] text-conflict">
            {schemaError}
          </div>
        )}
        {!schema && !schemaError && (
          <div className="px-3 py-2 text-[12px] text-muted">
            Reading the schema…
          </div>
        )}
        {schema?.schemas.map((group) => {
          const relations = group.relations.filter(matches);
          if (lower && relations.length === 0) return null;
          const groupOpen = !closedGroups.has(group.name);
          const many = schema.schemas.length > 1;
          return (
            <div key={group.name}>
              {many && (
                <button
                  type="button"
                  className="flex w-full items-center gap-1 px-2 py-1 text-left font-semibold text-fg-2"
                  onClick={() =>
                    setClosedGroups(toggle(closedGroups, group.name))
                  }
                >
                  {groupOpen ? (
                    <ChevronDown size={10} />
                  ) : (
                    <ChevronRight size={10} />
                  )}
                  {group.name}
                  <span className="ml-auto text-[11px] font-normal text-muted">
                    {group.relations.length}
                  </span>
                </button>
              )}
              {groupOpen &&
                relations.map((r) => {
                  const key = `${group.name}.${r.name}`;
                  const expanded =
                    open.has(key) ||
                    (!!lower &&
                      r.name.toLowerCase() !== lower &&
                      r.columns.some((c) =>
                        c.name.toLowerCase().includes(lower),
                      ));
                  return (
                    <div key={key}>
                      <button
                        type="button"
                        className="flex w-full items-center gap-1 py-[3px] pr-2 text-left hover:bg-control"
                        style={{ paddingLeft: many ? 18 : 8 }}
                        aria-expanded={expanded}
                        onClick={() => setOpen(toggle(open, key))}
                      >
                        {expanded ? (
                          <ChevronDown size={10} />
                        ) : (
                          <ChevronRight size={10} />
                        )}
                        <span className="mono truncate">{r.name}</span>
                        {r.kind !== "table" && (
                          <span className="text-[10.5px] text-muted">
                            {r.kind === "view"
                              ? "view"
                              : r.kind === "materialized_view"
                                ? "mat. view"
                                : "foreign"}
                          </span>
                        )}
                        <span className="ml-auto text-[11px] text-muted">
                          {r.estimated_rows !== null && r.estimated_rows >= 0
                            ? `~${compact(r.estimated_rows)}`
                            : ""}
                        </span>
                      </button>
                      {expanded && (
                        <div
                          className="pb-1"
                          style={{ paddingLeft: many ? 34 : 24 }}
                        >
                          {r.columns.map((c) => (
                            <div
                              key={c.name}
                              className="flex items-baseline gap-2 py-[1px] pr-2"
                              title={`${c.name} ${c.type_name}${c.nullable ? "" : " not null"}${c.default ? ` default ${c.default}` : ""}`}
                            >
                              <span
                                className={`mono truncate ${c.primary_key ? "font-semibold" : ""}`}
                              >
                                {c.name}
                              </span>
                              <span className="mono ml-auto shrink-0 text-[11px] text-muted">
                                {c.type_name}
                                {c.nullable ? "" : " ·nn"}
                              </span>
                            </div>
                          ))}
                          {r.indexes.length > 0 && (
                            <div className="mt-1 text-[11px] text-muted">
                              {r.indexes.map((i) => (
                                <div
                                  key={i.name}
                                  className="mono truncate"
                                  title={i.definition}
                                >
                                  {i.primary
                                    ? "pk "
                                    : i.unique
                                      ? "unique "
                                      : "index "}
                                  {i.name}
                                </div>
                              ))}
                            </div>
                          )}
                          {r.foreign_keys.length > 0 && (
                            <div className="mt-1 text-[11px] text-muted">
                              {r.foreign_keys.map((k) => (
                                <div
                                  key={k.name}
                                  className="mono truncate"
                                  title={k.definition}
                                >
                                  {k.definition}
                                </div>
                              ))}
                            </div>
                          )}
                          <button
                            type="button"
                            className="btn btn-sm mt-1.5"
                            onClick={() =>
                              onOpenSql(
                                selectRows(
                                  group.name,
                                  r.name,
                                  group.name === schema.default_schema,
                                ),
                                r.name,
                              )
                            }
                          >
                            Select Rows
                          </button>
                        </div>
                      )}
                    </div>
                  );
                })}
            </div>
          );
        })}
        {schema?.schemas.every((g) => g.relations.length === 0) && (
          <div className="px-3 py-2 text-[12px] text-muted">No tables yet.</div>
        )}
      </div>
    </>
  );
}

function compact(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${Math.round(n / 100) / 10}k`.replace(".0k", "k");
  return `${Math.round(n / 100_000) / 10}M`.replace(".0M", "M");
}

function SavedList({ connection, queries, onOpenQuery }: Props) {
  const [filter, setFilter] = useState("");
  const lower = filter.trim().toLowerCase();
  const mine = queries.filter(
    (q) =>
      q.connection_id === connection.id &&
      (!lower ||
        q.name.toLowerCase().includes(lower) ||
        q.folder.toLowerCase().includes(lower) ||
        q.sql.toLowerCase().includes(lower)),
  );
  return (
    <>
      <Filter
        value={filter}
        onChange={setFilter}
        label="Filter saved queries"
      />
      <div className="min-h-0 flex-1 overflow-y-auto py-1">
        {mine.length === 0 && (
          <div className="px-3 py-2 text-[12px] text-muted">
            {lower
              ? "No saved query matches."
              : `No saved queries run on ${connection.name}. ⌘S saves the tab's text.`}
          </div>
        )}
        {mine.map((q) => (
          <button
            key={q.id}
            type="button"
            className="flex w-full flex-col items-start gap-0.5 px-3 py-1.5 text-left hover:bg-control"
            title={q.sql}
            onClick={() => onOpenQuery(q)}
          >
            <span className="text-[12.5px]">
              {q.folder && <span className="text-muted">{q.folder}/</span>}
              {q.name}
            </span>
            {q.description && (
              <span className="truncate text-[11.5px] text-muted">
                {q.description}
              </span>
            )}
          </button>
        ))}
      </div>
    </>
  );
}

function HistoryList({ connection, runTick, onOpenSql }: Props) {
  const [search, setSearch] = useState("");
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => {
    try {
      setEntries(await ipc.queryHistory(connection.id, search, 0, 200));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [connection.id, search]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: a run adds to the history.
  useEffect(() => {
    const t = setTimeout(() => void load(), 120);
    return () => clearTimeout(t);
  }, [load, runTick]);
  return (
    <>
      <Filter value={search} onChange={setSearch} label="Search the history" />
      <div className="min-h-0 flex-1 overflow-y-auto py-1">
        {error && (
          <div className="px-3 py-2 text-[12px] text-conflict">{error}</div>
        )}
        {entries.length === 0 && !error && (
          <div className="px-3 py-2 text-[12px] text-muted">
            {search ? "Nothing ran that matches." : "Nothing ran here yet."}
          </div>
        )}
        {entries.map((e) => (
          <button
            key={e.id}
            type="button"
            className="flex w-full flex-col items-start gap-0.5 px-3 py-1.5 text-left hover:bg-control"
            title="Open in a new tab"
            onClick={() => onOpenSql(e.sql)}
          >
            <span className="mono line-clamp-2 text-[11.5px]">{e.sql}</span>
            <span
              className={`text-[11px] ${e.error ? "text-conflict" : "text-muted"}`}
            >
              {new Date(e.ran_at).toLocaleString(undefined, {
                month: "short",
                day: "numeric",
                hour: "2-digit",
                minute: "2-digit",
              })}{" "}
              · {formatDuration(e.elapsed_ms)}
              {e.error
                ? " · failed"
                : e.rows !== null
                  ? ` · ${rowCount(e.rows)}`
                  : ""}
            </span>
          </button>
        ))}
      </div>
      {entries.length > 0 && (
        <div className="border-t px-3 py-1.5">
          <button
            type="button"
            className="btn btn-sm btn-ghost"
            onClick={async () => {
              const sure = await ask(
                `Every statement run on ${connection.name} is removed from the history.`,
                { title: "Clear History", kind: "warning", okLabel: "Clear" },
              );
              if (!sure) return;
              await ipc.clearQueryHistory(connection.id).catch(() => {});
              void load();
            }}
          >
            Clear History of {connection.name}
          </button>
        </div>
      )}
    </>
  );
}
