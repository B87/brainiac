import { useState } from "react";
import {
  type DbConnection,
  errorMessage,
  ipc,
  type SavedQuery,
} from "../lib/ipc";
import Dialog from "./Dialog";

/** The password of a connection that asks for it once per run. */
export function PasswordDialog({
  connection,
  onClose,
  onUnlocked,
}: {
  connection: DbConnection;
  onClose: () => void;
  onUnlocked: (connection: DbConnection) => void;
}) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const submit = async () => {
    try {
      onUnlocked(await ipc.unlockDbConnection(connection.id, password));
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return (
    <Dialog
      title={`Password for ${connection.name}`}
      width={420}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!password}
            onClick={() => void submit()}
          >
            Connect
          </button>
        </>
      }
    >
      <form
        className="flex flex-col gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <p className="m-0 text-[12.5px] text-fg-2">
          {connection.user}@{connection.host} · kept in memory until Brainiac
          quits.
        </p>
        <input
          className="text-input"
          type="password"
          aria-label="Password"
          autoFocus
          autoComplete="off"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        {error && <div className="text-[12px] text-conflict">{error}</div>}
      </form>
    </Dialog>
  );
}

/** Save Query (⌘S): a name, a folder, a description, and the connection. */
export function SaveQueryDialog({
  sql,
  existing,
  connections,
  connectionId,
  folders,
  onClose,
  onSaved,
}: {
  sql: string;
  /** The saved query the tab came from; Save updates it, Save as New does not. */
  existing: SavedQuery | null;
  connections: DbConnection[];
  connectionId: string | null;
  folders: string[];
  onClose: () => void;
  onSaved: (query: SavedQuery) => void;
}) {
  const [name, setName] = useState(existing?.name ?? "");
  const [folder, setFolder] = useState(existing?.folder ?? "");
  const [description, setDescription] = useState(existing?.description ?? "");
  const [connection, setConnection] = useState(
    existing?.connection_id ?? connectionId ?? "",
  );
  const [error, setError] = useState<string | null>(null);
  const save = async (asNew: boolean) => {
    try {
      onSaved(
        await ipc.saveQuery({
          id: asNew ? null : (existing?.id ?? null),
          expected_version: asNew ? null : (existing?.version ?? null),
          name,
          folder,
          description,
          connection_id: connection || null,
          sql,
        }),
      );
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return (
    <Dialog
      title={existing ? "Save Query" : "Save Query As"}
      width={480}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          {existing && (
            <button
              type="button"
              className="btn"
              disabled={!name.trim()}
              onClick={() => void save(true)}
            >
              Save as New
            </button>
          )}
          <button
            type="button"
            className="btn btn-primary"
            disabled={!name.trim()}
            onClick={() => void save(false)}
          >
            Save
          </button>
        </>
      }
    >
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          void save(false);
        }}
      >
        <label className="flex flex-col gap-1">
          <span className="text-[12px] font-medium text-fg-2">Name</span>
          <input
            className="text-input"
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-[12px] font-medium text-fg-2">Folder</span>
          <input
            className="text-input"
            list="saved-query-folders"
            placeholder="Such as Billing or Support/Weekly"
            value={folder}
            onChange={(e) => setFolder(e.target.value)}
          />
          <datalist id="saved-query-folders">
            {folders.map((f) => (
              <option key={f} value={f} />
            ))}
          </datalist>
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-[12px] font-medium text-fg-2">Description</span>
          <textarea
            className="text-input min-h-[60px] py-1.5"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-[12px] font-medium text-fg-2">Runs on</span>
          <select
            className="text-input"
            value={connection}
            onChange={(e) => setConnection(e.target.value)}
          >
            <option value="">No connection</option>
            {connections.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        </label>
        {error && (
          <div role="alert" className="selectable text-[12px] text-conflict">
            {error}
          </div>
        )}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}
