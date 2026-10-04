import { useEffect, useMemo, useRef, useState } from "react";
import {
  COPY_LABEL,
  type CopyFormat,
  cellDisplay,
  cellText,
  copyRows,
  formatBytes,
  formatDuration,
  lineAndColumn,
  partialCells,
  planRows,
  rightAligned,
  rowCount,
  sortRows,
} from "../lib/databases";
import type {
  Cell,
  ExportFormat,
  PlanNode,
  ResultColumn,
  StatementRun,
} from "../lib/ipc";
import { usePref } from "../lib/prefs";
import { AlertIcon, ChevronDown, PanelRightIcon } from "./icons";
import Popover from "./Popover";

const ROW_HEIGHT = 26;
const OVERSCAN = 30;
const NUMBER_WIDTH = 48;

type Props = {
  runs: StatementRun[];
  /** The tab's text, for an error's line and column. */
  text: string;
  running: boolean;
  /** When the running statement started, for "Running · 4.2 s". */
  startedAt: number | null;
  timeoutSeconds: number;
  /** A problem before anything ran, such as a connection that failed. */
  problem: string | null;
  onFetchAll: (run: StatementRun) => void;
  onExport: (run: StatementRun, format: ExportFormat) => void;
  onGoToError: (position: number) => void;
  onNotice: (text: string) => void;
};

function summary(run: StatementRun): string {
  const time = formatDuration(run.elapsed_ms);
  switch (run.result.kind) {
    case "rows":
      return `${rowCount(run.result.rows.length)} · ${time}`;
    case "command":
      return `${run.result.tag} · ${time}`;
    case "plan":
      return `Plan · ${time}`;
    case "failed":
      return `Failed after ${time}`;
  }
}

/** Results under the editor: one tab per statement that ran. */
export default function DbResults(props: Props) {
  const { runs, running } = props;
  const [selected, setSelected] = useState(0);
  // Hidden until asked for, then remembered: the grid keeps its width.
  const [inspector, setInspector] = usePref<boolean>(
    "brainiac.databases.inspector",
    false,
  );
  // A new run shows its last result: the first failure, or the last statement.
  useEffect(() => setSelected(Math.max(0, runs.length - 1)), [runs]);
  const run = runs[selected] ?? null;
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (!running || props.startedAt === null) return;
    const started = props.startedAt;
    const t = setInterval(() => setElapsed(Date.now() - started), 200);
    return () => clearInterval(t);
  }, [running, props.startedAt]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="relative flex min-h-[38px] flex-wrap items-center gap-2.5 border-b bg-header px-3 py-1.5">
        {running && <div className="progress-line" aria-hidden="true" />}
        {runs.length > 1 && (
          <div className="seg seg-sm" role="tablist" aria-label="Results">
            {runs.map((r, i) => (
              <button
                // biome-ignore lint/suspicious/noArrayIndexKey: one result per statement, in order.
                key={i}
                type="button"
                role="tab"
                aria-selected={i === selected}
                title={r.sql}
                onClick={() => setSelected(i)}
              >
                {r.result.kind === "failed" ? "⚠ " : ""}
                {i + 1}
              </button>
            ))}
          </div>
        )}
        {running ? (
          <span className="text-[12px] text-fg-2">
            Running · {formatDuration(elapsed)} of {props.timeoutSeconds} s
            limit
            {run && (
              <span className="ml-2 text-muted">
                showing the previous result
              </span>
            )}
          </span>
        ) : run ? (
          <span
            className={`text-[12px] ${run.result.kind === "failed" ? "font-medium text-conflict" : "text-fg-2"}`}
          >
            {summary(run)}
            {run.result.kind !== "failed" && run.ran_read_only && (
              <span className="text-muted"> · read only</span>
            )}
            {run.reconnected && (
              <span
                className="text-muted"
                title="The session was opened again, so settings such as SET were lost."
              >
                {" "}
                · reconnected
              </span>
            )}
          </span>
        ) : null}
        {run?.result.kind === "rows" && run.result.more && !running && (
          <>
            <span className="text-[12px] text-muted">
              first {rowCount(run.result.rows.length)} · more available
            </span>
            {run.ran_read_only && (
              <button
                type="button"
                className="btn btn-sm"
                title="Run it again for up to 100,000 rows"
                onClick={() => props.onFetchAll(run)}
              >
                Fetch All
              </button>
            )}
          </>
        )}
        <span className="flex-1" />
        {run?.result.kind === "rows" && (
          <>
            <CopyMenu
              columns={run.result.columns}
              rows={run.result.rows}
              onNotice={props.onNotice}
            />
            <ExportMenu run={run} onExport={props.onExport} />
            <button
              type="button"
              className="btn btn-sm btn-ghost px-1.5"
              aria-label={inspector ? "Hide inspector" : "Show inspector"}
              aria-pressed={inspector}
              title="Inspector"
              onClick={() => setInspector(!inspector)}
            >
              <PanelRightIcon />
            </button>
          </>
        )}
      </div>
      {props.problem && !running ? (
        <div className="p-4">
          <ErrorBox message={props.problem} />
        </div>
      ) : !run ? (
        <div className="p-6 text-[12.5px] text-muted">
          ⌘⏎ runs the statement under the cursor · ⇧⌘⏎ runs them all · ⌘E
          explains it
        </div>
      ) : run.result.kind === "rows" ? (
        <Grid
          key={`${selected}:${runs.length}`}
          columns={run.result.columns}
          rows={run.result.rows}
          inspector={inspector}
          onNotice={props.onNotice}
        />
      ) : run.result.kind === "command" ? (
        <div className="p-6 text-[13px]">
          <span className="mono font-semibold">{run.result.tag}</span>
          <span className="ml-2 text-muted">
            {run.transaction
              ? "in the open transaction, not yet committed"
              : run.ran_read_only
                ? ""
                : "committed"}
          </span>
        </div>
      ) : run.result.kind === "plan" ? (
        <PlanView
          plan={run.result.plan}
          planning={run.result.planning_ms}
          execution={run.result.execution_ms}
        />
      ) : (
        <Failure
          run={run}
          text={props.text}
          onGoToError={props.onGoToError}
          onNotice={props.onNotice}
        />
      )}
    </div>
  );
}

function Failure({
  run,
  text,
  onGoToError,
  onNotice,
}: {
  run: StatementRun;
  text: string;
  onGoToError: (position: number) => void;
  onNotice: (text: string) => void;
}) {
  if (run.result.kind !== "failed") return null;
  const failure = run.result.failure;
  const position = failure.position;
  const at = position !== null ? lineAndColumn(text, position) : null;
  const meta = [
    failure.code,
    at ? `line ${at.line}, column ${at.column}` : null,
    run.transaction || failure.reason === "connection"
      ? null
      : "nothing was changed",
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <div className="min-h-0 flex-1 overflow-auto p-4">
      <ErrorBox
        message={failure.message}
        hint={failure.hint}
        detail={failure.detail}
        meta={meta}
        onGoTo={position !== null ? () => onGoToError(position) : undefined}
        onCopy={() => {
          void navigator.clipboard.writeText(failure.message);
          onNotice("Copied the message");
        }}
      />
    </div>
  );
}

export function ErrorBox({
  message,
  hint,
  detail,
  meta,
  onGoTo,
  onCopy,
}: {
  message: string;
  hint?: string | null;
  detail?: string | null;
  meta?: string;
  onGoTo?: () => void;
  onCopy?: () => void;
}) {
  return (
    <div role="alert" className="db-error">
      <AlertIcon className="mt-0.5 shrink-0 text-conflict" />
      <div className="flex min-w-0 flex-col gap-1.5">
        <div className="mono selectable text-[12.5px] text-fg">{message}</div>
        {detail && (
          <div className="selectable text-[12.5px] text-fg-2">{detail}</div>
        )}
        {hint && (
          <div className="selectable text-[12.5px] text-fg-2">Hint: {hint}</div>
        )}
        {meta && <div className="text-[11.5px] text-muted">{meta}</div>}
        {(onGoTo || onCopy) && (
          <div className="mt-1 flex gap-2">
            {onGoTo && (
              <button type="button" className="btn btn-sm" onClick={onGoTo}>
                Go to Error
              </button>
            )}
            {onCopy && (
              <button
                type="button"
                className="btn btn-sm btn-ghost"
                onClick={onCopy}
              >
                Copy Message
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function CopyMenu({
  columns,
  rows,
  onNotice,
}: {
  columns: ResultColumn[];
  rows: Cell[][];
  onNotice: (text: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const copy = (format: CopyFormat) => {
    setOpen(false);
    void navigator.clipboard.writeText(copyRows(format, columns, rows));
    const partial = partialCells(rows);
    onNotice(
      partial
        ? `Copied ${rowCount(rows.length)}; ${partial === 1 ? "1 value was" : `${partial.toLocaleString("en-US")} values were`} cut short and copied as shown. Export writes values whole.`
        : `Copied ${rowCount(rows.length)}`,
    );
  };
  return (
    <div className="relative">
      <button
        type="button"
        className="btn btn-sm btn-ghost"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        Copy
        <ChevronDown size={9} />
      </button>
      {open && (
        <Popover align="right" onClose={() => setOpen(false)}>
          {(Object.keys(COPY_LABEL) as CopyFormat[]).map((f) => (
            <button
              key={f}
              type="button"
              role="menuitem"
              className="menu-item"
              onClick={() => copy(f)}
            >
              {COPY_LABEL[f]}
            </button>
          ))}
        </Popover>
      )}
    </div>
  );
}

function ExportMenu({
  run,
  onExport,
}: {
  run: StatementRun;
  onExport: (run: StatementRun, format: ExportFormat) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="relative">
      <button
        type="button"
        className="btn btn-sm btn-ghost"
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={!run.ran_read_only}
        title={
          run.ran_read_only
            ? "Every row, by running the statement again read only"
            : "Export runs the statement again, and this one writes"
        }
        onClick={() => setOpen(!open)}
      >
        Export
        <ChevronDown size={9} />
      </button>
      {open && (
        <Popover align="right" onClose={() => setOpen(false)}>
          {(["csv", "json"] as const).map((f) => (
            <button
              key={f}
              type="button"
              role="menuitem"
              className="menu-item"
              onClick={() => {
                setOpen(false);
                onExport(run, f);
              }}
            >
              Every Row as {f.toUpperCase()}…
            </button>
          ))}
        </Popover>
      )}
    </div>
  );
}

function initialWidth(c: ResultColumn): number {
  // Wide enough for the name and its type side by side in the header.
  const base = c.name.length * 7.5 + c.type_name.length * 6.5 + 36;
  const byKind =
    c.kind === "json" || c.kind === "text"
      ? 200
      : c.kind === "temporal"
        ? 170
        : c.kind === "uuid"
          ? 290
          : 90;
  return Math.min(360, Math.max(base, byKind));
}

/** The grid: virtualized rows, resizable columns, sorting on screen. */
function Grid({
  columns,
  rows,
  inspector,
  onNotice,
}: {
  columns: ResultColumn[];
  rows: Cell[][];
  inspector: boolean;
  onNotice: (text: string) => void;
}) {
  const [widths, setWidths] = useState(() => columns.map(initialWidth));
  const [sort, setSort] = useState<{
    column: number;
    dir: "asc" | "desc";
  } | null>(null);
  const [selected, setSelected] = useState<{
    row: number;
    column: number;
  } | null>(null);
  const [scroll, setScroll] = useState({ top: 0, height: 400 });
  const box = useRef<HTMLDivElement>(null);
  const shown = useMemo(
    () => (sort ? sortRows(rows, sort.column, sort.dir) : rows),
    [rows, sort],
  );
  const first = Math.max(0, Math.floor(scroll.top / ROW_HEIGHT) - OVERSCAN);
  const last = Math.min(
    shown.length,
    Math.ceil((scroll.top + scroll.height) / ROW_HEIGHT) + OVERSCAN,
  );
  const total = NUMBER_WIDTH + widths.reduce((a, b) => a + b, 0);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () =>
      setScroll({ top: el.scrollTop, height: el.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const resize = (index: number, startX: number) => {
    const start = widths[index] ?? 100;
    const move = (e: MouseEvent) => {
      const next = Math.max(48, start + e.clientX - startX);
      setWidths((w) => w.map((x, i) => (i === index ? next : x)));
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  /** Move the selection with the arrows, keeping it on screen. */
  const move = (rowBy: number, columnBy: number) => {
    const at = selected ?? { row: 0, column: 0 };
    const next = {
      row: Math.min(Math.max(0, at.row + rowBy), shown.length - 1),
      column: Math.min(Math.max(0, at.column + columnBy), columns.length - 1),
    };
    setSelected(next);
    const el = box.current;
    if (!el) return;
    const top = next.row * ROW_HEIGHT;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + 2 * ROW_HEIGHT > el.scrollTop + el.clientHeight)
      el.scrollTop = top + 2 * ROW_HEIGHT - el.clientHeight;
  };

  const cell = selected
    ? (shown[selected.row]?.[selected.column] ?? null)
    : null;
  const column = selected ? columns[selected.column] : null;

  return (
    <div className="flex min-h-0 flex-1">
      <div
        ref={box}
        data-own-arrows
        className="min-w-0 flex-1 overflow-auto"
        onScroll={(e) =>
          setScroll({
            top: e.currentTarget.scrollTop,
            height: e.currentTarget.clientHeight,
          })
        }
      >
        <table
          className="rg"
          // biome-ignore lint/a11y/noNoninteractiveElementToInteractiveRole: the result is navigated cell by cell with the arrows, as a grid.
          role="grid"
          aria-label="Result"
          aria-rowcount={shown.length + 1}
          tabIndex={0}
          style={{ width: total }}
          onClick={(e) => {
            const td = (e.target as HTMLElement).closest("td[data-column]");
            if (!(td instanceof HTMLElement)) return;
            setSelected({
              row: Number(td.dataset.row),
              column: Number(td.dataset.column),
            });
          }}
          onKeyDown={(e) => {
            const arrows: Record<string, [number, number]> = {
              ArrowUp: [-1, 0],
              ArrowDown: [1, 0],
              ArrowLeft: [0, -1],
              ArrowRight: [0, 1],
            };
            const step = arrows[e.key];
            if (step && shown.length) {
              e.preventDefault();
              move(step[0], step[1]);
            } else if (e.metaKey && e.key === "c" && selected) {
              e.preventDefault();
              void navigator.clipboard.writeText(cellText(cell));
              onNotice("Copied the value");
            }
          }}
        >
          <colgroup>
            <col style={{ width: NUMBER_WIDTH }} />
            {widths.map((w, i) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: one column per result column.
              <col key={i} style={{ width: w }} />
            ))}
          </colgroup>
          <thead>
            <tr>
              <th className="rg-num">
                {sort && (
                  <button
                    type="button"
                    className="text-[11px] text-link"
                    title="Clear the sort"
                    onClick={() => setSort(null)}
                  >
                    ×
                  </button>
                )}
              </th>
              {columns.map((c, i) => (
                <th
                  // biome-ignore lint/suspicious/noArrayIndexKey: columns can share a name.
                  key={i}
                  aria-sort={
                    sort?.column === i
                      ? sort.dir === "asc"
                        ? "ascending"
                        : "descending"
                      : undefined
                  }
                >
                  <button
                    type="button"
                    className={`rg-sort ${rightAligned(c.kind) ? "justify-end" : ""}`}
                    title={`${c.name} · ${c.type_name || "no declared type"} · click to sort on screen`}
                    onClick={() =>
                      setSort((s) =>
                        s?.column !== i
                          ? { column: i, dir: "asc" }
                          : s.dir === "asc"
                            ? { column: i, dir: "desc" }
                            : null,
                      )
                    }
                  >
                    <span className="truncate">{c.name}</span>
                    <span className="ty">{c.type_name}</span>
                    {sort?.column === i && (
                      <span className="text-[10px] text-link">
                        {sort.dir === "asc" ? "▲" : "▼"}
                      </span>
                    )}
                  </button>
                  {/* biome-ignore lint/a11y/noStaticElementInteractions: a drag handle for the mouse; columns also fit their header. */}
                  <span
                    className="grip"
                    onMouseDown={(e) => {
                      e.preventDefault();
                      resize(i, e.clientX);
                    }}
                  />
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            <tr className="rg-spacer" style={{ height: first * ROW_HEIGHT }} />
            {shown.slice(first, last).map((row, offset) => {
              const r = first + offset;
              return (
                <tr key={r} aria-rowindex={r + 2}>
                  <td className="rg-num">{r + 1}</td>
                  {row.map((value, i) => (
                    <td
                      // biome-ignore lint/suspicious/noArrayIndexKey: cells by column.
                      key={i}
                      data-row={r}
                      data-column={i}
                      data-selected={
                        selected?.row === r && selected.column === i
                          ? "true"
                          : undefined
                      }
                      className={`${value === null ? "null" : ""} ${rightAligned(columns[i]?.kind ?? "text") ? "r" : ""}`}
                    >
                      {cellDisplay(value)}
                    </td>
                  ))}
                </tr>
              );
            })}
            <tr
              className="rg-spacer"
              style={{ height: Math.max(0, shown.length - last) * ROW_HEIGHT }}
            />
          </tbody>
        </table>
      </div>
      {inspector && (
        <aside
          aria-label="Inspector"
          className="flex w-[280px] shrink-0 flex-col border-l bg-panel"
        >
          {selected && column ? (
            <Inspector column={column} value={cell} onNotice={onNotice} />
          ) : (
            <div className="p-4 text-[12px] text-muted">
              Select a cell to see its whole value.
            </div>
          )}
        </aside>
      )}
    </div>
  );
}

function Inspector({
  column,
  value,
  onNotice,
}: {
  column: ResultColumn;
  value: Cell;
  onNotice: (text: string) => void;
}) {
  let body: string = cellText(value);
  let note: string | null = null;
  if (value === null) {
    body = "NULL";
  } else if (typeof value === "object") {
    if (value.kind === "cut")
      note = `Cut at ${formatBytes(value.text.length)} of ${formatBytes(value.length)}. Export writes the whole value.`;
    if (value.kind === "bytes") {
      body = value.hex.replace(/(.{2})/g, "$1 ").trim();
      note = `${formatBytes(value.size)} binary${value.size > value.hex.length / 2 ? ", first 4 KB shown" : ""}.`;
    }
    if (value.kind === "other")
      note = `Brainiac cannot show ${value.type_name} values. Cast the column to ::text to see it.`;
  }
  if (column.kind === "json" && typeof value === "string") {
    try {
      body = JSON.stringify(JSON.parse(value), null, 2);
    } catch {
      // Shown as it is.
    }
  }
  return (
    <>
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <span className="mono truncate text-[12px] font-semibold">
          {column.name}
        </span>
        <span className="mono text-[11px] text-muted">{column.type_name}</span>
        <span className="flex-1" />
        <button
          type="button"
          className="btn btn-sm btn-ghost"
          onClick={() => {
            void navigator.clipboard.writeText(cellText(value));
            onNotice("Copied the value");
          }}
        >
          Copy
        </button>
      </div>
      <pre
        className={`selectable m-0 min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words px-3 py-2 font-[var(--mono)] text-[12px] ${value === null ? "italic text-faint" : ""}`}
      >
        {body}
      </pre>
      {note && (
        <div className="border-t px-3 py-2 text-[11.5px] text-muted">
          {note}
        </div>
      )}
    </>
  );
}

function PlanView({
  plan,
  planning,
  execution,
}: {
  plan: PlanNode;
  planning: number | null;
  execution: number | null;
}) {
  const rows = planRows(plan);
  const number = (n: number | null) =>
    n === null ? "" : n.toLocaleString("en-US", { maximumFractionDigits: 2 });
  return (
    <div className="min-h-0 flex-1 overflow-auto p-3">
      {(planning !== null || execution !== null) && (
        <div className="mb-2 text-[12px] text-fg-2">
          {planning !== null && `Planning ${formatDuration(planning)}`}
          {planning !== null && execution !== null && " · "}
          {execution !== null && `Execution ${formatDuration(execution)}`}
        </div>
      )}
      <table className="db-table">
        <thead>
          <tr>
            <th>Step</th>
            <th className="r">Cost</th>
            <th className="r">Rows (estimate)</th>
            {rows.some((r) => r.node.actual_rows !== null) && (
              <>
                <th className="r">Rows (actual)</th>
                <th className="r">Time</th>
                <th className="r">Loops</th>
              </>
            )}
          </tr>
        </thead>
        <tbody>
          {rows.map(({ node, depth }, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: plan nodes in order.
            <tr key={i}>
              <td style={{ paddingLeft: 10 + depth * 18 }}>
                <div className="mono text-[12px]">
                  {depth > 0 && <span className="text-faint">→ </span>}
                  {node.label}
                </div>
                {node.details.map((d) => (
                  <div key={d} className="mono text-[11.5px] text-muted">
                    {d}
                  </div>
                ))}
              </td>
              <td className="r mono">
                {node.total_cost !== null
                  ? `${number(node.startup_cost)}..${number(node.total_cost)}`
                  : ""}
              </td>
              <td className="r mono">{number(node.rows)}</td>
              {rows.some((r) => r.node.actual_rows !== null) && (
                <>
                  <td className="r mono">{number(node.actual_rows)}</td>
                  <td className="r mono">
                    {node.actual_ms !== null
                      ? `${number(node.actual_ms)} ms`
                      : ""}
                  </td>
                  <td className="r mono">{number(node.loops)}</td>
                </>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
