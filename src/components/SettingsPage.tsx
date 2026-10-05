import { type ReactNode, useCallback, useRef, useState } from "react";
import { ACCESS_CHOICES } from "../lib/agent";
import { errorMessage, ipc, type Settings, type VaultState } from "../lib/ipc";
import { SECTIONS, type SettingsSection, sectionLabel } from "../lib/settings";
import AgentRunsPane from "./AgentRunsPane";
import {
  ArchiveIcon,
  BranchIcon,
  ChevronLeft,
  DatabaseIcon,
  KeyIcon,
  NoteIcon,
  PersonIcon,
  PlayIcon,
  SlidersIcon,
  TerminalIcon,
} from "./icons";
import {
  AccountsPane,
  AgentsPane,
  BackupPane,
  DatabasesPane,
  GeneralPane,
  NotesPane,
  RepositoriesPane,
  SecretsPane,
} from "./SettingsPanes";

const ICONS: Record<SettingsSection, ReactNode> = {
  general: <SlidersIcon size={16} />,
  notes: <NoteIcon size={16} />,
  repositories: <BranchIcon size={16} />,
  databases: <DatabaseIcon size={16} />,
  accounts: <PersonIcon size={16} />,
  secrets: <KeyIcon size={16} />,
  agents: <TerminalIcon size={16} />,
  runs: <PlayIcon size={16} />,
  backup: <ArchiveIcon size={16} />,
};

/** Saves a change to the settings; true when it was saved. */
export type SaveSettings = (patch: Partial<Settings>) => Promise<boolean>;

/**
 * Settings as a page of the main window (SPEC.md, Main window — v0.2): its
 * sidebar lists the sections in place of the app's, and Back returns to the
 * view it was opened from. Each change is saved as it is made.
 */
export default function SettingsPage({
  settings: initial,
  section,
  vault,
  onSection,
  onBack,
  onVault,
  onChanged,
  onExport,
  onRestore,
  top,
}: {
  settings: Settings;
  section: SettingsSection;
  vault: VaultState | null;
  onSection: (section: SettingsSection) => void;
  onBack: () => void;
  onVault: (state: VaultState) => void;
  /** A setting or an account changed: the app reloads its snapshot. */
  onChanged: () => void;
  onExport: () => void;
  onRestore: () => void;
  /** The app's banners: an update, Git missing, or an error such as a failed export. */
  top?: ReactNode;
}) {
  const [error, setError] = useState<string | null>(null);
  // Saves run one after another. What is shown is the last saved settings
  // with the changes still being saved on top, so a refused change puts back
  // only itself and never a change made after it.
  const confirmed = useRef(initial);
  const pending = useRef<Partial<Settings>[]>([]);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const [shown, setShown] = useState(initial);
  const show = useCallback(
    () => setShown(Object.assign({}, confirmed.current, ...pending.current)),
    [],
  );

  const save: SaveSettings = useCallback(
    (patch) => {
      pending.current.push(patch);
      show();
      const run = queue.current.then(async () => {
        try {
          confirmed.current = await ipc.updateSettings({
            ...confirmed.current,
            ...patch,
          });
          setError(null);
          onChanged();
          return true;
        } catch (e) {
          setError(errorMessage(e));
          return false;
        } finally {
          pending.current.splice(pending.current.indexOf(patch), 1);
          show();
        }
      });
      queue.current = run;
      return run;
    },
    [show, onChanged],
  );

  const accessLabel = ACCESS_CHOICES.find(
    (c) => c.value === shown.agent_access,
  )?.label;

  return (
    <div className="flex min-h-0 flex-1">
      <nav
        aria-label="Settings"
        className="flex w-58 shrink-0 flex-col border-r bg-sidebar"
      >
        {/* Room for the window's traffic lights; dragging here moves the window. */}
        <div data-tauri-drag-region className="h-12 shrink-0" />
        <div className="flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2.5 py-1">
          <button type="button" className="side-row text-fg-2" onClick={onBack}>
            <ChevronLeft size={14} />
            Back
          </button>
          <h2 className="section-label m-0 px-2 pt-3 pb-1">Settings</h2>
          {SECTIONS.map((s) => {
            const current = s.id === section;
            return (
              <button
                key={s.id}
                type="button"
                className="side-row"
                aria-current={current}
                onClick={() => onSection(s.id)}
              >
                <span className={current ? "text-accent" : "text-muted"}>
                  {ICONS[s.id]}
                </span>
                <span className="min-w-0 flex-1 truncate">{s.label}</span>
                {s.id === "agents" && accessLabel && (
                  <span className="text-[11px] font-normal text-muted">
                    {accessLabel}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      </nav>
      <main className="flex min-w-0 flex-1 flex-col bg-app">
        {top}
        <div
          data-tauri-drag-region
          className="flex h-12 shrink-0 items-center border-b bg-header px-4"
        >
          <h1 className="m-0 text-[14px] font-semibold">
            {sectionLabel(section)}
          </h1>
        </div>
        <div className="min-h-0 flex-1 overflow-auto bg-header">
          <div className="mx-auto flex max-w-160 flex-col gap-5 px-6 pt-5 pb-8">
            {error && (
              <div role="alert" className="text-[12.5px] text-conflict">
                {error}
              </div>
            )}
            {section === "general" && (
              <GeneralPane settings={shown} save={save} />
            )}
            {section === "notes" && (
              <NotesPane
                settings={shown}
                save={save}
                vault={vault}
                onVault={onVault}
                onError={setError}
              />
            )}
            {section === "repositories" && (
              <RepositoriesPane settings={shown} save={save} />
            )}
            {section === "databases" && (
              <DatabasesPane settings={shown} save={save} />
            )}
            {section === "accounts" && <AccountsPane onChanged={onChanged} />}
            {section === "secrets" && <SecretsPane />}
            {section === "agents" && (
              <AgentsPane settings={shown} save={save} />
            )}
            {section === "runs" && <AgentRunsPane />}
            {section === "backup" && (
              <BackupPane onExport={onExport} onRestore={onRestore} />
            )}
          </div>
        </div>
      </main>
    </div>
  );
}
