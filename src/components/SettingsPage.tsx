import { type ReactNode, useCallback, useRef, useState } from "react";
import { ACCESS_CHOICES } from "../lib/agent";
import { errorMessage, ipc, type Settings, type VaultState } from "../lib/ipc";
import {
  SECTIONS,
  type SettingsGroup,
  type SettingsSection,
  sectionLabel,
} from "../lib/settings";
import AgentRunsPane from "./AgentRunsPane";
import {
  ExplainAgentPane,
  ExplainConceptsPane,
  ExplainRepositoriesPane,
  ExplainStoredPane,
} from "./ExplanationsPane";
import {
  ArchiveIcon,
  BoxIcon,
  BranchIcon,
  BulbIcon,
  CheckIcon,
  ChevronLeft,
  DatabaseIcon,
  KeyIcon,
  LockIcon,
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
  "explain-agent": <BulbIcon size={16} />,
  "explain-repositories": <LockIcon size={16} />,
  "explain-concepts": <CheckIcon size={16} />,
  "explain-stored": <BoxIcon size={16} />,
};

/** The sidebar's headings, in order. */
const GROUPS: { id: SettingsGroup; label: string }[] = [
  { id: "settings", label: "Settings" },
  { id: "explanations", label: "Explanations" },
];

/** Sections whose list takes the page's full width. */
const WIDE: SettingsSection[] = ["explain-concepts", "explain-stored"];

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
  host,
  onHost,
  profile,
  onProfile,
  onOpenRun,
}: {
  settings: Settings;
  section: SettingsSection;
  /** Settings → Agents: the run host whose page is open. */
  host?: string;
  onHost: (host: string | null) => void;
  /** Settings → Agents: the profile whose page is open. */
  profile?: string;
  onProfile: (profile: string | null) => void;
  onOpenRun: (runId: string) => void;
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
  // Where a page draws what goes beside its title, such as a list's count.
  const [titleSlot, setTitleSlot] = useState<HTMLSpanElement | null>(null);
  const wide = WIDE.includes(section);
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
          {GROUPS.map((g) => (
            <section key={g.id} className="flex flex-col gap-px">
              <h2 className="section-label m-0 px-2 pt-3 pb-1">{g.label}</h2>
              {SECTIONS.filter((s) => (s.group ?? "settings") === g.id).map(
                (s) => {
                  const current = s.id === section;
                  return (
                    <button
                      key={s.id}
                      type="button"
                      className="side-row"
                      aria-current={current}
                      // The row is named by its section; a state shown beside
                      // it, such as Agent Access' Off, describes it.
                      aria-label={s.label}
                      aria-describedby={
                        s.id === "agents" && accessLabel
                          ? "settings-access-label"
                          : undefined
                      }
                      onClick={() => onSection(s.id)}
                    >
                      <span className={current ? "text-accent" : "text-muted"}>
                        {ICONS[s.id]}
                      </span>
                      <span className="min-w-0 flex-1 truncate">{s.label}</span>
                      {s.id === "agents" && accessLabel && (
                        <span
                          id="settings-access-label"
                          className="text-[11px] font-normal text-muted"
                        >
                          {accessLabel}
                        </span>
                      )}
                    </button>
                  );
                },
              )}
            </section>
          ))}
        </div>
      </nav>
      <main className="flex min-w-0 flex-1 flex-col bg-app">
        {top}
        <div
          data-tauri-drag-region
          className="flex h-12 shrink-0 items-center gap-2 border-b bg-header px-4"
        >
          <h1 className="m-0 text-[14px] font-semibold">
            {sectionLabel(section)}
          </h1>
          <span
            ref={setTitleSlot}
            className="flex min-w-0 flex-1 items-center gap-2 text-[13px]"
          />
        </div>
        <div className="min-h-0 flex-1 overflow-auto bg-header">
          <div
            className={
              wide
                ? "flex flex-col gap-3 px-6 pb-8"
                : "mx-auto flex max-w-160 flex-col gap-5 px-6 pt-5 pb-8"
            }
          >
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
            {section === "runs" && (
              <AgentRunsPane
                hostId={host ?? null}
                onHost={onHost}
                profileId={profile ?? null}
                onProfile={onProfile}
                onOpenRun={onOpenRun}
              />
            )}
            {section === "explain-agent" && (
              <ExplainAgentPane onOpenAgents={() => onSection("runs")} />
            )}
            {section === "explain-repositories" && <ExplainRepositoriesPane />}
            {section === "explain-concepts" && (
              <ExplainConceptsPane titleSlot={titleSlot} />
            )}
            {section === "explain-stored" && (
              <ExplainStoredPane titleSlot={titleSlot} />
            )}
            {section === "backup" && (
              <BackupPane onExport={onExport} onRestore={onRestore} />
            )}
          </div>
        </div>
      </main>
    </div>
  );
}
