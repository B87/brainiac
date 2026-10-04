import { useCallback, useEffect, useState } from "react";
import { ENV_SHORT } from "../lib/databases";
import {
  type DbConnection,
  errorMessage,
  ipc,
  type RepositorySummary,
} from "../lib/ipc";
import { DatabaseIcon } from "./icons";

/**
 * The database connections linked to a repository, so a project's database
 * is one click from its code (SPEC.md, Databases: Connections).
 */
export default function RepositoryDatabases({
  repository,
  onNewQuery,
  onError,
}: {
  repository: RepositorySummary;
  onNewQuery: (connectionId: string) => void;
  onError: (message: string | null) => void;
}) {
  const [connections, setConnections] = useState<DbConnection[]>([]);
  const load = useCallback(() => {
    void ipc
      .listDbConnections()
      .then(setConnections)
      .catch((e) => onError(errorMessage(e)));
  }, [onError]);
  useEffect(load, [load]);
  const linked = connections.filter((c) =>
    c.repository_ids.includes(repository.id),
  );
  const others = connections.filter(
    (c) => !c.repository_ids.includes(repository.id),
  );
  const link = async (id: string, on: boolean) => {
    try {
      await ipc.linkDbConnection(id, repository.id, on);
      load();
    } catch (e) {
      onError(errorMessage(e));
    }
  };
  if (connections.length === 0) return null;
  return (
    <div className="border-t">
      <div className="section-label px-3 pt-3 pb-1">Databases</div>
      {linked.map((c) => (
        <div key={c.id} className="flex items-center gap-2 px-3 py-1.5">
          <DatabaseIcon size={12} className="shrink-0 text-muted" />
          <span className="truncate">{c.name}</span>
          <span className="env-badge" data-env={c.environment}>
            {ENV_SHORT[c.environment]}
          </span>
          <span className="flex-1" />
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => onNewQuery(c.id)}
          >
            New Query
          </button>
          <button
            type="button"
            className="btn btn-sm btn-ghost"
            onClick={() => void link(c.id, false)}
          >
            Unlink
          </button>
        </div>
      ))}
      {others.length > 0 && (
        <div className="px-3 py-2">
          <select
            className="text-input w-full"
            aria-label={`Link a connection to ${repository.name}`}
            value=""
            onChange={(e) => {
              if (e.target.value) void link(e.target.value, true);
            }}
          >
            <option value="">Link a connection…</option>
            {others.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        </div>
      )}
    </div>
  );
}
