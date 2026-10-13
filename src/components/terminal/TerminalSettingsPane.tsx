/**
 * Settings → Terminal (SPEC.md, section 15): the switch, what a new tab
 * runs, where the dock sits, and Option as Meta.
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
import { terminalStore, useDockSide } from "./store";

export default function TerminalSettingsPane() {
  const [view, setView] = useState<TerminalSettingsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Choosing Another program shows its fields before one is named.
  const [anotherChosen, setAnotherChosen] = useState(false);
  const dockSide = useDockSide();
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
  const another = anotherChosen || s.program !== "";
  return (
    <>
      {error && (
        <p role="alert" className="m-0 text-red-700 dark:text-red-300">
          {error}
        </p>
      )}
      <Group>
        <div className="settings-group">
          <label className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">Terminal in Brainiac</span>
              <Hint>
                Programs you run here, and the agents they start, can change
                your repositories and run Git, as in any terminal. Brainiac
                itself still writes nothing to them. Turned off, Brainiac starts
                no programs.
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
      <Group label="Run">
        <div className="settings-group">
          <div className="settings-row flex-col items-stretch gap-2.5">
            <fieldset
              aria-label="What a new tab runs"
              className="seg m-0 self-start border-0"
            >
              <button
                type="button"
                aria-pressed={!another}
                onClick={() => {
                  setAnotherChosen(false);
                  if (s.program) void save({ program: "", args: [] });
                }}
              >
                Your login shell{" "}
                <span className="mono text-muted">{view.login_shell}</span>
              </button>
              <button
                type="button"
                aria-pressed={another}
                onClick={() => setAnotherChosen(true)}
              >
                Another program…
              </button>
            </fieldset>
            <Hint>
              {another
                ? "A name on your shell’s PATH, such as herdr, or a full path. Tabs already open keep their program."
                : "Each tab is a login shell, which reads your profile as a terminal app’s tab does."}
            </Hint>
          </div>
          {another && (
            <>
              <CommitField
                label="Program"
                value={s.program}
                format={(v) => v}
                parse={(t) =>
                  t.trim() ? { value: t.trim() } : { error: "Name a program." }
                }
                onCommit={(program) => save({ program })}
                placeholder="herdr"
                mono
              />
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
            </>
          )}
        </div>
      </Group>
      <Group label="Dock">
        <div className="settings-group">
          <div className="settings-row">
            <fieldset
              aria-label="Where the dock sits"
              className="seg m-0 self-start border-0"
            >
              {(["bottom", "right"] as const).map((side) => (
                <button
                  key={side}
                  type="button"
                  aria-pressed={dockSide === side}
                  onClick={() => terminalStore.setDockSide(side)}
                >
                  {side === "bottom" ? "Bottom" : "Right"}
                </button>
              ))}
            </fieldset>
          </div>
        </div>
      </Group>
      <Group label="Keys">
        <div className="settings-group">
          <label className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">Option sends Meta</span>
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
