import { ask } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { ENV_SHORT, formatBytes, formatDuration } from "../lib/databases";
import {
  type DbConnection,
  errorMessage,
  type HealthPoint,
  type HealthSample,
  type HealthSession,
  ipc,
  type MachineSample,
  onDbHealthSample,
} from "../lib/ipc";
import { AlertIcon, PulseIcon } from "./icons";

const POINTS = 360;
/** Connections above this share of the limit are near it. */
const NEAR_LIMIT = 0.85;

type Props = {
  connection: DbConnection;
  visible: boolean;
  onEditConnection: () => void;
  onOpenSql: (sql: string, title?: string) => void;
  onNotice: (text: string) => void;
};

/** A connection's Health tab (SPEC.md, Databases: Health). */
export default function DbHealth({
  connection,
  visible,
  onEditConnection,
  onOpenSql,
  onNotice,
}: Props) {
  const [points, setPoints] = useState<HealthPoint[]>([]);
  const [sample, setSample] = useState<HealthSample | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Brainiac's window is minimised or hidden: Health stops sampling.
  const [pageVisible, setPageVisible] = useState(() => !document.hidden);
  useEffect(() => {
    const check = () => setPageVisible(!document.hidden);
    document.addEventListener("visibilitychange", check);
    return () => document.removeEventListener("visibilitychange", check);
  }, []);
  const sampling = visible && pageVisible;

  // Sampled only while the tab is on screen; the hour so far stays in Brainiac.
  useEffect(() => {
    if (!sampling) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void onDbHealthSample((e) => {
      if (e.sample.connection_id !== connection.id) return;
      setSample(e.sample);
      setPoints((p) => [...p.slice(-(POINTS - 1)), e.point]);
    }).then((u) => {
      if (disposed) u();
      else unlisten = u;
    });
    void ipc
      .startDbHealth(connection.id)
      .then((snapshot) => {
        if (disposed) return;
        setPoints(snapshot.points);
        setSample(snapshot.latest);
        setError(null);
      })
      .catch((e) => setError(errorMessage(e)));
    return () => {
      disposed = true;
      unlisten?.();
      void ipc.stopDbHealth(connection.id).catch(() => {});
    };
  }, [sampling, connection.id]);

  const used = sample?.total_connections ?? 0;
  const max = sample?.max_connections ?? 0;
  const near = max > 0 && used / max >= NEAR_LIMIT;
  const writable = connection.access === "read_write";

  const signal = async (s: HealthSession, terminate: boolean) => {
    const what = terminate ? "End Session" : "Cancel Query";
    const sure = await ask(
      `${terminate ? "End the session of" : "Cancel the statement of"} ${s.user ?? "another user"} (${s.application || "no application name"}, pid ${s.pid})${s.query ? `:\n\n${s.query.slice(0, 300)}` : "."}`,
      { title: what, kind: "warning", okLabel: what },
    );
    if (!sure) return;
    try {
      const done = await ipc.signalDbBackend(connection.id, s.pid, terminate);
      onNotice(done ? `${what}: sent` : "The session had already ended");
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const waiting = (sample?.sessions ?? []).filter((s) => s.blocked_by.length);

  return (
    <div
      className="min-h-0 flex-1 overflow-y-auto"
      style={{ display: visible ? "block" : "none" }}
    >
      <div className="mx-auto flex max-w-[1180px] flex-col gap-4 p-5">
        <header className="flex flex-wrap items-center gap-2.5">
          <PulseIcon size={16} className="text-fg-2" />
          <h2 className="m-0 text-[16px] font-semibold">
            {connection.name} health
          </h2>
          <span className="env-badge" data-env={connection.environment}>
            {ENV_SHORT[connection.environment]}
          </span>
          <span className="text-[12px] text-muted">
            {sample
              ? `Updated ${new Date(sample.at).toLocaleTimeString()} · every 10 s while shown · the last hour is kept in memory only`
              : "Reading…"}
          </span>
        </header>
        {(error || sample?.problem) && (
          <div className="db-error" role="alert">
            <AlertIcon className="mt-0.5 shrink-0 text-conflict" />
            <span className="selectable text-[12.5px]">
              {error ?? sample?.problem}
            </span>
          </div>
        )}
        {near && (
          <div
            role="status"
            className="flex items-center gap-2 rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-[12.5px] text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100"
          >
            <AlertIcon size={13} />
            <strong>Near the connection limit:</strong> {used} of {max} in use.
            New connections will be refused at {max}.
          </div>
        )}
        <div className="grid grid-cols-[repeat(auto-fit,minmax(220px,1fr))] gap-3">
          <div className="tile">
            <TileLabel source="Postgres">Connections</TileLabel>
            <div className="flex items-baseline gap-2">
              <span className="value">{sample ? used : "–"}</span>
              <span className="text-[12px] text-muted">of {max || "–"}</span>
              {near && (
                <span className="inline-flex items-center gap-1 text-[11.5px] font-semibold text-[var(--env-staging)]">
                  <AlertIcon size={11} /> Near the limit
                </span>
              )}
            </div>
            <div className="meter" data-warn={near}>
              <span
                style={{
                  width: `${max ? Math.min(100, (used / max) * 100) : 0}%`,
                }}
              />
            </div>
            <span className="text-[11.5px] text-muted">
              {sample
                ? `${sample.active} active · ${sample.idle} idle · ${sample.idle_in_transaction} idle in transaction`
                : ""}
            </span>
          </div>
          <div className="tile">
            <TileLabel source="Postgres">Cache hit ratio</TileLabel>
            <span className="value">
              {percent(sample?.cache_hit_ratio ?? null, 1)}
            </span>
            <Spark
              values={points.map((p) => p.cache_hit_ratio)}
              min={0}
              max={1}
            />
            <span className="text-[11.5px] text-muted">
              Reads found in memory, last 10 s
            </span>
          </div>
          <div className="tile">
            <TileLabel source="Postgres">Transactions</TileLabel>
            <span className="value">
              {sample?.transactions_per_second != null
                ? `${sample.transactions_per_second.toFixed(1)}/s`
                : "–"}
            </span>
            <Spark
              values={points.map((p) => p.transactions_per_second)}
              min={0}
            />
            <span className="text-[11.5px] text-muted">
              {sample
                ? `${(sample.rollbacks_per_second ?? 0).toFixed(1)}/s rolled back · ${sample.deadlocks} deadlock${sample.deadlocks === 1 ? "" : "s"}`
                : ""}
            </span>
          </div>
          <div className="tile">
            <TileLabel source="Postgres">Longest open transaction</TileLabel>
            <span className="value">
              {sample?.longest_transaction_seconds != null
                ? formatDuration(sample.longest_transaction_seconds * 1000)
                : "None"}
            </span>
            <span className="text-[11.5px] text-muted">
              {sample
                ? `${sample.temp_files_hour} temporary file${sample.temp_files_hour === 1 ? "" : "s"} written this hour (queries past work_mem)`
                : ""}
            </span>
          </div>
        </div>

        {connection.runs_on ? (
          <Machine machine={sample?.machine ?? null} points={points} />
        ) : (
          <div className="flex flex-wrap items-center gap-3 rounded-[10px] border border-dashed border-control-line px-4 py-3">
            <div className="flex min-w-[240px] flex-1 flex-col gap-0.5">
              <span className="text-[13px] font-medium">
                Where does this server run?
              </span>
              <span className="text-[12px] text-muted">
                PostgreSQL does not report its machine's memory, CPU, or disk.
                Brainiac can read them from where it runs.
              </span>
            </div>
            <button type="button" className="btn" onClick={onEditConnection}>
              Google Cloud SQL…
            </button>
            <button type="button" className="btn" onClick={onEditConnection}>
              Docker Container…
            </button>
            <button
              type="button"
              className="btn"
              disabled
              title="Coolify serves its metrics only on the server; this waits for SSH tunnels."
            >
              Coolify · needs SSH
            </button>
          </div>
        )}

        {sample && !sample.sees_all && (
          <div className="rounded-lg border bg-info-bg px-3 py-2 text-[12.5px] text-info-fg">
            Other users' statements are hidden. Grant the role{" "}
            <span className="mono">pg_monitor</span> to{" "}
            <span className="mono">{connection.user}</span> to see them.
          </div>
        )}

        <Section title="Sessions" count={sample?.sessions.length}>
          <table className="db-table">
            <thead>
              <tr>
                <th>pid</th>
                <th>User</th>
                <th>Application</th>
                <th>State</th>
                <th className="r">For</th>
                <th>Waiting</th>
                <th>Statement</th>
                {writable && <th />}
              </tr>
            </thead>
            <tbody>
              {(sample?.sessions ?? []).map((s) => (
                <tr key={s.pid}>
                  <td className="mono">{s.pid}</td>
                  <td>{s.user ?? ""}</td>
                  <td>
                    {s.application}
                    {s.own && (
                      <span className="ml-1 text-muted">(Brainiac)</span>
                    )}
                  </td>
                  <td>
                    <span
                      className={
                        s.state?.startsWith("idle in transaction")
                          ? "font-medium text-[var(--env-staging)]"
                          : s.blocked_by.length
                            ? "font-medium text-conflict"
                            : ""
                      }
                    >
                      {s.blocked_by.length ? "waiting" : (s.state ?? "")}
                    </span>
                  </td>
                  <td className="r">
                    {s.state_seconds != null
                      ? formatDuration(s.state_seconds * 1000)
                      : ""}
                  </td>
                  <td className="text-[12px] text-muted">{s.wait ?? ""}</td>
                  <td
                    className="mono max-w-[420px] truncate text-[11.5px]"
                    title={s.query ?? ""}
                  >
                    {s.query ?? <span className="text-faint">hidden</span>}
                  </td>
                  {writable && (
                    <td className="whitespace-nowrap">
                      {!s.own && (
                        <>
                          <button
                            type="button"
                            className="btn btn-sm btn-ghost"
                            onClick={() => void signal(s, false)}
                          >
                            Cancel Query
                          </button>
                          <button
                            type="button"
                            className="btn btn-sm btn-ghost text-conflict"
                            onClick={() => void signal(s, true)}
                          >
                            End Session
                          </button>
                        </>
                      )}
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </Section>

        {waiting.length > 0 && (
          <Section title="Waiting on locks" count={waiting.length}>
            <ul className="m-0 flex list-none flex-col gap-1 p-3 text-[12.5px]">
              {waiting.map((s) => {
                const blockers = (sample?.sessions ?? []).filter((b) =>
                  s.blocked_by.includes(b.pid),
                );
                return (
                  <li key={s.pid}>
                    <span className="mono">{s.pid}</span> (
                    {s.application || s.user}) waits for{" "}
                    {blockers.length
                      ? blockers.map((b, i) => (
                          <span key={b.pid}>
                            {i > 0 && ", "}
                            <span className="mono">{b.pid}</span> (
                            {b.application || b.user}, {b.state})
                          </span>
                        ))
                      : s.blocked_by.join(", ")}
                  </li>
                );
              })}
            </ul>
          </Section>
        )}

        <Section title="Most time spent">
          {sample?.statements ? (
            <table className="db-table">
              <thead>
                <tr>
                  <th>Statement</th>
                  <th className="r">Calls</th>
                  <th className="r">Total</th>
                  <th className="r">Mean</th>
                  <th className="r">Rows</th>
                </tr>
              </thead>
              <tbody>
                {sample.statements.map((s) => (
                  <tr
                    key={s.query}
                    className="cursor-default hover:bg-control"
                    title="Open with explain in a new tab"
                    onClick={() => onOpenSql(`explain ${s.query}`, "Explain")}
                  >
                    <td className="mono max-w-[560px] truncate text-[11.5px]">
                      {s.query}
                    </td>
                    <td className="r">{s.calls.toLocaleString("en-US")}</td>
                    <td className="r">{formatDuration(s.total_ms)}</td>
                    <td className="r">{formatDuration(s.mean_ms)}</td>
                    <td className="r">{s.rows.toLocaleString("en-US")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <p className="m-0 p-3 text-[12.5px] text-muted">
              Needs the <span className="mono">pg_stat_statements</span>{" "}
              extension: add it to{" "}
              <span className="mono">shared_preload_libraries</span>, restart
              the server, and run{" "}
              <span className="mono">CREATE EXTENSION pg_stat_statements</span>.
              On Cloud SQL, set the flag{" "}
              <span className="mono">cloudsql.enable_pg_stat_statements</span>.
            </p>
          )}
        </Section>

        <Section title="Largest tables">
          <table className="db-table">
            <thead>
              <tr>
                <th>Table</th>
                <th className="r">Size</th>
                <th className="r">Rows</th>
                <th className="r">Dead rows</th>
                <th>Last autovacuum</th>
              </tr>
            </thead>
            <tbody>
              {(sample?.tables ?? []).map((t) => (
                <tr key={`${t.schema}.${t.name}`}>
                  <td className="mono">
                    {t.schema}.{t.name}
                  </td>
                  <td className="r">{formatBytes(t.total_bytes)}</td>
                  <td className="r">{t.live_rows.toLocaleString("en-US")}</td>
                  <td
                    className={`r ${(t.dead_ratio ?? 0) > 0.2 ? "font-medium text-[var(--env-staging)]" : ""}`}
                  >
                    {percent(t.dead_ratio, 0)}
                  </td>
                  <td className="text-[12px] text-muted">
                    {t.last_autovacuum
                      ? new Date(t.last_autovacuum).toLocaleString()
                      : "never"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="m-0 px-3 pb-2 text-[11px] text-muted">
            Sizes as of each table's last vacuum or analyze, read without
            waiting for locks.
          </p>
        </Section>
      </div>
    </div>
  );
}

function percent(value: number | null, digits: number): string {
  return value === null ? "–" : `${(value * 100).toFixed(digits)}%`;
}

function TileLabel({
  children,
  source,
}: {
  children: React.ReactNode;
  source: string;
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-[12px] font-medium text-fg-2">{children}</span>
      <span className="ml-auto text-[10.5px] text-muted">{source}</span>
    </div>
  );
}

function Section({
  title,
  count,
  children,
}: {
  title: string;
  count?: number;
  children: React.ReactNode;
}) {
  return (
    <section className="overflow-hidden rounded-[10px] border">
      <h3 className="m-0 flex items-center gap-2 border-b bg-header px-3 py-2 text-[12.5px] font-semibold">
        {title}
        {count !== undefined && (
          <span className="font-normal text-muted">{count}</span>
        )}
      </h3>
      <div className="overflow-x-auto">{children}</div>
    </section>
  );
}

/** A one-hour line: muted, with its last value as a dot. */
function Spark({
  values,
  min,
  max,
}: {
  values: (number | null)[];
  min: number;
  max?: number;
}) {
  const w = 200;
  const h = 28;
  const known = values
    .map((v, i) => (v === null ? null : { v, i }))
    .filter((p): p is { v: number; i: number } => p !== null);
  if (known.length < 2) return <div className="h-[28px]" />;
  const top = max ?? Math.max(...known.map((p) => p.v), min + 1e-9);
  const span = Math.max(1, POINTS - 1);
  const x = (i: number) => ((i + (POINTS - values.length)) / span) * w;
  const y = (v: number) => h - 2 - ((v - min) / (top - min || 1)) * (h - 4);
  const line = known
    .map((p) => `${x(p.i).toFixed(1)},${y(p.v).toFixed(1)}`)
    .join(" ");
  const last = known[known.length - 1];
  return (
    <svg
      className="spark"
      viewBox={`0 0 ${w} ${h}`}
      width="100%"
      height={h}
      preserveAspectRatio="none"
      role="img"
      aria-label="The last hour"
    >
      <polyline
        points={line}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        vectorEffect="non-scaling-stroke"
      />
      {last && <circle className="end" cx={x(last.i)} cy={y(last.v)} r="2.5" />}
    </svg>
  );
}

function Machine({
  machine,
  points,
}: {
  machine: MachineSample | null;
  points: HealthPoint[];
}) {
  const source = machine?.source || "Machine";
  return (
    <div className="grid grid-cols-[repeat(auto-fit,minmax(220px,1fr))] gap-3">
      {machine?.problem && (
        <div className="db-error col-span-full" role="alert">
          <AlertIcon className="mt-0.5 shrink-0 text-conflict" />
          <span className="selectable text-[12.5px]">{machine.problem}</span>
        </div>
      )}
      <div className="tile">
        <TileLabel source={source}>Memory</TileLabel>
        <div className="flex items-baseline gap-2">
          <span className="value">
            {percent(machine?.memory_ratio ?? null, 0)}
          </span>
          <span className="text-[12px] text-muted">
            {machine?.memory_used != null
              ? formatBytes(machine.memory_used)
              : ""}
            {machine?.memory_limit != null
              ? ` of ${formatBytes(machine.memory_limit)}`
              : ""}
          </span>
        </div>
        <div
          className="meter"
          data-warn={(machine?.memory_ratio ?? 0) >= NEAR_LIMIT}
        >
          <span
            style={{
              width: `${Math.min(100, (machine?.memory_ratio ?? 0) * 100)}%`,
            }}
          />
        </div>
        <Spark values={points.map((p) => p.memory_ratio)} min={0} max={1} />
      </div>
      <div className="tile">
        <TileLabel source={source}>CPU</TileLabel>
        <span className="value">{percent(machine?.cpu_ratio ?? null, 0)}</span>
        <Spark values={points.map((p) => p.cpu_ratio)} min={0} max={1} />
        <span className="text-[11.5px] text-muted">
          Share of the CPUs it may use
        </span>
      </div>
      <div className="tile">
        <TileLabel source={source}>Disk</TileLabel>
        {machine?.disk_used != null ? (
          <>
            <div className="flex items-baseline gap-2">
              <span className="value">{formatBytes(machine.disk_used)}</span>
              {machine.disk_quota != null && (
                <span className="text-[12px] text-muted">
                  of {formatBytes(machine.disk_quota)}
                </span>
              )}
            </div>
            {machine.disk_quota != null && machine.disk_quota > 0 && (
              <div
                className="meter"
                data-warn={machine.disk_used / machine.disk_quota >= NEAR_LIMIT}
              >
                <span
                  style={{
                    width: `${Math.min(100, (machine.disk_used / machine.disk_quota) * 100)}%`,
                  }}
                />
              </div>
            )}
          </>
        ) : (
          <>
            <span className="value">
              {machine?.disk_read_rate != null
                ? `${formatBytes(machine.disk_read_rate)}/s`
                : "–"}
            </span>
            <span className="text-[11.5px] text-muted">
              read ·{" "}
              {machine?.disk_write_rate != null
                ? `${formatBytes(machine.disk_write_rate)}/s`
                : "–"}{" "}
              written · Docker reports I/O, not space used
            </span>
          </>
        )}
      </div>
    </div>
  );
}
