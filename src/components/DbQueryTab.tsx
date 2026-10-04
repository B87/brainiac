import { EditorView } from "@codemirror/view";
import { ask, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  completionSchema,
  connectionPlace,
  defaultMode,
  ENV_SHORT,
  kindLabel,
  MODE_LABEL,
  modesFor,
} from "../lib/databases";
import {
  createSqlState,
  markError,
  type SqlEditorOptions,
  selectionOf,
  setSqlLanguage,
} from "../lib/editor/sql";
import {
  type DbConnection,
  type DbSchema,
  type ExplainMode,
  type ExportFormat,
  errorMessage,
  ipc,
  isAppError,
  type ParamValue,
  type QueryTab,
  type RunMode,
  type RunStatementRequest,
  type SavedQuery,
  type StatementRun,
  type TransactionInfo,
} from "../lib/ipc";
import { useKeys } from "../lib/keys";
import { PasswordDialog } from "./DbDialogs";
import DbResults from "./DbResults";
import DbSidePanel, { type SidePanelSection } from "./DbSidePanel";
import {
  ChevronDown,
  LockIcon,
  PanelRightIcon,
  PlayIcon,
  PlusIcon,
  SearchIcon,
  StopIcon,
} from "./icons";
import Popover from "./Popover";

export type SchemaState = { schema: DbSchema | null; error: string | null };

type Props = {
  tab: QueryTab;
  visible: boolean;
  connections: DbConnection[];
  queries: SavedQuery[];
  schema: SchemaState | undefined;
  /** Read (or read again) a connection's schema. */
  onSchema: (connectionId: string, refresh: boolean) => void;
  onChange: (patch: Partial<QueryTab>) => void;
  onConnectionChanged: (connection: DbConnection) => void;
  onOpenSql: (sql: string, title?: string) => void;
  onOpenQuery: (query: SavedQuery) => void;
  onSave: () => void;
  onNewConnection: () => void;
  onTransaction: (info: TransactionInfo | null) => void;
  onNotice: (text: string) => void;
  /** Run as soon as the tab opens: Run from Home or ⌘K. */
  autoRun: boolean;
  onAutoRan: () => void;
};

type RunOptions = {
  all?: boolean;
  explain?: ExplainMode;
  fetchAll?: StatementRun;
  /** The password was just entered: run without checking for it. */
  unlocked?: boolean;
};

/** Values kept per tab for `:name` parameters while Brainiac runs. */
const remembered = new Map<string, ParamValue[]>();

export default function DbQueryTab(props: Props) {
  const { tab, connections } = props;
  const connection =
    connections.find((c) => c.id === tab.connection_id) ?? null;
  const saved = props.queries.find((q) => q.id === tab.saved_query_id) ?? null;
  const [runs, setRuns] = useState<StatementRun[]>([]);
  const [running, setRunning] = useState(false);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [transaction, setTransaction] = useState<TransactionInfo | null>(null);
  const [lastRunAt, setLastRunAt] = useState(Date.now());
  const [params, setParams] = useState<ParamValue[]>(
    () => remembered.get(tab.id) ?? saved?.parameters ?? [],
  );
  const [paramNames, setParamNames] = useState<string[]>([]);
  const [panelOpen, setPanelOpen] = useState(true);
  const [section, setSection] = useState<SidePanelSection>("schema");
  const [switcher, setSwitcher] = useState(false);
  const [explainMenu, setExplainMenu] = useState(false);
  const [askPassword, setAskPassword] = useState<RunOptions | null>(null);
  const [runTick, setRunTick] = useState(0);
  const [, setNow] = useState(0);
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const paramForm = useRef<HTMLFormElement>(null);

  // The latest values for the editor's key handlers, created once.
  const latest = useRef<{ run: (o: RunOptions) => void; cancel: () => void }>({
    run: () => {},
    cancel: () => {},
  });

  const editorOptions = useCallback(
    (): SqlEditorOptions => ({
      dialect: connection?.kind ?? "postgres",
      schema: completionSchema(props.schema?.schema ?? null),
      defaultSchema: props.schema?.schema?.default_schema ?? undefined,
      onChange: (text) =>
        props.onChange({
          text,
          dirty: saved ? text !== saved.sql : text.trim() !== "",
        }),
      onRun: () => latest.current.run({}),
      onRunAll: () => latest.current.run({ all: true }),
      onCancel: () => latest.current.cancel(),
      onExplain: () => latest.current.run({ explain: "plan" }),
    }),
    [connection?.kind, props.schema, saved, props.onChange],
  );
  const optionsRef = useRef(editorOptions);
  optionsRef.current = editorOptions;

  // biome-ignore lint/correctness/useExhaustiveDependencies: the editor is created once; options change below.
  useEffect(() => {
    if (!host.current) return;
    const created = new EditorView({
      parent: host.current,
      state: createSqlState(tab.text, {
        ...editorOptions(),
        // Always the latest handlers, whatever the options were at creation.
        onChange: (text) => optionsRef.current().onChange?.(text),
      }),
    });
    view.current = created;
    return () => {
      created.destroy();
      view.current = null;
    };
  }, []);

  // A new dialect or schema reconfigures the language, keeping text and undo.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only these change the language.
  useEffect(() => {
    if (view.current) setSqlLanguage(view.current, editorOptions());
  }, [connection?.kind, props.schema?.schema]);

  // Text set from outside (opening a saved query in this tab).
  useEffect(() => {
    const v = view.current;
    if (v && v.state.doc.toString() !== tab.text) {
      v.dispatch({
        changes: { from: 0, to: v.state.doc.length, insert: tab.text },
      });
    }
  }, [tab.text]);

  // The schema is read when the tab's connection is first used.
  // biome-ignore lint/correctness/useExhaustiveDependencies: once per connection.
  useEffect(() => {
    if (connection?.password_ready && !props.schema)
      props.onSchema(connection.id, false);
  }, [connection?.id, connection?.password_ready]);

  useEffect(() => {
    if (props.visible) view.current?.focus();
  }, [props.visible]);

  // The transaction header turns amber after 5 idle minutes.
  useEffect(() => {
    if (!transaction) return;
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, [transaction]);

  useEffect(() => {
    remembered.set(tab.id, params);
  }, [tab.id, params]);

  const setTx = (info: TransactionInfo | null) => {
    setTransaction(info);
    props.onTransaction(info);
  };

  const run = async (options: RunOptions) => {
    if (!connection) {
      setSwitcher(true);
      return;
    }
    if (running || !view.current) return;
    if (
      connection.password === "ask" &&
      !connection.password_ready &&
      !options.unlocked
    ) {
      setAskPassword(options);
      return;
    }
    const v = view.current;
    const selection = selectionOf(v);
    let request: RunStatementRequest = {
      tab_id: tab.id,
      connection_id: connection.id,
      text: v.state.doc.toString(),
      from: selection.from,
      to: selection.to,
      all: !!options.all,
      fetch_all: false,
      mode: tab.mode,
      parameters: [],
      explain: options.explain ?? null,
    };
    if (options.fetchAll) {
      request = {
        ...request,
        text: options.fetchAll.sql,
        from: 0,
        to: 0,
        all: true,
        fetch_all: true,
      };
    }
    let names: string[] = [];
    try {
      names = await ipc.statementParameters(request);
    } catch (e) {
      setProblem(errorMessage(e));
      return;
    }
    setParamNames(names);
    if (names.length) {
      const missing = names.filter((n) => !params.some((p) => p.name === n));
      if (missing.length) {
        // Ask for the values in the form above the editor; its Run runs again.
        setParams((ps) => [
          ...ps,
          ...missing.map((name) => ({ name, value: "" })),
        ]);
        setTimeout(() => {
          paramForm.current
            ?.querySelector<HTMLInputElement>(
              `input[data-param="${missing[0]}"]`,
            )
            ?.focus();
        }, 0);
        pendingRun.current = options;
        return;
      }
      request.parameters = names.map(
        (n) => params.find((p) => p.name === n) ?? { name: n, value: null },
      );
    }
    if (options.explain === "analyze" && tab.mode !== "read_only") {
      const sure = await ask(
        "Explain Analyze runs the statement to measure it. In this tab it may change data.",
        { title: "Explain Analyze", kind: "warning", okLabel: "Run It" },
      );
      if (!sure) return;
    }
    setRunning(true);
    setStartedAt(Date.now());
    setProblem(null);
    markError(v, null);
    try {
      const result = await ipc.runStatement(request);
      setRuns(result);
      const last = result[result.length - 1];
      // The last statement's report is the transaction's state, open or not:
      // a typed COMMIT or a lost connection ends it.
      if (last) setTx(last.transaction);
      setLastRunAt(Date.now());
      if (
        last?.result.kind === "failed" &&
        last.result.failure.position !== null &&
        !options.fetchAll
      )
        markError(v, last.result.failure.position);
      if (saved && names.length)
        void ipc
          .rememberQueryParameters(saved.id, request.parameters)
          .catch(() => {});
      setRunTick((t) => t + 1);
    } catch (e) {
      if (
        isAppError(e) &&
        e.code === "PERMISSION_DENIED" &&
        connection.password === "ask"
      ) {
        props.onConnectionChanged({ ...connection, password_ready: false });
        setAskPassword({ ...options, unlocked: false });
      }
      setProblem(errorMessage(e));
    } finally {
      setRunning(false);
    }
  };
  const pendingRun = useRef<RunOptions | null>(null);

  const cancel = () => {
    if (running) void ipc.cancelStatement(tab.id);
  };
  latest.current = { run: (o) => void run(o), cancel };

  useKeys(
    props.visible
      ? {
          "mod+.": () => cancel(),
        }
      : {},
  );

  // Run from Home or ⌘K.
  // biome-ignore lint/correctness/useExhaustiveDependencies: once, when asked.
  useEffect(() => {
    if (props.autoRun && view.current) {
      props.onAutoRan();
      void run({ all: true });
    }
  }, [props.autoRun]);

  const endTransaction = async (commit: boolean) => {
    try {
      await ipc.endTransaction(tab.id, commit);
      setTx(null);
      props.onNotice(commit ? "Committed" : "Rolled back");
      setRunTick((t) => t + 1);
    } catch (e) {
      setProblem(errorMessage(e));
      // A COMMIT that fails can still end the transaction: ask what is open.
      const open = await ipc.openDbTransactions().catch(() => null);
      if (open && !open.includes(tab.id)) setTx(null);
    }
  };

  // An edited connection: a mode it no longer allows goes back to its
  // default, and becoming Production turns writes off until asked again.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only when the connection's access or environment changes.
  useEffect(() => {
    if (!connection || transaction) return;
    if (
      !modesFor(connection).includes(tab.mode) ||
      (connection.environment === "production" && tab.mode !== "read_only")
    )
      props.onChange({ mode: defaultMode(connection) });
  }, [connection?.access, connection?.environment]);

  const setMode = async (mode: RunMode) => {
    if (!connection || mode === tab.mode) return;
    if (tab.mode === "read_only" && connection.environment === "production") {
      const sure = await ask(
        `Statements in this tab will be able to change data on ${connection.name}, which is Production. Read and write lasts until the tab closes.`,
        { title: "Turn On Writes", kind: "warning", okLabel: "Turn On" },
      );
      if (!sure) return;
    }
    props.onChange({ mode });
  };

  const switchTo = async (next: DbConnection) => {
    setSwitcher(false);
    if (next.id === tab.connection_id) return;
    if (transaction) {
      const sure = await ask(
        "This tab has a transaction open. Switching connections rolls it back.",
        {
          title: "Roll Back and Switch",
          kind: "warning",
          okLabel: "Roll Back",
        },
      );
      if (!sure) return;
      await ipc.endTransaction(tab.id, false).catch(() => {});
      setTx(null);
    }
    await ipc.closeDbSession(tab.id).catch(() => {});
    props.onChange({
      connection_id: next.id,
      mode: defaultMode(next),
    });
    setRuns([]);
    setProblem(null);
  };

  const exportRun = async (r: StatementRun, format: ExportFormat) => {
    if (!connection) return;
    const path = await saveDialog({
      title: "Export",
      defaultPath: `${tab.title.replace(/[/:]/g, "-")}.${format}`,
      filters: [{ name: format.toUpperCase(), extensions: [format] }],
    });
    if (!path) return;
    try {
      const result = await ipc.exportResult({
        tab_id: tab.id,
        connection_id: connection.id,
        sql: r.sql,
        parameters: params,
        format,
        path,
      });
      props.onNotice(`Exported ${result.rows.toLocaleString("en-US")} rows`);
    } catch (e) {
      setProblem(errorMessage(e));
    }
  };

  const writable = tab.mode !== "read_only";
  const idleMinutes = transaction ? (Date.now() - lastRunAt) / 60_000 : 0;

  return (
    <div
      className="flex min-h-0 flex-1"
      style={{ display: props.visible ? "flex" : "none" }}
    >
      <div className="relative flex min-w-0 flex-1 flex-col">
        <div className="flex border-b" data-env={connection?.environment}>
          <div
            className="env-strip"
            data-writable={writable && connection?.environment === "production"}
            title={connection ? ENV_SHORT[connection.environment] : undefined}
          />
          {/* One line: as it narrows, the hints go, then the name truncates. */}
          <div className="@container flex min-w-0 flex-1 items-center gap-2 px-3 py-[7px]">
            <div className="relative min-w-0">
              <button
                type="button"
                className="btn max-w-full gap-[7px] text-fg"
                disabled={running}
                aria-haspopup="listbox"
                aria-expanded={switcher}
                title="The tab's connection"
                onClick={() => setSwitcher(!switcher)}
              >
                {connection ? (
                  <>
                    <span
                      className="env-dot"
                      data-env={connection.environment}
                    />
                    <span className="truncate font-medium">
                      {connection.name}
                    </span>
                    <span
                      className="env-badge"
                      data-env={connection.environment}
                    >
                      {ENV_SHORT[connection.environment]}
                    </span>
                    {!writable && (
                      <span className="inline-flex shrink-0 items-center gap-1 text-fg-2">
                        <LockIcon size={11} />
                        <span className="@max-3xl:sr-only">Read only</span>
                      </span>
                    )}
                  </>
                ) : (
                  <span className="truncate text-muted">
                    Choose a connection
                  </span>
                )}
                <span className="shrink-0">
                  <ChevronDown size={10} />
                </span>
              </button>
              {switcher && (
                <Switcher
                  connections={connections}
                  current={connection}
                  onPick={(c) => void switchTo(c)}
                  onNew={() => {
                    setSwitcher(false);
                    props.onNewConnection();
                  }}
                  onClose={() => setSwitcher(false)}
                />
              )}
            </div>
            {connection && connection.access === "read_write" && (
              <div
                className="seg seg-sm"
                role="radiogroup"
                aria-label="How statements run"
              >
                {modesFor(connection).map((m) => (
                  <button
                    key={m}
                    type="button"
                    aria-pressed={tab.mode === m}
                    disabled={running || (!!transaction && m !== "manual")}
                    title={
                      m === "manual"
                        ? "The first change opens a transaction; Commit or Roll Back ends it"
                        : m === "auto_commit"
                          ? "Each statement commits"
                          : "Statements cannot change data"
                    }
                    onClick={() =>
                      void setMode(m === "read_only" ? "read_only" : m)
                    }
                  >
                    {MODE_LABEL[m]}
                  </button>
                ))}
              </div>
            )}
            <div className="h-[18px] w-px bg-line" />
            <button
              type="button"
              className="btn btn-primary"
              disabled={running}
              onClick={() => void run({})}
            >
              <PlayIcon />
              Run
              <span className="text-[11px] opacity-80 @max-4xl:hidden">⌘⏎</span>
            </button>
            <button
              type="button"
              className="btn"
              disabled={running}
              title="Run every statement, stopping at the first failure (⇧⌘⏎)"
              onClick={() => void run({ all: true })}
            >
              Run All
            </button>
            <button
              type="button"
              className="btn"
              disabled={!running}
              style={
                running
                  ? {
                      color: "var(--conflict)",
                      borderColor:
                        "color-mix(in srgb, var(--conflict) 45%, transparent)",
                    }
                  : undefined
              }
              title="Cancel the statement on the server (⌘.)"
              onClick={cancel}
            >
              <StopIcon />
              Cancel
            </button>
            <div className="relative">
              <button
                type="button"
                className="btn btn-ghost"
                disabled={running}
                aria-haspopup="menu"
                onClick={() => setExplainMenu(!explainMenu)}
              >
                Explain
                <ChevronDown size={9} />
              </button>
              {explainMenu && (
                <Popover onClose={() => setExplainMenu(false)}>
                  <button
                    type="button"
                    role="menuitem"
                    className="menu-item"
                    onClick={() => {
                      setExplainMenu(false);
                      void run({ explain: "plan" });
                    }}
                  >
                    Explain
                    <span className="muted-in-menu ml-auto text-[11px] text-muted">
                      ⌘E
                    </span>
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    className="menu-item"
                    onClick={() => {
                      setExplainMenu(false);
                      void run({ explain: "analyze" });
                    }}
                  >
                    Explain Analyze (runs it)
                  </button>
                </Popover>
              )}
            </div>
            <div className="ml-auto flex shrink-0 items-center gap-2">
              <button
                type="button"
                className="btn btn-ghost"
                onClick={props.onSave}
                title="Save the tab's text as a saved query (⌘S)"
              >
                {saved ? "Save" : "Save Query…"}
              </button>
              <button
                type="button"
                className="btn btn-ghost icon-btn"
                aria-label={panelOpen ? "Hide side panel" : "Show side panel"}
                aria-pressed={panelOpen}
                onClick={() => setPanelOpen(!panelOpen)}
              >
                <PanelRightIcon />
              </button>
            </div>
          </div>
        </div>
        {transaction && (
          <div
            role="status"
            className={`flex flex-wrap items-center gap-2 border-b px-3 py-1.5 text-[12.5px] ${
              transaction.failed
                ? "bg-red-50 text-red-900 dark:bg-red-950 dark:text-red-100"
                : idleMinutes >= 5
                  ? "bg-amber-50 text-amber-900 dark:bg-amber-950 dark:text-amber-100"
                  : "bg-info-bg text-info-fg"
            }`}
          >
            <strong className="font-semibold">
              {transaction.failed ? "Transaction failed" : "Transaction open"}
            </strong>
            <span>
              {transaction.failed
                ? "· Roll Back to continue"
                : `· ${transaction.statements} statement${transaction.statements === 1 ? "" : "s"} · since ${new Date(transaction.since).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}`}
              {!transaction.failed &&
                idleMinutes >= 5 &&
                " · idle, holding locks others may wait on"}
            </span>
            <span className="flex-1" />
            {!transaction.failed && (
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => void endTransaction(true)}
              >
                Commit
              </button>
            )}
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => void endTransaction(false)}
            >
              Roll Back
            </button>
          </div>
        )}
        {paramNames.length > 0 && (
          <form
            ref={paramForm}
            className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b bg-panel px-3 py-2"
            aria-label="Parameters"
            onSubmit={(e) => {
              e.preventDefault();
              const pending = pendingRun.current ?? {};
              pendingRun.current = null;
              void run(pending);
            }}
          >
            {paramNames.map((name) => {
              const p = params.find((x) => x.name === name);
              const isNull = p?.value === null;
              const set = (value: string | null) =>
                setParams((ps) => [
                  ...ps.filter((x) => x.name !== name),
                  { name, value },
                ]);
              return (
                <label key={name} className="flex items-center gap-1.5">
                  <span className="mono text-[12px] text-fg-2">:{name}</span>
                  <input
                    className="text-input mono h-[26px] w-[180px]"
                    data-param={name}
                    disabled={isNull}
                    value={isNull ? "NULL" : (p?.value ?? "")}
                    onChange={(e) => set(e.target.value)}
                  />
                  <button
                    type="button"
                    className="btn btn-sm btn-ghost px-1.5 text-[11px]"
                    aria-pressed={isNull}
                    title="Send NULL"
                    onClick={() => set(isNull ? "" : null)}
                  >
                    NULL
                  </button>
                </label>
              );
            })}
            <button type="submit" className="btn btn-sm btn-primary">
              <PlayIcon size={9} />
              Run
            </button>
            {saved && (
              <span className="text-[11px] text-muted">
                Kept with the saved query.
              </span>
            )}
          </form>
        )}
        <div
          ref={host}
          className="h-[38%] min-h-[120px] shrink-0 overflow-hidden border-b"
        />
        <DbResults
          runs={runs}
          text={tab.text}
          running={running}
          startedAt={startedAt}
          timeoutSeconds={connection?.statement_timeout_seconds ?? 30}
          problem={problem}
          onFetchAll={(r) => void run({ fetchAll: r })}
          onExport={(r, f) => void exportRun(r, f)}
          onGoToError={(p) => view.current && markError(view.current, p, true)}
          onNotice={props.onNotice}
        />
      </div>
      {panelOpen && connection && (
        <DbSidePanel
          connection={connection}
          schema={props.schema?.schema ?? null}
          schemaError={props.schema?.error ?? null}
          section={section}
          onSection={setSection}
          onRefreshSchema={() => props.onSchema(connection.id, true)}
          queries={props.queries}
          runTick={runTick}
          onOpenSql={props.onOpenSql}
          onOpenQuery={props.onOpenQuery}
        />
      )}
      {askPassword && connection && (
        <PasswordDialog
          connection={connection}
          onClose={() => setAskPassword(null)}
          onUnlocked={(c) => {
            props.onConnectionChanged(c);
            const options = askPassword;
            setAskPassword(null);
            props.onSchema(c.id, false);
            void run({ ...options, unlocked: true });
          }}
        />
      )}
    </div>
  );
}

function Switcher({
  connections,
  current,
  onPick,
  onNew,
  onClose,
}: {
  connections: DbConnection[];
  current: DbConnection | null;
  onPick: (c: DbConnection) => void;
  onNew: () => void;
  onClose: () => void;
}) {
  const [filter, setFilter] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const lower = filter.trim().toLowerCase();
  const shown = connections.filter(
    (c) =>
      !lower ||
      c.name.toLowerCase().includes(lower) ||
      connectionPlace(c).toLowerCase().includes(lower),
  );
  return (
    <Popover onClose={onClose} className="w-[380px] max-h-[420px]">
      <div className="mb-1 flex items-center gap-1.5 rounded-md border border-control-line px-2 py-1 text-muted">
        <SearchIcon size={12} />
        <input
          className="min-w-0 flex-1 bg-transparent text-[12.5px] text-fg outline-none"
          placeholder="Switch this tab to…"
          aria-label="Find a connection"
          ref={input}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && shown[0]) onPick(shown[0]);
          }}
        />
      </div>
      {shown.map((c) => (
        <button
          key={c.id}
          type="button"
          role="menuitemradio"
          aria-checked={c.id === current?.id}
          aria-current={c.id === current?.id}
          className="menu-item h-[34px]"
          onClick={() => onPick(c)}
        >
          <span className="env-dot" data-env={c.environment} />
          <span className="font-medium">{c.name}</span>
          <span className="muted-in-menu truncate text-[11.5px] text-muted">
            {kindLabel(c)} · {connectionPlace(c)}
          </span>
          <span className="env-badge ml-auto" data-env={c.environment}>
            {ENV_SHORT[c.environment]}
          </span>
        </button>
      ))}
      <div className="menu-sep" />
      <button
        type="button"
        role="menuitem"
        className="menu-item"
        onClick={onNew}
      >
        <PlusIcon size={12} />
        New Connection…
      </button>
      {current && (
        <div className="px-2 pb-1 pt-1 text-[11px] text-muted">
          Switching keeps the tab's text and closes its session.
        </div>
      )}
    </Popover>
  );
}
