import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useId, useState } from "react";
import {
  accessSummary,
  PROVIDERS,
  type Provider,
  tokenSummary,
} from "../lib/accounts";
import { errorMessage, type ForgeAccountSlot, ipc } from "../lib/ipc";

/** Settings → Accounts: one GitHub and one Bitbucket Cloud account (SPEC.md, Accounts). */
export default function AccountsSection() {
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

  const replace = (slot: ForgeAccountSlot) =>
    setSlots(
      (all) => all?.map((s) => (s.kind === slot.kind ? slot : s)) ?? null,
    );

  return (
    <section className="flex flex-col gap-2">
      <h3 className="section-label m-0">Accounts</h3>
      <p className="m-0 text-[12.5px] text-fg-2">
        For pull requests. Tokens are kept in the macOS Keychain and sent only
        to the service they belong to.
      </p>
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
            onSlots={setSlots}
          />
        );
      })}
    </section>
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
}: {
  provider: Provider;
  slot: ForgeAccountSlot | null;
  loading: boolean;
  onSlot: (slot: ForgeAccountSlot) => void;
  onSlots: (slots: ForgeAccountSlot[]) => void;
}) {
  const id = useId();
  const account = slot?.account ?? null;
  const [mode, setMode] = useState<Mode>({ kind: "idle" });
  const [token, setToken] = useState("");
  const [email, setEmail] = useState(account?.email ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const startForm = (fromKeychain: boolean) => {
    setEmail(account?.email ?? "");
    setError(null);
    setMode({ kind: "form", useKeychain: fromKeychain });
  };

  const reset = () => {
    setMode({ kind: "idle" });
    setToken("");
    setError(null);
  };

  const save = async (readOnly: boolean, useKeychain: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const outcome = await ipc.saveForgeAccount({
        kind: p.kind,
        token: useKeychain ? null : token,
        email: p.needsEmail ? email : null,
        read_only: readOnly,
      });
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
  const canSubmit =
    !busy &&
    (useKeychain || token.trim() !== "") &&
    (!p.needsEmail || email.trim() !== "");

  return (
    <div className="flex flex-col gap-2 rounded-md border bg-panel px-3 py-2.5">
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
                  Replace Token…
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
              onClick={reset}
              disabled={busy}
            >
              Cancel
            </button>
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
            Remove the {p.name} account? Its token is deleted from the Keychain
            item <span className="mono">{p.keychainItem}</span>, and its pull
            requests are no longer shown.
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
