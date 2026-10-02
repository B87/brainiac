import { open, save } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { errorMessage, ipc, type VaultState } from "../lib/ipc";
import { NoteIcon } from "./icons";

/**
 * Choose the vault (SPEC.md, The vault): an existing folder of Markdown
 * notes, edited where they are, or a new empty one.
 */
export default function VaultSetup({
  onDone,
  compact = false,
}: {
  onDone: (state: VaultState) => void;
  /** A smaller version for Settings. */
  compact?: boolean;
}) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const choose = async (create: boolean) => {
    setError(null);
    try {
      const path = create
        ? await save({ title: "Create a New Vault", defaultPath: "Notes" })
        : await open({
            directory: true,
            multiple: false,
            title: "Choose the Vault Folder",
          });
      if (typeof path !== "string") return;
      setBusy(true);
      onDone(await ipc.selectVault(path, create));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const buttons = (
    <div className="flex gap-2">
      <button
        type="button"
        className="btn btn-primary"
        disabled={busy}
        onClick={() => void choose(false)}
      >
        Choose Folder…
      </button>
      <button
        type="button"
        className="btn"
        disabled={busy}
        onClick={() => void choose(true)}
      >
        Create a New Vault…
      </button>
    </div>
  );
  if (compact)
    return (
      <div className="flex flex-col gap-2">
        {buttons}
        {error && (
          <p role="alert" className="m-0 text-conflict">
            {error}
          </p>
        )}
      </div>
    );
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center p-8">
      <div className="flex max-w-[460px] flex-col items-center gap-4 text-center">
        <NoteIcon size={32} className="text-muted" />
        <h1 className="m-0 text-[18px] font-semibold">Set up your vault</h1>
        <p className="m-0 text-fg-2">
          Your notes are a folder of ordinary Markdown files. Brainiac edits
          them where they are and never moves or converts them, so they stay
          usable in any editor. The folder may also be a Git repository;
          Brainiac never touches its Git state.
        </p>
        {buttons}
        <p className="m-0 text-[12px] text-muted">
          Today and Tasks work without a vault.
        </p>
        {error && (
          <p role="alert" className="m-0 text-conflict">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
