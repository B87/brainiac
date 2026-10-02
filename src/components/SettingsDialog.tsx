import { useEffect, useState } from "react";
import {
  ACCESS_CHOICES,
  accessSummary,
  claudeCommand,
  connectedText,
} from "../lib/agent";
import { shortPath } from "../lib/format";
import {
  type AgentAccess,
  type AgentAccessStatus,
  type AppSnapshot,
  errorMessage,
  ipc,
  type VaultState,
} from "../lib/ipc";
import Dialog from "./Dialog";
import VaultSetup from "./VaultSetup";

/** How often the connected agents are counted while Settings is open. */
const AGENT_POLL_MS = 2000;

/** Settings: the vault, note identity, search, agent access, and backups. */
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
  const [agent, setAgent] = useState<AgentAccessStatus | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let alive = true;
    const load = () =>
      ipc
        .getAgentAccessStatus()
        .then((s) => alive && setAgent(s))
        .catch(() => {});
    void load();
    const timer = setInterval(load, AGENT_POLL_MS);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  const setAccess = async (access: AgentAccess) => {
    try {
      await ipc.updateSettings({ ...settings, agent_access: access });
      onChanged();
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  const command = agent ? claudeCommand(agent.executable) : null;
  const copyCommand = () => {
    if (!command) return;
    void navigator.clipboard
      ?.writeText(command)
      .then(() => setCopied(true))
      .catch(() => {});
  };

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
          <h3 className="section-label m-0">Agent access</h3>
          <fieldset aria-label="Agent access" className="seg self-start">
            {ACCESS_CHOICES.map((choice) => (
              <button
                key={choice.value}
                type="button"
                aria-pressed={settings.agent_access === choice.value}
                onClick={() => void setAccess(choice.value)}
              >
                {choice.label}
              </button>
            ))}
          </fieldset>
          <p className="m-0 text-[12.5px] text-fg-2">
            {accessSummary(settings.agent_access)}
          </p>
          {agent && (
            <span className="text-[12px] text-muted" aria-live="polite">
              {connectedText(agent.connections, settings.agent_access)}
            </span>
          )}
          {agent?.problem && (
            <div role="alert" className="text-[12.5px] text-conflict">
              Agents cannot connect: {agent.problem}
            </div>
          )}
          {command && (
            <>
              <span className="text-[12px] text-muted">
                Add Brainiac to Claude Code from a terminal:
              </span>
              <div className="flex items-center gap-2 rounded-md border bg-panel px-2.5 py-2">
                <code className="mono min-w-0 flex-1 select-all break-all text-[12px]">
                  {command}
                </code>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={copyCommand}
                >
                  {copied ? "Copied" : "Copy"}
                </button>
              </div>
              <span className="text-[12px] text-muted">
                Or install the Brainiac plugin, which also teaches Claude Code
                how to use it:{" "}
                <span className="mono select-all">
                  /plugin marketplace add B87/brainiac
                </span>
              </span>
            </>
          )}
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
