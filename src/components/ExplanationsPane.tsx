import { useCallback, useEffect, useState } from "react";
import { DEPTHS, depthLabel } from "../lib/explain";
import {
  type ExplanationSettings,
  type ExplanationSettingsView,
  errorMessage,
  ipc,
  type LanguageLevel,
  onExplanationChanged,
  subscribe,
} from "../lib/ipc";
import { ConceptList, StoredList } from "./ExplanationLists";
import { CommitField, Group, Hint, Lede } from "./SettingsPanes";

const LEVELS: { value: LanguageLevel; label: string }[] = [
  { value: "new", label: "New" },
  { value: "comfortable", label: "Comfortable" },
  { value: "expert", label: "Expert" },
];

/**
 * Settings → Explanations' settings and lists, kept current, with how to save
 * a change or run an action and reload (SPEC.md, section 14). Each of its four
 * pages loads its own.
 */
function useExplanationSettings() {
  const [view, setView] = useState<ExplanationSettingsView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(
    () =>
      ipc
        .getExplanationSettings()
        .then(setView)
        .catch((e) => setError(errorMessage(e))),
    [],
  );
  useEffect(() => {
    void load();
    return subscribe(onExplanationChanged(() => void load()));
  }, [load]);

  const save = async (patch: Partial<ExplanationSettings>) => {
    if (!view) return false;
    try {
      await ipc.saveExplanationSettings({ ...view.settings, ...patch });
      setError(null);
      await load();
      return true;
    } catch (e) {
      setError(errorMessage(e));
      return false;
    }
  };
  const act = async (what: () => Promise<unknown>) => {
    try {
      await what();
      setError(null);
      await load();
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return { view, error, save, act };
}

function ErrorLine({ error }: { error: string | null }) {
  return error ? (
    <div role="alert" className="text-[12.5px] text-conflict">
      {error}
    </div>
  ) : null;
}

function Loading({ error }: { error: string | null }) {
  return error ? (
    <ErrorLine error={error} />
  ) : (
    <span className="text-muted">Loading…</span>
  );
}

/** Settings → Agent and Depth: who explains, with what model and time, for whom. */
export function ExplainAgentPane({
  onOpenAgents,
}: {
  onOpenAgents: () => void;
}) {
  const { view, error, save } = useExplanationSettings();
  const [language, setLanguage] = useState("");

  if (!view) return <Loading error={error} />;
  const s = view.settings;
  const profile = view.profiles.find((p) => p.profile_id === s.profile_id);
  const minutes = (text: string): { value: number } | { error: string } => {
    const n = Number(text.trim());
    return Number.isInteger(n) && n >= 1 && n <= 120
      ? { value: n }
      : { error: "Between 1 and 120 minutes." };
  };
  const model = (text: string): { value: string } | { error: string } =>
    /\s/.test(text.trim())
      ? { error: "A model name has no spaces." }
      : { value: text.trim() };
  return (
    <>
      <Lede>
        An agent explains a commit, a branch, a pull request, or a run's result
        when you ask, in a run of its own on the host chosen here. It needs what
        a run needs in Agents; each repository is asked once before its code is
        sent.
      </Lede>
      <ErrorLine error={error} />

      <Group label="Agent">
        <div className="settings-group">
          <label className="settings-row">
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">Explains with</span>
              <Hint>
                At first the agent and host of your last run, or the first agent
                whose test passed on this Mac.
              </Hint>
            </span>
            <select
              className="field"
              value={s.profile_id ?? ""}
              onChange={(e) =>
                void save({
                  profile_id: e.target.value || null,
                  host_id: e.target.value ? (s.host_id ?? "local") : null,
                })
              }
            >
              <option value="">As the last run</option>
              {view.profiles.map((p) => (
                <option key={p.profile_id} value={p.profile_id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
          {profile && profile.hosts.length > 1 && (
            <label className="settings-row">
              <span className="flex-1 font-medium">On</span>
              <select
                className="field"
                value={s.host_id ?? "local"}
                onChange={(e) => void save({ host_id: e.target.value })}
              >
                {profile.hosts.map((h) => (
                  <option key={h.host_id} value={h.host_id}>
                    {h.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          {view.profiles.length === 0 && (
            <div className="settings-row">
              <span className="flex-1 text-fg-2">No agent is set up yet.</span>
              <button
                type="button"
                className="btn btn-sm"
                onClick={onOpenAgents}
              >
                Open Agents
              </button>
            </div>
          )}
        </div>
      </Group>

      <Group label="Model and time limit by depth">
        <div className="settings-group">
          <CommitField
            label="Brief, Claude Code's model"
            value={s.brief_model}
            format={(v) => v}
            parse={model}
            onCommit={(v) => save({ brief_model: v })}
            mono
            suggestions={["sonnet", "opus", "haiku"]}
          />
          <CommitField
            label="Teach me, Claude Code's model"
            value={s.teach_me_model}
            format={(v) => v}
            parse={model}
            onCommit={(v) => save({ teach_me_model: v })}
            mono
            suggestions={["sonnet", "opus", "haiku"]}
          />
          <CommitField
            label="Deep, Claude Code's model"
            hint="OpenCode uses its own model from Agents."
            value={s.deep_model}
            format={(v) => v}
            parse={model}
            onCommit={(v) => save({ deep_model: v })}
            mono
            suggestions={["sonnet", "opus", "haiku"]}
          />
          <CommitField
            label="Brief, time limit"
            value={s.brief_minutes}
            format={String}
            parse={minutes}
            onCommit={(v) => save({ brief_minutes: v })}
            unit="minutes"
            width={70}
          />
          <CommitField
            label="Teach me, time limit"
            value={s.teach_me_minutes}
            format={String}
            parse={minutes}
            onCommit={(v) => save({ teach_me_minutes: v })}
            unit="minutes"
            width={70}
          />
          <CommitField
            label="Deep, time limit"
            value={s.deep_minutes}
            format={String}
            parse={minutes}
            onCommit={(v) => save({ deep_minutes: v })}
            unit="minutes"
            width={70}
          />
        </div>
      </Group>

      <Group label="You">
        <div className="settings-group">
          <div className="settings-row">
            <span className="flex-1 font-medium">Default depth</span>
            <fieldset className="seg m-0 border-0" aria-label="Default depth">
              {DEPTHS.map((d) => (
                <button
                  key={d}
                  type="button"
                  aria-pressed={s.default_depth === d}
                  onClick={() => void save({ default_depth: d })}
                >
                  {depthLabel(d)}
                </button>
              ))}
            </fieldset>
          </div>
          <label className="settings-row">
            <span className="flex-1 font-medium">Add questions by default</span>
            <input
              type="checkbox"
              role="switch"
              className="switch"
              aria-checked={s.questions}
              checked={s.questions}
              onChange={(e) => void save({ questions: e.target.checked })}
            />
          </label>
          {s.levels.map((l) => (
            <div key={l.language} className="settings-row">
              <span className="flex-1">{l.language}</span>
              <select
                className="field"
                aria-label={`Your level in ${l.language}`}
                value={l.level}
                onChange={(e) =>
                  void save({
                    levels: s.levels.map((x) =>
                      x.language === l.language
                        ? { ...x, level: e.target.value as LanguageLevel }
                        : x,
                    ),
                  })
                }
              >
                {LEVELS.map((v) => (
                  <option key={v.value} value={v.value}>
                    {v.label}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="btn btn-sm btn-ghost"
                onClick={() =>
                  void save({
                    levels: s.levels.filter((x) => x.language !== l.language),
                  })
                }
              >
                Remove
              </button>
            </div>
          ))}
          <form
            className="settings-row"
            onSubmit={(e) => {
              e.preventDefault();
              const name = language.trim();
              if (!name || s.levels.some((l) => l.language === name)) return;
              void save({
                levels: [...s.levels, { language: name, level: "comfortable" }],
              }).then((ok) => ok && setLanguage(""));
            }}
          >
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">Your level, by language</span>
              <Hint>Explanations say more where you are new.</Hint>
            </span>
            <input
              className="field w-36"
              placeholder="Rust, TypeScript…"
              value={language}
              onChange={(e) => setLanguage(e.target.value)}
            />
            <button type="submit" className="btn btn-sm">
              Add
            </button>
          </form>
        </div>
      </Group>
    </>
  );
}

/** Settings → Concepts You Know: the ledger explanations leave out. */
export function ExplainConceptsPane({
  titleSlot,
}: {
  titleSlot: HTMLElement | null;
}) {
  const { view, error, act } = useExplanationSettings();
  if (!view) return <Loading error={error} />;
  return (
    <>
      <ErrorLine error={error} />
      <ConceptList concepts={view.concepts} act={act} titleSlot={titleSlot} />
    </>
  );
}

/** Settings → Stored Explanations: every explanation kept, with its cost. */
export function ExplainStoredPane({
  titleSlot,
}: {
  titleSlot: HTMLElement | null;
}) {
  const { view, error, act } = useExplanationSettings();
  if (!view) return <Loading error={error} />;
  return (
    <>
      <ErrorLine error={error} />
      <StoredList
        stored={view.stored}
        totalBytes={view.stored_bytes}
        act={act}
        titleSlot={titleSlot}
      />
      <Hint>
        Kept until deleted, unlike runs, each with its run's conversation, which
        holds the code the agent read.
      </Hint>
    </>
  );
}
