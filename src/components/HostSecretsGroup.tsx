import { useCallback, useEffect, useState } from "react";
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
import { Group, Hint } from "./SettingsPanes";

const TONE_CLASS = {
  ok: "text-fg-2",
  warn: "text-conflict",
  bad: "text-conflict",
  idle: "text-muted",
} as const;

/**
 * Secrets on a run host (docs/design/scheduled-runs.md): each named secret
 * from Settings → Secrets, whether the host has a copy, and Share and Remove.
 * Opening it asks the host what it holds; it never asks for a value, and the
 * host never returns one. A copy does not follow a change made here until it
 * is shared again.
 */
export default function HostSecretsGroup({ host }: { host: AgentHost }) {
  const [state, setState] = useState<HostSecrets | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setState(await ipc.getHostSecrets(host.id));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [host.id]);
  useEffect(() => {
    void load();
  }, [load]);

  const act = async (
    row: HostSecretRow,
    action: (id: string, hostId: string) => Promise<HostSecrets>,
  ) => {
    setBusy(row.secret_id);
    setError(null);
    try {
      setState(await action(row.secret_id, host.id));
    } catch (e) {
      setError(errorMessage(e));
      // The failure is recorded on the share: show it.
      await load();
    } finally {
      setBusy(null);
    }
  };

  return (
    <Group label="Secrets on this host">
      <div className="settings-group flex flex-col">
        {state?.rows.length === 0 && (
          <div className="settings-row text-[12.5px] text-muted">
            No secrets yet. Add one in Settings → Secrets, then share it here.
          </div>
        )}
        {state?.rows.map((row) => (
          <fieldset
            key={row.secret_id}
            className="settings-row m-0 flex min-w-0 flex-col items-stretch gap-1 border-0"
            aria-label={row.label}
          >
            <div className="flex items-center gap-2">
              <span className="min-w-0 flex-1 truncate font-medium">
                {row.label}{" "}
                <span className="mono text-[12px] font-normal text-muted">
                  {row.env_name}
                </span>
              </span>
              <span
                className={`text-[12px] ${TONE_CLASS[stateTone(row.state)]}`}
              >
                {stateText(row.state)}
                {row.shared_at &&
                  row.state !== "not_shared" &&
                  row.state !== "failed" &&
                  ` · ${relativeTime(row.shared_at)}`}
              </span>
              <button
                type="button"
                className="btn btn-sm"
                disabled={busy !== null}
                onClick={() => void act(row, ipc.shareSecretWithHost)}
              >
                {busy === row.secret_id ? "Working…" : shareLabel(row.state)}
              </button>
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
            <Hint>{stateHelp(row)}</Hint>
          </fieldset>
        ))}
        {state?.only_on_host.map((s) => (
          <div key={s.name} className="settings-row text-[12.5px]">
            <span className="mono min-w-0 flex-1">{s.name}</span>
            <Hint>
              Held by the host, but not a secret in Settings → Secrets.
            </Hint>
          </div>
        ))}
        {state?.problem && (
          <div
            className="settings-row text-[12.5px] text-conflict"
            role="alert"
          >
            {state.problem} Brainiac can only show what it sent.
          </div>
        )}
        {error && (
          <div
            className="settings-row text-[12.5px] text-conflict"
            role="alert"
          >
            {error}
          </div>
        )}
        {!state && !error && (
          <div className="settings-row text-[12px] text-muted">
            Asking the host…
          </div>
        )}
      </div>
      <Hint>
        The host keeps a sealed copy that only its controller can open, so a run
        started on {host.name} can use it without this Mac. Anyone with access
        to the host or its administrator account can still reach what a run
        uses: share tokens that can read and nothing more.
      </Hint>
    </Group>
  );
}
