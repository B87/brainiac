/**
 * Databases (SPEC.md, section 11): what the query view shows and copies,
 * kept out of the components so it can be tested without a window.
 */
import type {
  Cell,
  ColumnKind,
  DbConnection,
  DbEnvironment,
  DbSchema,
  PlanNode,
  QueryTab,
  ResultColumn,
  RunMode,
  SavedQuery,
} from "./ipc";

export const ENVIRONMENTS: DbEnvironment[] = [
  "local",
  "development",
  "staging",
  "production",
];

export const ENV_LABEL: Record<DbEnvironment, string> = {
  local: "Local",
  development: "Development",
  staging: "Staging",
  production: "Production",
};

/** Short, uppercase, for the label beside a connection's name. */
export const ENV_SHORT: Record<DbEnvironment, string> = {
  local: "LOCAL",
  development: "DEV",
  staging: "STAGING",
  production: "PRODUCTION",
};

/** Where a connection points, in a few words: `user@host:port/db`, or the file. */
export function connectionPlace(c: DbConnection): string {
  if (c.kind === "sqlite") return c.file_path ?? "";
  const port = c.port && c.port !== 5432 ? `:${c.port}` : "";
  const user = c.user ? `${c.user}@` : "";
  return `${user}${c.host ?? ""}${port}/${c.database ?? ""}`;
}

export function kindLabel(c: Pick<DbConnection, "kind">): string {
  return c.kind === "sqlite" ? "SQLite" : "PostgreSQL";
}

/** The modes a tab on this connection may use, in the order offered. */
export function modesFor(c: DbConnection): RunMode[] {
  return c.access === "read_write"
    ? ["read_only", "auto_commit", "manual"]
    : ["read_only"];
}

/**
 * A new tab's mode: read only on a read-only connection and on production,
 * where read and write is turned on per tab; auto-commit elsewhere.
 */
export function defaultMode(c: DbConnection | null): RunMode {
  if (c?.access !== "read_write" || c.environment === "production")
    return "read_only";
  return "auto_commit";
}

/** Turning on read and write in a tab: Manual on production, else auto-commit. */
export function writableMode(c: DbConnection): RunMode {
  return c.environment === "production" ? "manual" : "auto_commit";
}

export const MODE_LABEL: Record<RunMode, string> = {
  read_only: "Read only",
  auto_commit: "Auto-commit",
  manual: "Manual",
};

/** A cell as text, as it is copied: `NULL` is empty here; the grid draws it apart. */
export function cellText(cell: Cell): string {
  if (cell === null) return "";
  if (typeof cell === "boolean") return cell ? "true" : "false";
  if (typeof cell === "number") return String(cell);
  if (typeof cell === "string") return cell;
  switch (cell.kind) {
    case "cut":
      return cell.text;
    case "bytes":
      return `\\x${cell.hex}`;
    case "other":
      return `<${cell.type_name}>`;
  }
}

/** What the grid shows in a cell; long values are cut by CSS. */
export function cellDisplay(cell: Cell): string {
  if (cell === null) return "NULL";
  if (typeof cell === "object" && cell.kind === "bytes")
    return `${formatBytes(cell.size)} binary`;
  if (typeof cell === "object" && cell.kind === "other")
    return `${cell.type_name} · cast to ::text`;
  const text = cellText(cell);
  return text.length > 500 ? `${text.slice(0, 500)}…` : text;
}

/**
 * Values the result holds only in part (long text cut, binary shown in
 * part, a type shown by name): copying copies them as shown.
 */
export function partialCells(rows: Cell[][]): number {
  let count = 0;
  for (const row of rows)
    for (const cell of row) {
      if (cell === null || typeof cell !== "object") continue;
      if (
        cell.kind === "cut" ||
        cell.kind === "other" ||
        (cell.kind === "bytes" && cell.hex.length / 2 < cell.size)
      )
        count += 1;
    }
  return count;
}

export function rightAligned(kind: ColumnKind): boolean {
  return kind === "number" || kind === "numeric";
}

/** Rows sorted on screen by one column; NULLs last, numbers as numbers. */
export function sortRows(
  rows: Cell[][],
  column: number,
  direction: "asc" | "desc",
): Cell[][] {
  const sign = direction === "asc" ? 1 : -1;
  const numeric = (c: Cell): number | null => {
    if (typeof c === "number") return c;
    if (typeof c === "string" && c.trim() !== "" && !Number.isNaN(Number(c)))
      return Number(c);
    return null;
  };
  return rows
    .map((row, index) => ({ row, index }))
    .sort((a, b) => {
      const x = a.row[column] ?? null;
      const y = b.row[column] ?? null;
      if (x === null && y === null) return a.index - b.index;
      if (x === null) return 1;
      if (y === null) return -1;
      const nx = numeric(x);
      const ny = numeric(y);
      const order =
        nx !== null && ny !== null
          ? nx - ny
          : cellText(x).localeCompare(cellText(y), undefined, {
              numeric: true,
            });
      return order === 0 ? a.index - b.index : order * sign;
    })
    .map((r) => r.row);
}

export type CopyFormat = "tsv" | "csv" | "json" | "markdown" | "insert";

export const COPY_LABEL: Record<CopyFormat, string> = {
  tsv: "Tab-separated (for a spreadsheet)",
  csv: "CSV",
  json: "JSON",
  markdown: "Markdown table (for a note)",
  insert: "SQL INSERT statements",
};

function csvField(text: string): string {
  return /[",\n\r]/.test(text) || /^\s|\s$/.test(text)
    ? `"${text.replace(/"/g, '""')}"`
    : text;
}

function jsonValue(cell: Cell, kind: ColumnKind): unknown {
  if (cell === null || typeof cell === "boolean" || typeof cell === "number")
    return cell;
  const text = cellText(cell);
  if (kind === "json") {
    try {
      return JSON.parse(text);
    } catch {
      return text;
    }
  }
  return text;
}

function sqlLiteral(cell: Cell, kind: ColumnKind): string {
  if (cell === null) return "NULL";
  if (typeof cell === "boolean") return cell ? "TRUE" : "FALSE";
  if (typeof cell === "number") return String(cell);
  const text = cellText(cell);
  if ((kind === "number" || kind === "numeric") && /^-?[\d.]+$/.test(text))
    return text;
  return `'${text.replace(/'/g, "''")}'`;
}

function identifier(name: string): string {
  return /^[a-z_][a-z0-9_]*$/.test(name)
    ? name
    : `"${name.replace(/"/g, '""')}"`;
}

/** Rows as text to paste elsewhere. `table` names the table for INSERTs. */
export function copyRows(
  format: CopyFormat,
  columns: ResultColumn[],
  rows: Cell[][],
  table = "table_name",
): string {
  const names = columns.map((c) => c.name);
  switch (format) {
    case "tsv":
      return [names, ...rows.map((r) => r.map(cellText))]
        .map((r) => r.map((t) => t.replace(/[\t\n\r]/g, " ")).join("\t"))
        .join("\n");
    case "csv":
      return [names, ...rows.map((r) => r.map(cellText))]
        .map((r) => r.map(csvField).join(","))
        .join("\n");
    case "json":
      return JSON.stringify(
        rows.map((r) =>
          Object.fromEntries(
            columns.map((c, i) => [c.name, jsonValue(r[i] ?? null, c.kind)]),
          ),
        ),
        null,
        2,
      );
    case "markdown": {
      const cellOf = (t: string) =>
        t.replace(/\|/g, "\\|").replace(/[\n\r]+/g, " ");
      const head = `| ${names.map(cellOf).join(" | ")} |`;
      const rule = `| ${columns.map((c) => (rightAligned(c.kind) ? "---:" : "---")).join(" | ")} |`;
      const body = rows.map(
        (r) =>
          `| ${r.map((c) => (c === null ? "" : cellOf(cellText(c)))).join(" | ")} |`,
      );
      return [head, rule, ...body].join("\n");
    }
    case "insert": {
      const target = identifier(table);
      const list = names.map(identifier).join(", ");
      return rows
        .map(
          (r) =>
            `INSERT INTO ${target} (${list}) VALUES (${r.map((c, i) => sqlLiteral(c, columns[i]?.kind ?? "text")).join(", ")});`,
        )
        .join("\n");
    }
  }
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${Math.round(n)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = n / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  if (minutes < 60)
    return seconds ? `${minutes} min ${seconds} s` : `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  return `${hours} h ${minutes % 60} min`;
}

/** Rows in words, with thousands separated: "1,000 rows". */
export function rowCount(n: number): string {
  return `${n.toLocaleString("en-US")} ${n === 1 ? "row" : "rows"}`;
}

/** `Untitled n` with the lowest n not already used by a tab. */
export function untitled(tabs: Pick<QueryTab, "title">[]): string {
  const used = new Set(tabs.map((t) => t.title));
  let n = 1;
  while (used.has(`Untitled ${n}`)) n += 1;
  return `Untitled ${n}`;
}

/** Saved queries grouped by folder, top level first, then folders A to Z. */
export function byFolder(
  queries: SavedQuery[],
): { folder: string; queries: SavedQuery[] }[] {
  const groups = new Map<string, SavedQuery[]>();
  for (const q of queries) {
    const list = groups.get(q.folder) ?? [];
    list.push(q);
    groups.set(q.folder, list);
  }
  return [...groups.entries()]
    .sort(([a], [b]) =>
      a === ""
        ? -1
        : b === ""
          ? 1
          : a.localeCompare(b, undefined, { sensitivity: "base" }),
    )
    .map(([folder, list]) => ({
      folder,
      queries: list.sort((a, b) =>
        a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
      ),
    }));
}

/** Copy as Markdown: a saved query for a note. */
export function queryMarkdown(
  q: SavedQuery,
  connection: string | null,
): string {
  const lines = [`**${q.name}**${connection ? ` · ${connection}` : ""}`];
  if (q.description) lines.push("", q.description);
  lines.push("", "```sql", q.sql.trimEnd(), "```");
  return lines.join("\n");
}

/** Whether `query` matches a saved query's name, folder, description, or SQL. */
export function queryMatches(q: SavedQuery, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  return [q.name, q.folder, q.description, q.sql].some((t) =>
    t.toLowerCase().includes(needle),
  );
}

/** A completion option, shaped as CodeMirror's `Completion`. */
export type CompletionOption = {
  label: string;
  type: string;
  detail?: string;
  apply?: string;
};

/** Completion data, shaped as `@codemirror/lang-sql`'s `SQLNamespace`. */
export type CompletionNamespace = Record<
  string,
  { self: CompletionOption; children: CompletionOption[] | CompletionNamespace }
>;

function option(label: string, type: string, detail?: string) {
  const quoted = identifier(label);
  return {
    label,
    type,
    ...(detail ? { detail } : {}),
    ...(quoted !== label ? { apply: quoted } : {}),
  };
}

/**
 * Completion data for CodeMirror's SQL language: schemas, then tables
 * (qualified and, for the default schema, bare) with their columns, each
 * labelled with what it is.
 */
export function completionSchema(schema: DbSchema | null): CompletionNamespace {
  const out: CompletionNamespace = {};
  // Schemas first, so the qualified names below find them already labelled.
  for (const group of schema?.schemas ?? [])
    out[escapeDots(group.name)] = {
      self: option(group.name, "schema"),
      children: {},
    };
  for (const group of schema?.schemas ?? []) {
    for (const relation of group.relations) {
      const entry = {
        self: option(
          relation.name,
          relation.kind === "view" || relation.kind === "materialized_view"
            ? "view"
            : "table",
        ),
        children: relation.columns.map((c) =>
          option(c.name, "column", c.type_name),
        ),
      };
      out[`${escapeDots(group.name)}.${escapeDots(relation.name)}`] = entry;
      if (group.name === schema?.default_schema)
        out[escapeDots(relation.name)] = entry;
    }
  }
  return out;
}

/** lang-sql splits names on dots; a dot inside a name is escaped. */
function escapeDots(name: string): string {
  return name.replace(/\./g, "\\.");
}

/** `select * from <table> limit 100` with the name quoted when it needs to be. */
export function selectRows(
  schemaName: string,
  table: string,
  isDefault: boolean,
): string {
  const name = isDefault
    ? identifier(table)
    : `${identifier(schemaName)}.${identifier(table)}`;
  return `select * from ${name} limit 100;`;
}

/** The nodes of a plan, depth first, with their depth, for drawing indented. */
export function planRows(
  node: PlanNode,
  depth = 0,
): { node: PlanNode; depth: number }[] {
  return [
    { node, depth },
    ...node.children.flatMap((c) => planRows(c, depth + 1)),
  ];
}

/** The line and column (from 1) of a UTF-16 offset in the text. */
export function lineAndColumn(
  text: string,
  offset: number,
): { line: number; column: number } {
  const before = text.slice(0, offset);
  const line = before.split("\n").length;
  const column = offset - before.lastIndexOf("\n");
  return { line, column };
}
