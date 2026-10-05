import { useEffect, useState } from "react";
import {
  type CredentialOwner,
  errorMessage,
  ipc,
  type SecretEntry,
  type SecretsOverview,
} from "../lib/ipc";
import { commandPreview, sourceLabel, stateLabel } from "../lib/secrets";

const ownerKey = (o: CredentialOwner) =>
  o.kind === "forge_account"
    ? `forge:${o.provider}`
    : o.kind === "db_connection"
      ? `db:${o.id}`
      : `agent:${o.id}`;

/**
 * Settings → Secrets (SPEC.md, Secrets): the store in use, and where each
 * account's token and each connection's password comes from. Opening it
 * reads no secret, runs no program, and unlocks nothing.
 */
export default function SecretsSection() {
  const [overview, setOverview] = useState<SecretsOverview | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    ipc
      .listSecrets()
      .then((o) => alive && setOverview(o))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, []);

  return (
    <div className="flex flex-col gap-2">
      {overview && (
        <p className="m-0 text-[12.5px] text-fg-2">
          Store: <span className="font-medium">{overview.store}</span>, items{" "}
          <span className="mono">brainiac/…</span>. Brainiac writes only there;
          every other source is read, never changed.
        </p>
      )}
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}
      {overview?.entries.length === 0 && (
        <p className="m-0 text-[12.5px] text-muted">
          No account or connection has a secret yet.
        </p>
      )}
      {overview?.entries.map((entry) => (
        <SecretRow
          key={ownerKey(entry.owner)}
          entry={entry}
          onOverview={setOverview}
        />
      ))}
      {!overview && !error && (
        <div className="text-[12px] text-muted">Loading…</div>
      )}
    </div>
  );
}

function SecretRow({
  entry,
  onOverview,
}: {
  entry: SecretEntry;
  onOverview: (o: SecretsOverview) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshed, setRefreshed] = useState(false);
  const { state, source } = entry;

  const act = async (action: () => Promise<SecretsOverview | null>) => {
    setBusy(true);
    setError(null);
    try {
      const next = await action();
      if (next) onOverview(next);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  };

  const refreshable =
    source.kind !== "none" &&
    !state.needs_approval &&
    state.pending !== "save" &&
    state.pending !== "removal";

  return (
    <section
      className="flex flex-col gap-2 rounded-[10px] border bg-app px-3.5 py-3"
      aria-label={entry.label}
    >
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{entry.label}</div>
          <div className="text-[12px] text-fg-2">
            {sourceLabel(source)} → {entry.destination}
          </div>
          <div
            className={`text-[12px] ${state.needs_approval || state.pending ? "text-conflict" : "text-muted"}`}
          >
            {stateLabel(entry)}
          </div>
        </div>
        <div className="flex shrink-0 gap-2">
          {state.needs_approval && !confirming && (
            <button
              type="button"
              className="btn btn-sm btn-primary"
              onClick={() => setConfirming(true)}
            >
              Allow…
            </button>
          )}
          {(state.pending === "cleanup" || state.pending === "removal") && (
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={() =>
                void act(() => ipc.retryCredentialCleanup(entry.owner))
              }
            >
              {busy ? "Retrying…" : "Retry"}
            </button>
          )}
          {refreshable && (
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              title="Forget the secret kept for this run, so it is read or asked for again"
              onClick={() =>
                void act(async () => {
                  await ipc.refreshCredential(entry.owner);
                  setRefreshed(true);
                  return null;
                })
              }
            >
              Refresh
            </button>
          )}
        </div>
      </div>
      {refreshed && (
        <p className="m-0 text-[12px] text-muted">
          Forgotten: it is read, or asked for, again when next used.
        </p>
      )}
      {confirming && (
        <div className="flex flex-col gap-2" role="alert">
          <p className="m-0 text-[12.5px]">
            Brainiac will read {sourceLabel(source)} and send it only to{" "}
            {entry.destination}.
          </p>
          {source.kind === "command" && (
            <code className="mono selectable break-all rounded-md border bg-header px-2 py-1 text-[11.5px]">
              {commandPreview(source.program, source.args)}
            </code>
          )}
          {source.kind === "command" && (
            <p className="m-0 text-[12px] text-muted">
              The program runs with your permissions, without a shell. Allow it
              only if you recognize it and its arguments.
            </p>
          )}
          <div className="flex gap-2">
            <button
              type="button"
              className="btn btn-sm btn-primary"
              disabled={busy}
              onClick={() =>
                void act(() =>
                  ipc.approveSecretSource(entry.owner, state.revision),
                )
              }
            >
              Allow This Source
            </button>
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={() => setConfirming(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}
    </section>
  );
}
