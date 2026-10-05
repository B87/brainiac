import { type ReactNode, useEffect, useId, useRef, useState } from "react";
import {
  ACCESS_CHOICES,
  accessSummary,
  claudeCommand,
  connectedText,
  pluginFits,
} from "../lib/agent";
import { shortPath } from "../lib/format";
import {
  type AgentAccessStatus,
  errorMessage,
  ipc,
  type Settings,
  type VaultState,
} from "../lib/ipc";
import {
  argsText,
  EDITOR_PRESETS,
  editorPreset,
  hasPathArgument,
  MIB,
  parseArgs,
  parseInRange,
} from "../lib/settings";
import AccountsSection from "./AccountsSection";
import SecretsSection from "./SecretsSection";
import type { SaveSettings } from "./SettingsPage";
import VaultSetup from "./VaultSetup";

/** How often the connected agents are counted while Agent Access is shown. */
const AGENT_POLL_MS = 2000;

/** What the section is for; the page's header names it. */
export function Lede({ children }: { children: ReactNode }) {
  return <p className="m-0 text-[12.5px] text-fg-2">{children}</p>;
}

export function Group({
  label,
  children,
}: {
  label?: string;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-1.5">
      {label && <h2 className="section-label m-0">{label}</h2>}
      {children}
    </section>
  );
}

export function Hint({ children, id }: { children: ReactNode; id?: string }) {
  return (
    <span id={id} className="text-[12px] text-muted">
      {children}
    </span>
  );
}

/**
 * A field saved when it is left or with Return; Escape puts back the saved
 * value. `parse` says why a value cannot be saved, shown under the field.
 */
export function CommitField<T>({
  label,
  hint,
  value,
  format,
  parse,
  onCommit,
  unit,
  mono = false,
  width = 220,
  placeholder,
  suggestions,
}: {
  label: string;
  hint?: ReactNode;
  value: T;
  format: (value: T) => string;
  parse: (text: string) => { value: T } | { error: string };
  onCommit: (value: T) => Promise<boolean>;
  unit?: string;
  mono?: boolean;
  width?: number;
  placeholder?: string;
  /** Values offered while typing; anything else can still be typed. */
  suggestions?: string[];
}) {
  const id = useId();
  const saved = format(value);
  const [text, setText] = useState(saved);
  const [problem, setProblem] = useState<string | null>(null);
  useEffect(() => setText(saved), [saved]);

  const commit = async () => {
    const parsed = parse(text);
    if ("error" in parsed) {
      setProblem(parsed.error);
      return;
    }
    setProblem(null);
    // "060" or extra spaces mean the saved value: show it as saved.
    const typed = format(parsed.value);
    setText(typed);
    if (typed === saved) return;
    if (!(await onCommit(parsed.value))) setText(saved);
  };

  // Leaving Settings by a shortcut or the menu removes the field without a
  // blur; what was typed is saved all the same, if it can be.
  const commitRef = useRef(commit);
  commitRef.current = commit;
  const typedRef = useRef(false);
  typedRef.current = text !== saved;
  useEffect(
    () => () => {
      if (typedRef.current) void commitRef.current();
    },
    [],
  );
  const describedBy = [hint && `${id}-hint`, problem && `${id}-problem`]
    .filter(Boolean)
    .join(" ");

  return (
    <div className="settings-row">
      <div className="flex min-w-55 flex-1 flex-col gap-0.5">
        <label htmlFor={id} className="font-medium">
          {label}
        </label>
        {hint && <Hint id={`${id}-hint`}>{hint}</Hint>}
        {problem && (
          <span
            id={`${id}-problem`}
            role="alert"
            className="text-[12px] text-conflict"
          >
            {problem}
          </span>
        )}
      </div>
      <span className="flex items-center gap-1.5 text-[12px] text-fg-2">
        <input
          id={id}
          className={`h-7 rounded-md border border-control-line bg-field px-2 text-fg ${
            mono ? "mono text-[12px]" : "text-right"
          } ${problem ? "border-conflict" : ""}`}
          style={{ width }}
          value={text}
          placeholder={placeholder}
          list={suggestions ? `${id}-suggestions` : undefined}
          spellCheck={false}
          inputMode={unit ? "numeric" : undefined}
          aria-invalid={problem ? true : undefined}
          aria-describedby={describedBy || undefined}
          onChange={(e) => setText(e.target.value)}
          onBlur={() => void commit()}
          onKeyDown={(e) => {
            if (e.key === "Enter") void commit();
            if (e.key === "Escape") {
              setText(saved);
              setProblem(null);
            }
          }}
        />
        {suggestions && (
          <datalist id={`${id}-suggestions`}>
            {suggestions.map((s) => (
              <option key={s} value={s} />
            ))}
          </datalist>
        )}
        {unit}
      </span>
    </div>
  );
}

export function GeneralPane({
  settings,
  save,
}: {
  settings: Settings;
  save: SaveSettings;
}) {
  const editor = settings.editor;
  const matched = editorPreset(editor);
  // Choosing Custom keeps the preset's values to edit from.
  const [customChosen, setCustomChosen] = useState(false);
  const current = customChosen || !matched ? "custom" : matched;
  const preset = EDITOR_PRESETS.find((p) => p.id === current);
  const needsPath = (text: string) => {
    const args = parseArgs(text);
    return hasPathArgument(args)
      ? { value: args }
      : {
          error: "Include {path} or {path_url}, where the folder or file goes.",
        };
  };
  return (
    <>
      <Lede>
        Which editor Open in Editor uses, for repositories, files in a diff, and
        notes.
      </Lede>
      <Group label="Open in editor">
        <div className="settings-group">
          <div className="settings-row flex-col items-stretch gap-2.5">
            <fieldset
              aria-label="Editor"
              className="seg m-0 self-start border-0"
            >
              {EDITOR_PRESETS.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  aria-pressed={current === p.id}
                  onClick={() => {
                    setCustomChosen(false);
                    void save({ editor: p.editor });
                  }}
                >
                  {p.label}
                </button>
              ))}
              <button
                type="button"
                aria-pressed={current === "custom"}
                onClick={() => setCustomChosen(true)}
              >
                Custom
              </button>
            </fieldset>
            <Hint>
              {preset
                ? preset.note
                : "Any program, with its arguments for a repository and for a file at a line."}
            </Hint>
          </div>
          {current === "custom" && (
            <>
              <CommitField
                label="Program"
                hint="A full path, or a name on the PATH macOS gives apps opened from the Dock."
                value={editor.executable}
                format={(v) => v}
                parse={(t) =>
                  t.trim() ? { value: t.trim() } : { error: "Name a program." }
                }
                onCommit={(executable) =>
                  save({ editor: { ...editor, executable } })
                }
                mono
              />
              <CommitField
                label="Arguments for a repository"
                value={editor.repo_args}
                format={argsText}
                parse={needsPath}
                onCommit={(repo_args) =>
                  save({ editor: { ...editor, repo_args } })
                }
                mono
              />
              <CommitField
                label="Arguments for a file at a line"
                value={editor.file_args}
                format={argsText}
                parse={needsPath}
                onCommit={(file_args) =>
                  save({ editor: { ...editor, file_args } })
                }
                mono
              />
            </>
          )}
        </div>
        {current === "custom" && (
          <Hint>
            Arguments are separated by spaces;{" "}
            <span className="mono">{"{path}"}</span>,{" "}
            <span className="mono">{"{path_url}"}</span> (the path encoded for a
            link), and <span className="mono">{"{line}"}</span> are filled in.
            Brainiac runs the program directly, never through a shell.
          </Hint>
        )}
      </Group>
    </>
  );
}

export function NotesPane({
  settings,
  save,
  vault,
  onVault,
  onError,
}: {
  settings: Settings;
  save: SaveSettings;
  vault: VaultState | null;
  onVault: (state: VaultState) => void;
  onError: (message: string) => void;
}) {
  const [rebuilding, setRebuilding] = useState(false);
  const rebuild = async () => {
    setRebuilding(true);
    try {
      await ipc.rebuildSearch();
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setRebuilding(false);
    }
  };
  const index = vault?.index;
  return (
    <>
      <Lede>
        Your vault is an ordinary folder of Markdown files. Brainiac edits them
        where they are and never moves or converts them.
      </Lede>
      <Group label="Vault">
        <div className="settings-group">
          <div className="settings-row">
            {vault?.vault ? (
              <>
                <span
                  className="mono min-w-0 flex-1 truncate text-[12.5px]"
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
                      .catch((e) => onError(errorMessage(e)))
                  }
                >
                  Reveal in Finder
                </button>
              </>
            ) : (
              <span className="text-muted">No vault chosen.</span>
            )}
          </div>
          <div className="settings-row">
            <div className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">
                {vault?.vault ? "Use another folder" : "Choose a vault"}
              </span>
              <Hint>
                Notes Brainiac knew in the old folder stay listed as missing,
                with their tasks and links.
              </Hint>
            </div>
            <VaultSetup compact onDone={onVault} />
          </div>
        </div>
      </Group>
      <Group label="Note identity">
        <div className="settings-group">
          <label className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">
                Write <span className="mono">brainiac_id</span> into a note when
                it first gets a task or a repository link
              </span>
              <Hint>
                So renaming the note outside Brainiac keeps its context. Opening
                or indexing a note never writes it.
              </Hint>
            </span>
            <input
              type="checkbox"
              role="switch"
              className="switch"
              aria-checked={settings.write_note_ids}
              checked={settings.write_note_ids}
              onChange={(e) => void save({ write_note_ids: e.target.checked })}
            />
          </label>
        </div>
      </Group>
      <Group label="Search">
        <div className="settings-group">
          <div className="settings-row">
            <span className="flex-1 text-[12.5px] text-fg-2">
              {index?.state === "indexing"
                ? `Indexing notes: ${index.done} of ${index.total}.`
                : index?.state === "unavailable"
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
        </div>
      </Group>
    </>
  );
}

export function RepositoriesPane({
  settings,
  save,
}: {
  settings: Settings;
  save: SaveSettings;
}) {
  const range = (min: number, max: number) => (text: string) =>
    parseInRange(text, min, max);
  const count = (n: number) => n.toLocaleString("en");
  return (
    <>
      <Lede>
        How often Brainiac looks at your repositories, and how much of a diff it
        shows. These apply to every workspace.
      </Lede>
      <Group label="Status">
        <div className="settings-group">
          <CommitField
            label="Refresh status every"
            hint="While Brainiac is open. Refresh in a repository's header reads it at once."
            value={settings.refresh_interval_seconds}
            format={count}
            parse={range(10, 86_400)}
            onCommit={(v) => save({ refresh_interval_seconds: v })}
            unit="seconds"
            width={80}
          />
        </div>
      </Group>
      <Group label="Fetching">
        <div className="settings-group">
          <CommitField
            label="Auto-fetch every"
            hint="For workspaces that turned auto-fetch on in their Activity tab. It is off by default."
            value={settings.auto_fetch_interval_minutes}
            format={count}
            parse={range(5, 10_080)}
            onCommit={(v) => save({ auto_fetch_interval_minutes: v })}
            unit="minutes"
            width={80}
          />
          <CommitField
            label="Stop a fetch after"
            value={settings.fetch_timeout_seconds}
            format={count}
            parse={range(10, 3_600)}
            onCommit={(v) => save({ fetch_timeout_seconds: v })}
            unit="seconds"
            width={80}
          />
        </div>
        <div className="rounded-lg border border-info-line bg-info-bg px-3 py-2 text-[12px] text-info-fg">
          A fetch only updates remote-tracking branches and tags. Brainiac never
          checks out, pulls, commits, or runs hooks.
        </div>
      </Group>
      <Group label="Diffs">
        <div className="settings-group">
          <CommitField
            label="Show a file's diff up to"
            value={Math.max(
              1,
              Math.round(settings.diff_limits.max_bytes / MIB),
            )}
            format={count}
            parse={range(1, 1_024)}
            onCommit={(v) =>
              save({
                diff_limits: { ...settings.diff_limits, max_bytes: v * MIB },
              })
            }
            unit="MiB"
            width={80}
          />
          <CommitField
            label="and up to"
            hint="A longer diff is cut off with a note and Open in Editor, never silently."
            value={settings.diff_limits.max_lines}
            format={count}
            parse={range(1_000, 10_000_000)}
            onCommit={(v) =>
              save({ diff_limits: { ...settings.diff_limits, max_lines: v } })
            }
            unit="lines"
            width={80}
          />
        </div>
      </Group>
    </>
  );
}

export function DatabasesPane({
  settings,
  save,
}: {
  settings: Settings;
  save: SaveSettings;
}) {
  return (
    <>
      <Lede>
        Connections are added and edited from Databases. Passwords stay in the
        Keychain or come from a source in Settings → Secrets; results stay in
        memory until their tab closes.
      </Lede>
      <Group label="History">
        <div className="settings-group">
          <label className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">
                Keep a history of the statements run
              </span>
              <Hint>
                The SQL, the connection, when, how long, and the row count or
                error: never the rows. 90 days or 10,000 runs per connection.
                Each connection's history can be cleared from its side panel.
              </Hint>
            </span>
            <input
              type="checkbox"
              role="switch"
              className="switch"
              aria-checked={settings.query_history}
              checked={settings.query_history}
              onChange={(e) => void save({ query_history: e.target.checked })}
            />
          </label>
        </div>
      </Group>
    </>
  );
}

export function AccountsPane({ onChanged }: { onChanged: () => void }) {
  return (
    <>
      <Lede>
        For pull requests on GitHub and Bitbucket Cloud. A token is kept in the
        macOS Keychain, or read from a command such as{" "}
        <span className="mono">gh auth token</span> or an environment variable,
        and sent only to the service it belongs to.
      </Lede>
      <AccountsSection onChanged={onChanged} />
      <Hint>
        Pull requests stay off in each workspace until it turns them on.
      </Hint>
    </>
  );
}

export function SecretsPane() {
  return (
    <>
      <Lede>
        Where each account's token and each connection's password comes from,
        and what needs your attention. Change a source where it is entered: in
        Accounts, in the connection's Edit Connection…, or in Agents.
      </Lede>
      <SecretsSection />
      <Hint>
        Brainiac keeps a secret it read for as long as it runs. Refresh forgets
        it, so it is read, or asked for, again when next used.
      </Hint>
    </>
  );
}

export function AgentsPane({
  settings,
  save,
}: {
  settings: Settings;
  save: SaveSettings;
}) {
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

  const command = agent ? claudeCommand(agent.executable) : null;
  const plugin = agent ? pluginFits(agent.executable) : false;
  const copyCommand = () => {
    if (!command) return;
    void navigator.clipboard
      ?.writeText(command)
      .then(() => setCopied(true))
      .catch(() => {});
  };

  return (
    <>
      <Lede>
        Lets agents such as Claude Code use your notes, tasks, and tracked
        repositories over MCP. A change applies at once to agents already
        connected.
      </Lede>
      <div className="settings-group">
        <div className="settings-row flex-col items-stretch gap-2.5">
          <fieldset
            aria-label="Agent access"
            className="seg m-0 self-start border-0"
          >
            {ACCESS_CHOICES.map((choice) => (
              <button
                key={choice.value}
                type="button"
                aria-pressed={settings.agent_access === choice.value}
                onClick={() => void save({ agent_access: choice.value })}
              >
                {choice.label}
              </button>
            ))}
          </fieldset>
          <p className="m-0 text-[12.5px] text-fg-2">
            {accessSummary(settings.agent_access)}
          </p>
        </div>
        {agent && (
          <div className="settings-row">
            <span className="text-[12.5px] text-fg-2" aria-live="polite">
              {connectedText(agent.connections, settings.agent_access)}
            </span>
          </div>
        )}
        {agent?.problem && (
          <div
            role="alert"
            className="settings-row text-[12.5px] text-conflict"
          >
            Agents cannot connect: {agent.problem}
          </div>
        )}
      </div>
      {command && (
        <Group label="Connect Claude Code">
          <div className="settings-group">
            <div className="settings-row flex-col items-stretch gap-2">
              <Hint>
                Run once in a terminal; it adds Brainiac for every project.
              </Hint>
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
            </div>
            {plugin && (
              <div className="settings-row flex-col items-stretch gap-1">
                <span className="font-medium">
                  Or install the plugin instead
                </span>
                <Hint>
                  It adds the server and a skill that teaches Claude Code how to
                  use Brainiac:{" "}
                  <span className="mono select-all">
                    /plugin marketplace add B87/brainiac
                  </span>
                  , then{" "}
                  <span className="mono select-all">
                    /plugin install brainiac@brainiac
                  </span>
                  . Use one or the other, not both.
                </Hint>
              </div>
            )}
          </div>
          <Hint>
            Other MCP clients run the same program with the argument{" "}
            <span className="mono">mcp</span>.
          </Hint>
        </Group>
      )}
    </>
  );
}

export function BackupPane({
  onExport,
  onRestore,
}: {
  onExport: () => void;
  onRestore: () => void;
}) {
  return (
    <>
      <Lede>
        Brainiac snapshots its data before each upgrade and daily, on this Mac.
        A complete backup is an export kept on another device or backup system.
      </Lede>
      <div className="settings-group">
        <div className="settings-row">
          <div className="flex min-w-55 flex-1 flex-col gap-0.5">
            <span className="font-medium">Export</span>
            <Hint>The File menu has these too.</Hint>
          </div>
          <span className="flex flex-wrap gap-1.5">
            <button type="button" className="btn btn-sm" onClick={onExport}>
              Export…
            </button>
            <button type="button" className="btn btn-sm" onClick={onRestore}>
              Restore from Export…
            </button>
          </span>
        </div>
      </div>
    </>
  );
}
