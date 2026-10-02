import { useState } from "react";
import { shortPath } from "../lib/format";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
  type VaultState,
} from "../lib/ipc";
import Dialog from "./Dialog";
import VaultSetup from "./VaultSetup";

/** Settings: the vault, note identity, search, and backups. */
export default function SettingsDialog({
  snapshot,
  vault,
  onVault,
  onClose,
  onExport,
  onRestore,
  onChanged,
}: {
  snapshot: AppSnapshot;
  vault: VaultState | null;
  onVault: (state: VaultState) => void;
  onClose: () => void;
  onExport: () => void;
  onRestore: () => void;
  onChanged: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [rebuilding, setRebuilding] = useState(false);
  const settings = snapshot.settings;

  const setWriteIds = async (on: boolean) => {
    try {
      await ipc.updateSettings({ ...settings, write_note_ids: on });
      onChanged();
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  const rebuild = async () => {
    setRebuilding(true);
    try {
      await ipc.rebuildSearch();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setRebuilding(false);
    }
  };

  return (
    <Dialog
      title="Settings"
      onClose={onClose}
      width={560}
      footer={
        <button type="button" className="btn btn-primary" onClick={onClose}>
          Done
        </button>
      }
    >
      <div className="flex flex-col gap-5">
        <section className="flex flex-col gap-2">
          <h3 className="section-label m-0">Vault</h3>
          {vault?.vault ? (
            <div className="flex items-center gap-2">
              <span
                className="mono min-w-0 flex-1 truncate"
                title={vault.vault.root_path}
              >
                {shortPath(vault.vault.root_path)}
              </span>
              <button
                type="button"
                className="btn btn-sm"
                onClick={() =>
                  void ipc
                    .revealVaultPath()
                    .catch((e) => setError(errorMessage(e)))
                }
              >
                Reveal in Finder
              </button>
            </div>
          ) : (
            <span className="text-muted">No vault chosen.</span>
          )}
          <VaultSetup compact onDone={onVault} />
          <span className="text-[12px] text-muted">
            Choosing another folder keeps the notes Brainiac knew in the old one
            as missing, with their tasks and links.
          </span>
        </section>
        <section className="flex flex-col gap-2">
          <h3 className="section-label m-0">Notes</h3>
          <label className="flex items-start gap-2">
            <input
              type="checkbox"
              className="mt-0.5"
              checked={settings.write_note_ids}
              onChange={(e) => void setWriteIds(e.target.checked)}
            />
            <span>
              Write <span className="mono">brainiac_id</span> into a note when
              it first gets a task or a repository link
              <span className="block text-[12px] text-muted">
                So renaming the note outside Brainiac keeps its context. Opening
                or indexing a note never writes it.
              </span>
            </span>
          </label>
        </section>
        <section className="flex flex-col gap-2">
          <h3 className="section-label m-0">Search</h3>
          <div className="flex items-center gap-2">
            <span className="flex-1 text-[12.5px] text-fg-2">
              {vault?.index.state === "indexing"
                ? `Indexing notes: ${vault.index.done} of ${vault.index.total}.`
                : vault?.index.state === "unavailable"
                  ? "Search is unavailable."
                  : "The search index is rebuilt from the vault whenever needed."}
            </span>
            <button
              type="button"
              className="btn btn-sm"
              disabled={rebuilding}
              onClick={() => void rebuild()}
            >
              {rebuilding ? "Rebuilding…" : "Rebuild Index"}
            </button>
          </div>
        </section>
        <section className="flex flex-col gap-2">
          <h3 className="section-label m-0">Backup</h3>
          <p className="m-0 text-[12.5px] text-fg-2">
            Brainiac snapshots its data before each upgrade and daily, on this
            Mac. A complete backup is an export kept on another device or backup
            system.
          </p>
          <div className="flex gap-2">
            <button type="button" className="btn" onClick={onExport}>
              Export…
            </button>
            <button type="button" className="btn" onClick={onRestore}>
              Restore from Export…
            </button>
          </div>
        </section>
        {error && (
          <div role="alert" className="text-conflict">
            {error}
          </div>
        )}
      </div>
    </Dialog>
  );
}
