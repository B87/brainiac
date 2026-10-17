import { Fragment, useEffect, useState } from "react";
import {
  type CredentialOwner,
  errorMessage,
  ipc,
  type NamedSecret,
  type NamedSecretUsage,
  type SecretEntry,
  type SecretsOverview,
} from "../lib/ipc";
import { commandPreview, sourceLabel, stateLabel } from "../lib/secrets";
import NamedSecretForm from "./NamedSecretForm";
import { PageTitle } from "./SettingsPanes";

const ownerKey = (o: CredentialOwner) =>
  o.kind === "forge_account"
    ? `forge:${o.provider}`
    : o.kind === "db_connection"
      ? `db:${o.id}`
      : o.kind === "named_secret"
        ? `named:${o.id}`
        : `agent:${o.id}`;

/**
 * Settings → Secrets (SPEC.md, Secrets): the store in use, and where each
 * account's token and each connection's password comes from. Opening it
 * reads no secret, runs no program, and unlocks nothing.
 */
export default function SecretsSection() {
  const [overview, setOverview] = useState<SecretsOverview | null>(null);
  const [named, setNamed] = useState<NamedSecret[]>([]);
  const [usage, setUsage] = useState<NamedSecretUsage[]>([]);
  // A form is open to add a secret (null) or to change one.
  const [editing, setEditing] = useState<NamedSecret | null | undefined>(
    undefined,
  );
  const [error, setError] = useState<string | null>(null);
  // The named secret whose details are open.
  const [open, setOpen] = useState<string | null>(null);
  const others =
    overview?.entries.filter((e) => e.owner.kind !== "named_secret") ?? [];

  const load = () => {
    ipc
      .listSecrets()
      .then(setOverview)
      .catch((e) => setError(errorMessage(e)));
    ipc
      .listNamedSecrets()
      .then(setNamed)
      .catch((e) => setError(errorMessage(e)));
    ipc
      .namedSecretUsage()
      .then(setUsage)
      .catch(() => {});
  };
  useEffect(load, []);

  return (
    <div className="flex flex-col gap-2">
      <PageTitle
        title="Secrets"
        actions={
          <button
            type="button"
            className="btn btn-sm btn-primary"
            onClick={() => setEditing(null)}
          >
            Add Secret…
          </button>
        }
      >
        Every credential Brainiac knows, where each comes from, and every host a
        copy went to. Change an account's or a connection's source where it is
        entered: in Accounts, or in the connection's Edit Connection….
      </PageTitle>
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
      <section aria-label="Access for runs" className="flex flex-col gap-2">
        <span className="mt-2 flex flex-col">
          <h2 className="m-0 text-[13px] font-semibold">Access for runs</h2>
          <span className="text-[12px] text-muted">
            Tokens and keys of your own. A run or routine chooses them under
            Access; a run host gets a sealed copy when one of them first needs
            it there.
          </span>
        </span>
        {editing !== undefined && (
          <NamedSecretForm
            key={editing?.id ?? "new"}
            secret={editing}
            onCancel={() => setEditing(undefined)}
            onSaved={() => {
              setEditing(undefined);
              load();
            }}
          />
        )}
        {named.length === 0 ? (
          <p className="m-0 text-[12.5px] text-muted">None yet.</p>
        ) : (
          <div className="overflow-x-auto rounded-[10px] border bg-app">
            <table
              aria-label="Access for runs"
              className="w-full border-collapse"
            >
              <thead>
                <tr className="text-left text-[11px] uppercase tracking-wide text-muted">
                  <th className="px-3.5 py-2 font-medium">Secret</th>
                  <th className="px-3 py-2 font-medium">Value comes from</th>
                  <th className="px-3 py-2 font-medium">Delivered</th>
                  <th className="px-3 py-2 font-medium">Used by</th>
                  <th className="px-3 py-2 font-medium">Copies on hosts</th>
                </tr>
              </thead>
              <tbody>
                {named.map((secret) => {
                  const use = usage.find((u) => u.secret_id === secret.id);
                  const entry = overview?.entries.find(
                    (e) =>
                      e.owner.kind === "named_secret" &&
                      e.owner.id === secret.id,
                  );
                  const opened = open === secret.id;
                  return (
                    <Fragment key={secret.id}>
                      <tr className="border-line-soft border-t align-top">
                        <td className="px-3.5 py-2.5">
                          <button
                            type="button"
                            className="flex flex-col items-start text-left"
                            aria-expanded={opened}
                            onClick={() => setOpen(opened ? null : secret.id)}
                          >
                            <span className="mono text-[12.5px] text-link">
                              {secret.env_name}
                            </span>
                            <span className="text-[12px] text-muted">
                              {secret.label}
                            </span>
                          </button>
                        </td>
                        <td className="px-3 py-2.5 text-[12.5px] text-fg-2">
                          {sourceLabel(secret.credential_source)}
                        </td>
                        <td className="px-3 py-2.5 text-[12.5px] text-fg-2">
                          {secret.as_file ? "A file" : "A value"}
                        </td>
                        <td className="px-3 py-2.5 text-[12.5px] text-fg-2">
                          {use?.routines.length
                            ? `${use.routines.length} ${use.routines.length === 1 ? "routine" : "routines"}`
                            : "Runs only"}
                        </td>
                        <td className="px-3 py-2.5">
                          <span className="flex flex-wrap gap-1">
                            {use?.copies.length ? (
                              use.copies.map((c) => (
                                <span
                                  key={c.host_id}
                                  className="state-pill"
                                  data-tone={
                                    c.state === "shared" ? "green" : "amber"
                                  }
                                >
                                  <span className="dot" />
                                  {c.host_name}
                                  {c.state === "shared"
                                    ? ""
                                    : c.state === "failed"
                                      ? " · did not finish"
                                      : " · out of date"}
                                </span>
                              ))
                            ) : (
                              <span className="text-[12.5px] text-muted">
                                None
                              </span>
                            )}
                          </span>
                        </td>
                      </tr>
                      {opened && entry && (
                        <tr>
                          <td colSpan={5} className="px-3 pb-3">
                            <SecretRow
                              entry={entry}
                              onOverview={setOverview}
                              onEdit={() => setEditing(secret)}
                              usage={use}
                              onCopiesChanged={load}
                              onDelete={async () => {
                                await ipc.deleteNamedSecret(
                                  secret.id,
                                  secret.version,
                                );
                                setOpen(null);
                                load();
                              }}
                            />
                          </td>
                        </tr>
                      )}
                    </Fragment>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </section>
      <section
        aria-label="Accounts and database passwords"
        className="flex flex-col gap-2"
      >
        <h2 className="m-0 mt-2 text-[13px] font-semibold">
          Accounts, databases and agents
        </h2>
        {others.length === 0 && overview && (
          <p className="m-0 text-[12.5px] text-muted">
            No account or connection has a secret yet.
          </p>
        )}
        {others.map((entry) => (
          <SecretRow
            key={ownerKey(entry.owner)}
            entry={entry}
            onOverview={setOverview}
          />
        ))}
      </section>
      {!overview && !error && (
        <div className="text-[12px] text-muted">Loading…</div>
      )}
    </div>
  );
}

function SecretRow({
  entry,
  onOverview,
  onEdit,
  onDelete,
  usage,
  onCopiesChanged,
}: {
  entry: SecretEntry;
  onOverview: (o: SecretsOverview) => void;
  /** A named secret: the routines that use it and its copies on hosts. */
  usage?: NamedSecretUsage;
  onCopiesChanged?: () => void;
  /** A named secret can be changed here. */
  onEdit?: () => void;
  onDelete?: () => Promise<void>;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshed, setRefreshed] = useState(false);
  const [tested, setTested] = useState<string | null>(null);
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
          {onEdit && (
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={onEdit}
            >
              Edit…
            </button>
          )}
          {onDelete && (
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={() =>
                void act(async () => {
                  await onDelete();
                  return null;
                })
              }
            >
              Delete
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
      {usage && (
        <div className="flex items-center gap-2 text-[12px]">
          <button
            type="button"
            className="btn btn-sm"
            disabled={busy}
            onClick={() =>
              void act(async () => {
                setTested(null);
                const t = await ipc.testNamedSecret(usage.secret_id);
                setTested(
                  `Printed ${t.chars} characters in ${(t.millis / 1000).toFixed(1)} s (never shown).`,
                );
                return null;
              })
            }
          >
            {busy ? "Testing…" : "Test"}
          </button>
          <span className="text-muted">
            {tested ??
              "Reads the source as a run would and says only how long the value is."}
          </span>
        </div>
      )}
      {usage && (
        <div className="flex flex-col gap-1 text-[12px]">
          <span className="text-fg-2">
            Used by{" "}
            {usage.routines.length ? usage.routines.join(", ") : "no routine"}
          </span>
          {usage.copies.map((c) => (
            <div key={c.host_id} className="flex items-center gap-2">
              <span
                className={c.state === "shared" ? "text-fg-2" : "text-conflict"}
              >
                {c.state === "shared"
                  ? `On ${c.host_name}`
                  : c.state === "failed"
                    ? `${c.host_name}: did not finish`
                    : `Out of date on ${c.host_name}: its runs get the old value`}
              </span>
              {c.shared_at && (
                <span className="text-muted">
                  sent {new Date(c.shared_at).toLocaleString()}
                </span>
              )}
              <span className="flex-1" />
              {c.state !== "shared" && (
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={busy}
                  onClick={() =>
                    void act(async () => {
                      await ipc.shareSecretWithHost(usage.secret_id, c.host_id);
                      onCopiesChanged?.();
                      return null;
                    })
                  }
                >
                  Send Again
                </button>
              )}
              <button
                type="button"
                className="btn btn-sm"
                disabled={busy}
                onClick={() =>
                  void act(async () => {
                    await ipc.removeSecretFromHost(usage.secret_id, c.host_id);
                    onCopiesChanged?.();
                    return null;
                  })
                }
              >
                Remove from {c.host_name}
              </button>
            </div>
          ))}
        </div>
      )}
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
