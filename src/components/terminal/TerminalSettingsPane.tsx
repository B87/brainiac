/**
 * Settings → Terminal (SPEC.md, section 15): the switch, the program a new
 * tab runs, and Option as Meta.
 */
import { useCallback, useEffect, useState } from "react";
import {
  errorMessage,
  ipc,
  type TerminalSettings,
  type TerminalSettingsView,
} from "../../lib/ipc";
import { argsText, parseArgs } from "../../lib/settings";
import { CommitField, Group, Hint, Lede } from "../SettingsPanes";
import { terminalStore } from "./store";

export default function TerminalSettingsPane() {
  const [view, setView] = useState<TerminalSettingsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    ipc
      .getTerminalSettings()
      .then(setView)
      .catch((e) => setError(errorMessage(e)));
  }, []);
  const save = useCallback(
    async (patch: Partial<TerminalSettings>): Promise<boolean> => {
      if (!view) return false;
      const settings = { ...view.settings, ...patch };
      try {
        await ipc.saveTerminalSettings(settings);
        setView({ ...view, settings });
        terminalStore.setOptionAsMeta(settings.option_as_meta);
        setError(null);
        return true;
      } catch (e) {
        setError(errorMessage(e));
        return false;
      }
    },
    [view],
  );
  if (!view)
    return error ? (
      <p className="text-red-700">{error}</p>
    ) : (
      <Lede>Loading…</Lede>
    );
  const s = view.settings;
  return (
    <>
      <Lede>
        Tabs in the Terminal view and the dock run your login shell, or the
        program you name here. What runs in them is yours, as in any terminal:
        commands and agents there can change your repositories and run Git.
      </Lede>
      {error && (
        <p role="alert" className="m-0 text-red-700 dark:text-red-300">
          {error}
        </p>
      )}
      <Group>
        <div className="settings-group">
          <label className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">Terminal</span>
              <Hint>
                Off, no tab opens and Brainiac starts no program. Tabs already
                open stay until you close them.
              </Hint>
            </span>
            <input
              type="checkbox"
              role="switch"
              className="switch"
              aria-checked={s.enabled}
              checked={s.enabled}
              onChange={(e) => void save({ enabled: e.target.checked })}
            />
          </label>
        </div>
      </Group>
      <Group label="A new tab runs">
        <div className="settings-group">
          <CommitField
            label="Program"
            hint={
              <>
                Empty for your login shell (
                <span className="mono">{view.login_shell}</span>). A name on
                your shell’s PATH, such as <span className="mono">herdr</span>,
                or a full path.
              </>
            }
            value={s.program}
            format={(v) => v}
            parse={(t) => ({ value: t.trim() })}
            onCommit={(program) => save({ program })}
            placeholder="Login shell"
            mono
          />
          {s.program && (
            <CommitField
              label="Arguments"
              hint="Separated by spaces, passed as they are, never to a shell."
              value={s.args}
              format={argsText}
              parse={(t) => ({ value: parseArgs(t) })}
              onCommit={(args) => save({ args })}
              mono
              width={320}
            />
          )}
        </div>
      </Group>
      <Group label="Keys">
        <div className="settings-group">
          <label className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">Use Option as Meta</span>
              <Hint>
                Option and a key sends Esc and the key, for a shell’s and an
                editor’s Alt keys. Off, Option types the characters it types
                elsewhere. Applies to tabs opened after the change.
              </Hint>
            </span>
            <input
              type="checkbox"
              role="switch"
              className="switch"
              aria-checked={s.option_as_meta}
              checked={s.option_as_meta}
              onChange={(e) => void save({ option_as_meta: e.target.checked })}
            />
          </label>
        </div>
      </Group>
    </>
  );
}
