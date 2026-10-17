import { useCallback, useEffect, useRef, useState } from "react";
import { relativeTime } from "../lib/format";
import {
  removable,
  shareLabel,
  stateHelp,
  stateText,
  stateTone,
} from "../lib/hostSecrets";
import {
  type AgentHost,
  errorMessage,
  type HostSecretRow,
  type HostSecrets,
  ipc,
} from "../lib/ipc";
import { Card, Hint } from "./SettingsPanes";

const TONE_CLASS = {
  ok: "text-fg-2",
  warn: "text-conflict",
  bad: "text-conflict",
  idle: "text-muted",
} as const;

/**
 * Held here (docs/design/scheduled-runs.md): the named secrets from
 * Settings → Secrets the host holds or was sent, as one line when all is
 * well and a row each otherwise, with Send Again and Remove from Host.
 * Opening it asks the host what it holds; it never asks for a value, and the
 * host never returns one. A copy does not follow a change made here until it
 * is shared again. Nothing is shared from here: a routine's approval or a
 * run's start sends what it needs.
 */
export default function HostSecretsGroup({
  host,
  onSecrets,
}: {
  host: AgentHost;
  /** Opens Settings › Secrets, where each copy is listed with its secret. */
  onSecrets?: () => void;
}) {
  const [state, setState] = useState<HostSecrets | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  // A response for a host the page has since left is dropped.
  const current = useRef(host.id);
  current.current = host.id;
  const load = useCallback(
    async (keepError = false) => {
      const asked = host.id;
      try {
        const next = await ipc.getHostSecrets(asked);
        if (current.current !== asked) return;
        setState(next);
        if (!keepError) setError(null);
      } catch (e) {
        if (current.current === asked) setError(errorMessage(e));
      }
    },
    [host.id],
  );
  useEffect(() => {
    setState(null);
    setError(null);
    void load();
  }, [load]);

  const act = async (
    row: HostSecretRow,
    action: (id: string, hostId: string) => Promise<HostSecrets>,
  ) => {
    setBusy(row.secret_id);
    setError(null);
    try {
      const next = await action(row.secret_id, host.id);
      if (current.current === host.id) setState(next);
    } catch (e) {
      setError(errorMessage(e));
      // A failed Share is recorded on the row: show it, and keep the message.
      await load(true);
    } finally {
      setBusy(null);
    }
  };

  const removeUnlisted = async (name: string) => {
    setBusy(name);
    setError(null);
    try {
      const next = await ipc.removeUnlistedHostSecret(name, host.id);
      if (current.current === host.id) setState(next);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  // Only what the host holds or was sent.
  const rows = state?.rows.filter((r) => r.state !== "not_shared") ?? [];
  const sealed = rows.filter((r) => r.state === "shared").length;
  const attention = rows.length - sealed + (state?.only_on_host.length ?? 0);
  const summary = !state
    ? "Asking the host…"
    : rows.length === 0 && state.only_on_host.length === 0
      ? "No secrets yet. One comes here with a routine's approval or a run's start."
      : [
          sealed > 0 &&
            `${sealed} ${sealed === 1 ? "secret" : "secrets"} sealed`,
          attention > 0 &&
            `${attention} ${attention === 1 ? "needs" : "need"} attention`,
        ]
          .filter(Boolean)
          .join(" · ");

  return (
    <Card
      title="Held here"
      actions={
        onSecrets && (
          <button
            type="button"
            className="btn btn-sm btn-ghost text-link"
            onClick={onSecrets}
          >
            See in Secrets
          </button>
        )
      }
    >
      <div className="settings-row">
        <span className="min-w-0 flex-1 text-[12.5px]">{summary}</span>
        <Hint>
          Sealed so only the host's controller can open them. Anyone who
          administers {host.name} can still reach what a run uses.
        </Hint>
      </div>
      {rows.map((row) => (
        <fieldset
          key={row.secret_id}
          className="settings-row m-0 flex min-w-0 flex-col items-stretch gap-1 border-0"
          aria-label={row.label}
        >
          <div className="flex items-center gap-2">
            <span className="min-w-0 flex-1 truncate text-[12.5px]">
              <span className="mono">{row.env_name}</span>{" "}
              <span className="text-muted">{row.label}</span>
            </span>
            <span className={`text-[12px] ${TONE_CLASS[stateTone(row.state)]}`}>
              {stateText(row.state)}
              {row.shared_at &&
                row.state !== "not_shared" &&
                row.state !== "failed" &&
                ` · ${relativeTime(row.shared_at)}`}
            </span>
            {row.state !== "shared" && (
              <button
                type="button"
                className="btn btn-sm"
                disabled={busy !== null}
                onClick={() => void act(row, ipc.shareSecretWithHost)}
              >
                {busy === row.secret_id ? "Working…" : shareLabel(row.state)}
              </button>
            )}
            {removable(row.state) && (
              <button
                type="button"
                className="btn btn-sm"
                disabled={busy !== null}
                onClick={() => void act(row, ipc.removeSecretFromHost)}
              >
                Remove from Host
              </button>
            )}
          </div>
          {row.state !== "shared" && <Hint>{stateHelp(row)}</Hint>}
        </fieldset>
      ))}
      {state?.only_on_host.map((s) => (
        <div key={s.name} className="settings-row text-[12.5px]">
          <span className="mono min-w-0 flex-1">{s.name}</span>
          <Hint>Held by the host, but not a secret in Settings → Secrets.</Hint>
          <button
            type="button"
            className="btn btn-sm"
            disabled={busy !== null}
            onClick={() => void removeUnlisted(s.name)}
          >
            Remove from Host
          </button>
        </div>
      ))}
      {state?.problem && (
        <div className="settings-row text-[12.5px] text-conflict" role="alert">
          {state.problem} Brainiac can only show what it sent.
        </div>
      )}
      {error && (
        <div className="settings-row text-[12.5px] text-conflict" role="alert">
          {error}
        </div>
      )}
    </Card>
  );
}
