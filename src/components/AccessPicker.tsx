import { useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import {
  errorMessage,
  type HostSecrets,
  ipc,
  type NamedSecret,
} from "../lib/ipc";
import Dialog from "./Dialog";
import NamedSecretForm from "./NamedSecretForm";

/**
 * What a host holds of the named secrets, asked when `hostId` is a run host
 * (`null` for this Mac, while loading, or when the host did not answer).
 */
export function useHostCopies(hostId: string): HostSecrets | null {
  const [held, setHeld] = useState<HostSecrets | null>(null);
  useEffect(() => {
    setHeld(null);
    if (!hostId) return;
    let alive = true;
    ipc
      .getHostSecrets(hostId)
      .then((h) => alive && setHeld(h.problem ? null : h))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [hostId]);
  return held;
}

/** The chosen secrets the host does not hold as they are now: they go with
 * the approval, or before the run starts. */
export function toSend(chosen: string[], held: HostSecrets | null): string[] {
  if (!held) return [];
  return held.rows
    .filter((r) => chosen.includes(r.secret_id) && r.state !== "shared")
    .map((r) => r.env_name);
}

/**
 * A run's or routine's access (docs/design/routines-ux.md): the named
 * secrets its runs get, ticked here and nowhere else. New access… adds a
 * secret and ticks it. Nothing is sent from here; the approval or Start does
 * it, and says so.
 */
export default function AccessPicker({
  hostId,
  hostName,
  chosen,
  onChange,
  sentWith,
}: {
  /** Empty: this Mac, which keeps no secrets. */
  hostId: string;
  hostName: string;
  chosen: string[];
  onChange: (ids: string[]) => void;
  /** "the approval" or "Start": when a secret the host lacks goes there. */
  sentWith: string;
}) {
  const [secrets, setSecrets] = useState<NamedSecret[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const held = useHostCopies(hostId);

  const load = useCallback(async () => {
    try {
      setSecrets(await ipc.listNamedSecrets());
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const toggle = (id: string, on: boolean) =>
    onChange(on ? [...chosen, id] : chosen.filter((c) => c !== id));
  const stateOf = (id: string) =>
    held?.rows.find((r) => r.secret_id === id)?.state;

  return (
    <div className="flex flex-col gap-1.5">
      {secrets === null ? (
        <span className="text-muted">{error ?? "Loading…"}</span>
      ) : secrets.length === 0 && !adding ? (
        <span className="text-muted">
          No secrets yet. A run gets none unless you add one here.
        </span>
      ) : (
        <ul className="m-0 flex list-none flex-col gap-1 p-0">
          {secrets.map((s) => {
            const on = chosen.includes(s.id);
            const state = stateOf(s.id);
            return (
              <li key={s.id} className="flex items-center gap-2">
                <input
                  type="checkbox"
                  id={`access-${s.id}`}
                  checked={on}
                  onChange={(e) => toggle(s.id, e.target.checked)}
                />
                <label
                  htmlFor={`access-${s.id}`}
                  className="flex min-w-0 flex-1 items-baseline gap-2"
                >
                  <code className="mono">{s.env_name}</code>
                  <span className="truncate text-[12px] text-muted">
                    {s.label} · {s.as_file ? "a file" : "a value"}
                  </span>
                </label>
                {on && hostId && held && (
                  <span className="text-[12px] text-muted">
                    {state === "shared"
                      ? `On ${hostName}`
                      : `Goes to ${hostName} with ${sentWith}`}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {/* A dialog on the page's body: the picker sits inside a routine's
          form, and a form may not hold another. */}
      {adding &&
        createPortal(
          <Dialog title="New access" onClose={() => setAdding(false)}>
            <NamedSecretForm
              secret={null}
              onSaved={(saved) => {
                setAdding(false);
                void load();
                onChange([...chosen, saved.id]);
              }}
              onCancel={() => setAdding(false)}
            />
          </Dialog>,
          document.body,
        )}
      <div>
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => setAdding(true)}
        >
          New Access…
        </button>
      </div>
      {chosen.length > 0 && !hostId ? (
        <span className="text-[12px] text-conflict">
          Secrets reach a run only on a Linux run host, which keeps them sealed.
          Choose a run host, or clear these.
        </span>
      ) : (
        <span className="text-[12px] text-muted">
          A run can print any secret it holds, and its network is not
          restricted: give it tokens that can read and nothing more.
        </span>
      )}
    </div>
  );
}
