import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useId, useState } from "react";
import {
  accessSummary,
  PROVIDERS,
  type Provider,
  tokenSummary,
} from "../lib/accounts";
import { errorMessage, type ForgeAccountSlot, ipc } from "../lib/ipc";
import {
  commandPreview,
  draftOf,
  pendingLabel,
  type SourceDraft,
  type SourceKind,
  sourceLabel,
  sourceOf,
} from "../lib/secrets";
import SecretSourceFields from "./SecretSourceFields";

/** Settings → Accounts: one GitHub and one Bitbucket Cloud account (SPEC.md, Accounts). */
export default function AccountsSection({
  onChanged,
}: {
  /** An account was added, replaced, or removed. */
  onChanged: () => void;
}) {
  const [slots, setSlots] = useState<ForgeAccountSlot[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    ipc
      .listForgeAccounts()
      .then((s) => alive && setSlots(s))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, []);

  const replace = (slot: ForgeAccountSlot) => {
    setSlots(
      (all) => all?.map((s) => (s.kind === slot.kind ? slot : s)) ?? null,
    );
    onChanged();
  };
  const replaceAll = (all: ForgeAccountSlot[]) => {
    setSlots(all);
    onChanged();
  };
  const reload = () =>
    ipc
      .listForgeAccounts()
      .then(replaceAll)
      .catch((e) => setError(errorMessage(e)));

  return (
    <div className="flex flex-col gap-2">
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}
      {PROVIDERS.map((p) => {
        const slot = slots?.find((s) => s.kind === p.kind);
        return (
          <AccountRow
            key={p.kind}
            provider={p}
            slot={slot ?? null}
            loading={slots === null && !error}
            onSlot={replace}
            onSlots={replaceAll}
            onReload={reload}
          />
        );
      })}
    </div>
  );
}

type Mode =
  | { kind: "idle" }
  | { kind: "form"; useKeychain: boolean }
  | { kind: "read_only"; login: string; missing: string[] }
  | { kind: "confirm_remove" };

function AccountRow({
  provider: p,
  slot,
  loading,
  onSlot,
  onSlots,
  onReload,
}: {
  provider: Provider;
  slot: ForgeAccountSlot | null;
  loading: boolean;
  onSlot: (slot: ForgeAccountSlot) => void;
  onSlots: (slots: ForgeAccountSlot[]) => void;
  onReload: () => Promise<void>;
}) {
  const id = useId();
  const account = slot?.account ?? null;
  const [mode, setMode] = useState<Mode>({ kind: "idle" });
  const [token, setToken] = useState("");
  const [email, setEmail] = useState(account?.email ?? "");
  const [source, setSource] = useState<SourceDraft>(() =>
    draftOf(account?.token_source, "store"),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // A test result is for the form it ran with; editing makes it stale.
  const [tested, setTested] = useState<{ key: string; text: string } | null>(
    null,
  );
  const owner = { kind: "forge_account" as const, provider: p.kind };

  const startForm = (fromKeychain: boolean) => {
    setEmail(account?.email ?? "");
    setSource(
      fromKeychain
        ? draftOf(null, "store")
        : draftOf(account?.token_source, "store"),
    );
    setError(null);
    setNotice(null);
    setTested(null);
    setMode({ kind: "form", useKeychain: fromKeychain });
  };

  const reset = () => {
    setMode({ kind: "idle" });
    setToken("");
    setError(null);
    setTested(null);
  };

  const formRequest = (readOnly: boolean, useKeychain: boolean) => ({
    kind: p.kind,
    source: sourceOf(source),
    token:
      source.kind === "store" && !useKeychain && token.trim() ? token : null,
    email: p.needsEmail ? email : null,
    read_only: readOnly,
  });
  const formKey = JSON.stringify(
    formRequest(false, mode.kind === "form" && mode.useKeychain),
  );

  const test = async (useKeychain: boolean) => {
    setBusy(true);
    setError(null);
    setTested(null);
    try {
      const result = await ipc.testForgeAccount(
        formRequest(false, useKeychain),
      );
      setTested({
        key: formKey,
        text: result.missing.length
          ? `Belongs to ${result.login}; read only, missing ${result.missing.join(", ")}.`
          : `Belongs to ${result.login}.`,
      });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  /** Allow This Source, or Retry a cleanup or removal, then show the result. */
  const act = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await onReload();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const save = async (readOnly: boolean, useKeychain: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const outcome = await ipc.saveForgeAccount(
        formRequest(readOnly, useKeychain),
      );
      if (outcome.outcome === "read_only") {
        setMode({
          kind: "read_only",
          login: outcome.login,
          missing: outcome.missing,
        });
        return;
      }
      onSlot({
        kind: p.kind,
        account: outcome.account,
        keychain_token: false,
      });
      reset();
      setNotice(outcome.warning);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    setError(null);
    try {
      onSlots(await ipc.removeForgeAccount(p.kind));
      reset();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const useKeychain = mode.kind === "form" && mode.useKeychain;
  const pastes = source.kind === "store" && !useKeychain;
  const sourceReady =
    source.kind === "store"
      ? useKeychain ||
        token.trim() !== "" ||
        account?.token_source.kind === "store"
      : source.kind === "environment"
        ? source.name.trim() !== ""
        : source.program.trim() !== "";
  const canSubmit =
    !busy &&
    (!pastes ||
      token.trim() !== "" ||
      account?.token_source.kind === "store") &&
    sourceReady &&
    (!p.needsEmail || email.trim() !== "");

  return (
    <div className="flex flex-col gap-2 rounded-[10px] border bg-app px-3.5 py-3">
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1">
          <div className="font-medium">{p.name}</div>
          {loading ? (
            <div className="text-[12px] text-muted">Loading…</div>
          ) : account ? (
            <div className="text-[12px] text-fg-2">
              <span className="mono">{account.login}</span>
              {account.display_name && ` (${account.display_name})`}
              {account.email && ` · ${account.email}`}
              {" · "}
              {tokenSummary(account)}
              {account.token_source.kind !== "store" &&
                ` · from ${sourceLabel(account.token_source)}`}
            </div>
          ) : slot?.keychain_token ? (
            <div className="text-[12px] text-fg-2">
              A token is in the Keychain item{" "}
              <span className="mono">{p.keychainItem}</span>.
            </div>
          ) : (
            <div className="text-[12px] text-muted">No account.</div>
          )}
        </div>
        {mode.kind === "idle" && !loading && (
          <div className="flex shrink-0 gap-2">
            {account ? (
              <>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => startForm(false)}
                >
                  Change Token…
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => setMode({ kind: "confirm_remove" })}
                >
                  Remove
                </button>
              </>
            ) : slot?.keychain_token ? (
              <>
                <button
                  type="button"
                  className="btn btn-sm btn-primary"
                  onClick={() =>
                    p.needsEmail ? startForm(true) : void save(false, true)
                  }
                  disabled={busy}
                >
                  {busy ? "Checking…" : "Use This Token"}
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => startForm(false)}
                >
                  Paste Another…
                </button>
              </>
            ) : (
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => startForm(false)}
              >
                Add Account…
              </button>
            )}
          </div>
        )}
      </div>

      {account && mode.kind === "idle" && (
        <p className="m-0 text-[12px] text-fg-2">{accessSummary(account)}</p>
      )}

      {account?.credential.needs_approval && mode.kind === "idle" && (
        <div className="flex flex-col gap-2" role="note">
          <p className="m-0 text-[12.5px]">
            Restored from a backup: Brainiac does not read its token until you
            allow it. It reads{" "}
            <span className="mono">{sourceLabel(account.token_source)}</span>{" "}
            and sends the token only to {p.name}.
          </p>
          {account.token_source.kind === "command" && (
            <>
              <code className="mono selectable break-all rounded-md border bg-header px-2 py-1 text-[11.5px]">
                {commandPreview(
                  account.token_source.program,
                  account.token_source.args,
                )}
              </code>
              <p className="m-0 text-[12px] text-muted">
                The program runs with your permissions, without a shell. Allow
                it only if you recognize it and its arguments.
              </p>
            </>
          )}
          <button
            type="button"
            className="btn btn-sm self-start"
            disabled={busy}
            onClick={() =>
              void act(() =>
                ipc.approveSecretSource(owner, account.credential.revision),
              )
            }
          >
            Allow This Source
          </button>
        </div>
      )}

      {account?.credential.pending && mode.kind === "idle" && (
        <div className="flex items-center gap-2" role="note">
          <p className="m-0 flex-1 text-[12.5px]">
            {pendingLabel(account.credential.pending)}
          </p>
          {account.credential.pending !== "save" && (
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={() => void act(() => ipc.retryCredentialCleanup(owner))}
            >
              Retry
            </button>
          )}
        </div>
      )}

      {notice && mode.kind === "idle" && (
        <p role="note" className="m-0 text-[12.5px] text-conflict">
          {notice}
        </p>
      )}

      {mode.kind === "form" && (
        <form
          className="flex flex-col gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (canSubmit) void save(false, useKeychain);
          }}
        >
          {p.needsEmail && (
            <label className="flex flex-col gap-1" htmlFor={`${id}-email`}>
              <span className="text-[12px]">Atlassian account email</span>
              <input
                id={`${id}-email`}
                className="text-input"
                type="email"
                autoComplete="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            </label>
          )}
          {!useKeychain && (
            <label className="flex flex-col gap-1" htmlFor={`${id}-source`}>
              <span className="text-[12px]">Token from</span>
              <select
                id={`${id}-source`}
                className="text-input"
                value={source.kind}
                onChange={(e) =>
                  setSource({ ...source, kind: e.target.value as SourceKind })
                }
              >
                <option value="store">Paste it: kept in the Keychain</option>
                <option value="command">
                  A command, such as gh auth token
                </option>
                <option value="environment">An environment variable</option>
              </select>
            </label>
          )}
          {!useKeychain && source.kind !== "store" && (
            <SecretSourceFields
              draft={source}
              onChange={setSource}
              what="token"
            />
          )}
          {pastes && (
            <label className="flex flex-col gap-1" htmlFor={`${id}-token`}>
              <span className="text-[12px]">{p.tokenLabel}</span>
              <input
                id={`${id}-token`}
                className="text-input mono"
                type="password"
                autoComplete="off"
                spellCheck={false}
                value={token}
                onChange={(e) => setToken(e.target.value)}
              />
            </label>
          )}
          {pastes && account?.token_source.kind === "store" && (
            <span className="text-[12px] text-muted">
              Leave it empty to check the token already in the Keychain again.
            </span>
          )}
          {!useKeychain && (
            <span className="text-[12px] text-muted">
              {p.needs}{" "}
              <button
                type="button"
                className="text-link hover:underline"
                onClick={() => void openUrl(p.tokenUrl).catch(() => {})}
              >
                Create a token
              </button>
            </span>
          )}
          <div className="flex gap-2">
            <button
              type="submit"
              className="btn btn-sm btn-primary"
              disabled={!canSubmit}
            >
              {busy
                ? "Checking…"
                : account
                  ? "Check and Replace"
                  : "Check and Add"}
            </button>
            <button
              type="button"
              className="btn btn-sm"
              disabled={!canSubmit}
              onClick={() => void test(useKeychain)}
            >
              Test
            </button>
            <button
              type="button"
              className="btn btn-sm"
              onClick={reset}
              disabled={busy}
            >
              Cancel
            </button>
            {tested?.key === formKey && (
              <span className="self-center text-[12px] text-clean">
                {tested.text}
              </span>
            )}
          </div>
        </form>
      )}

      {mode.kind === "read_only" && (
        <div className="flex flex-col gap-2" role="alert">
          <p className="m-0 text-[12.5px]">
            This token belongs to <span className="mono">{mode.login}</span> and
            can read pull requests, but not comment, review, or merge. It is
            missing {mode.missing.join(", ")}.
          </p>
          <div className="flex gap-2">
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={() => void save(true, token.trim() === "")}
            >
              {busy ? "Saving…" : "Save as Read-Only"}
            </button>
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={reset}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {mode.kind === "confirm_remove" && (
        <div className="flex flex-col gap-2" role="alert">
          <p className="m-0 text-[12.5px]">
            Remove the {p.name} account? Brainiac's Keychain item{" "}
            <span className="mono">{p.keychainItem}</span> is deleted
            {account && account.token_source.kind !== "store"
              ? ` if there is one; ${sourceLabel(account.token_source)} is not changed`
              : ""}
            , and its pull requests are no longer shown.
          </p>
          <div className="flex gap-2">
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={() => void remove()}
            >
              {busy ? "Removing…" : "Remove Account"}
            </button>
            <button
              type="button"
              className="btn btn-sm"
              disabled={busy}
              onClick={reset}
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
    </div>
  );
}
