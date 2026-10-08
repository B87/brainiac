import { ask } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { DEPTHS, depthLabel, formatCost, formatDuration } from "../lib/explain";
import { relativeTime } from "../lib/format";
import {
  type ConceptKind,
  type ExplanationSettings,
  type ExplanationSettingsView,
  errorMessage,
  ipc,
  type KnownConcept,
  type LanguageLevel,
  onExplanationChanged,
  subscribe,
} from "../lib/ipc";
import { plural } from "../lib/repo";
import { CommitField, Group, Hint, Lede } from "./SettingsPanes";

const LEVELS: { value: LanguageLevel; label: string }[] = [
  { value: "new", label: "New" },
  { value: "comfortable", label: "Comfortable" },
  { value: "expert", label: "Expert" },
];

const KIND_WORD: Record<ConceptKind, string> = {
  language: "Language",
  library: "Library",
  system: "System tool",
  project_pattern: "Project pattern",
};

function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

const PROVIDER: Record<string, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  openrouter: "OpenRouter",
};

/** Settings → Explanations (SPEC.md, section 14). */
export default function ExplanationsPane({
  onOpenAgents,
}: {
  onOpenAgents: () => void;
}) {
  const [view, setView] = useState<ExplanationSettingsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [language, setLanguage] = useState("");

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

  if (!view)
    return error ? (
      <div role="alert" className="text-[12.5px] text-conflict">
        {error}
      </div>
    ) : (
      <span className="text-muted">Loading…</span>
    );
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
  const general = view.concepts.filter((c) => !c.repository_id);
  const patterns = view.concepts.filter((c) => c.repository_id);
  const byRepository = new Map<string, KnownConcept[]>();
  for (const c of patterns) {
    const key = c.repository_name ?? c.repository_id ?? "";
    byRepository.set(key, [...(byRepository.get(key) ?? []), c]);
  }

  return (
    <>
      <Lede>
        An agent explains a commit, a branch, or a run's result when you ask, in
        a run of its own on the host chosen here. It needs what a run needs in
        Agents; each repository is asked once before its code is sent.
      </Lede>
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}

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

      <Group label="Concepts you know">
        <Hint>
          Languages, libraries, and system tools are left out of every
          repository's explanations; a project pattern only out of its own
          repository's, so one client's names never reach another's prompts.
        </Hint>
        <div className="settings-group">
          {view.concepts.length === 0 && (
            <div className="settings-row text-muted">
              Got it on a concept adds it here.
            </div>
          )}
          {general.map((c) => (
            <ConceptRow
              key={c.id}
              concept={c}
              others={general.filter(
                (o) => o.id !== c.id && o.kind === c.kind && !o.merged_into,
              )}
              act={act}
            />
          ))}
          {[...byRepository.entries()].map(([name, list]) => (
            <div key={name} className="flex flex-col">
              <div className="settings-row text-[12px] font-medium text-fg-2">
                {name}
              </div>
              {list.map((c) => (
                <ConceptRow
                  key={c.id}
                  concept={c}
                  others={list.filter((o) => o.id !== c.id && !o.merged_into)}
                  act={act}
                />
              ))}
            </div>
          ))}
        </div>
      </Group>

      <Group label="Repositories">
        <div className="settings-group">
          {view.answers.length === 0 && (
            <div className="settings-row text-muted">
              No repository asked yet. The first Explain in each asks.
            </div>
          )}
          {view.answers.map((a) => (
            <div
              key={`${a.scope}:${a.scope_id}:${a.provider}`}
              className="settings-row"
            >
              <span className="flex min-w-55 flex-1 flex-col gap-0.5">
                <span className="font-medium">
                  {a.scope === "workspace" ? "Every repository in " : ""}
                  {a.scope_name ?? "A removed repository"}
                </span>
                <Hint>
                  {a.allowed ? "Sent" : "Never sent"} to{" "}
                  {PROVIDER[a.provider] ?? a.provider} ·{" "}
                  {relativeTime(a.answered_at)}
                </Hint>
              </span>
              <select
                className="field"
                aria-label="Answer"
                value={a.allowed ? "yes" : "no"}
                onChange={(e) =>
                  void act(() =>
                    ipc.answerExplain({
                      repository_id: a.scope === "repository" ? a.scope_id : "",
                      provider: a.provider,
                      allowed: e.target.value === "yes",
                      workspace_id: a.scope === "workspace" ? a.scope_id : null,
                    }),
                  )
                }
              >
                <option value="yes">Yes</option>
                <option value="no">No</option>
              </select>
              <button
                type="button"
                className="btn btn-sm btn-ghost"
                title="Ask again at the next Explain"
                onClick={() =>
                  void act(() =>
                    ipc.forgetExplainAnswer(a.scope, a.scope_id, a.provider),
                  )
                }
              >
                Ask again
              </button>
            </div>
          ))}
        </div>
      </Group>

      <Group label="Stored explanations">
        <Hint>
          Kept until deleted, unlike runs, each with its run's conversation,
          which holds the code the agent read.{" "}
          {plural(view.stored.length, "explanation")},{" "}
          {bytes(view.stored_bytes)}
          {view.stored_cost.length > 0 &&
            `, ${view.stored_cost.map(formatCost).join(" + ")} reported`}
          .
        </Hint>
        <div className="settings-group">
          {view.stored.map((e) => (
            <div key={e.id} className="settings-row">
              <span className="flex min-w-55 flex-1 flex-col gap-0.5">
                <span className="truncate font-medium" title={e.title}>
                  {e.repository_name} · {e.title}
                </span>
                <Hint>
                  {e.subject.kind} · {depthLabel(e.depth)} ·{" "}
                  {e.model || e.agent} · {relativeTime(e.created_at)}
                  {e.state !== "ready" ? ` · ${e.state}` : ""}
                  {e.duration_secs !== null
                    ? ` · ${formatDuration(e.duration_secs)}`
                    : ""}
                  {e.cost ? ` · ${formatCost(e.cost)}` : ""}
                </Hint>
              </span>
              <button
                type="button"
                className="btn btn-sm btn-ghost"
                disabled={e.state === "working"}
                onClick={() => void act(() => ipc.deleteExplanation(e.id))}
              >
                Delete
              </button>
            </div>
          ))}
          {view.stored.length > 0 && (
            <div className="settings-row">
              <span className="flex-1" />
              <button
                type="button"
                className="btn btn-sm"
                onClick={() =>
                  void (async () => {
                    const sure = await ask(
                      "Delete every stored explanation and its run's conversation? One still being written is left.",
                      {
                        title: "Delete All Explanations",
                        kind: "warning",
                        okLabel: "Delete All",
                      },
                    );
                    if (sure) await act(() => ipc.deleteAllExplanations());
                  })()
                }
              >
                Delete all
              </button>
            </div>
          )}
        </div>
      </Group>
    </>
  );
}

function ConceptRow({
  concept,
  others,
  act,
}: {
  concept: KnownConcept;
  others: KnownConcept[];
  act: (what: () => Promise<unknown>) => Promise<void>;
}) {
  const merged = concept.merged_into;
  return (
    <div className="settings-row">
      <span className="flex min-w-55 flex-1 flex-col gap-0.5">
        <span className="font-medium">{concept.name}</span>
        <Hint>
          {KIND_WORD[concept.kind]}
          {merged ? " · merged" : ""}
        </Hint>
      </span>
      {!merged && others.length > 0 && (
        <select
          className="field"
          aria-label={`Merge ${concept.name} into`}
          value=""
          onChange={(e) =>
            e.target.value &&
            void act(() => ipc.mergeConcept(concept.id, e.target.value))
          }
        >
          <option value="">Merge into…</option>
          {others.map((o) => (
            <option key={o.id} value={o.id}>
              {o.name}
            </option>
          ))}
        </select>
      )}
      <button
        type="button"
        className="btn btn-sm btn-ghost"
        onClick={() => void act(() => ipc.forgetConcept(concept.id))}
      >
        Remove
      </button>
    </div>
  );
}
