import { useCallback, useEffect, useState } from "react";
import { relativeTime } from "../lib/format";
import {
  type AgentProvider,
  type AppSnapshot,
  type CodeAnswer,
  errorMessage,
  ipc,
  onCodeSharingChanged,
  subscribe,
} from "../lib/ipc";
import { Hint, PageTitle } from "./SettingsPanes";

const PROVIDER: Record<AgentProvider, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  openrouter: "OpenRouter",
};

/**
 * Settings → Code Sharing (SPEC.md, section 13, Code sharing): each
 * repository's and workspace's answer to whether its code may be sent to a
 * provider, by a run or an explanation, with Change and Ask again.
 */
export default function CodeSharingPane() {
  const [answers, setAnswers] = useState<CodeAnswer[] | null>(null);
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [target, setTarget] = useState("");
  const [provider, setProvider] = useState<AgentProvider>("anthropic");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(
    () =>
      ipc
        .listCodeSharingAnswers()
        .then(setAnswers)
        .catch((e) => setError(errorMessage(e))),
    [],
  );
  useEffect(() => {
    void ipc
      .getAppSnapshot()
      .then(setSnapshot)
      .catch((e) => setError(errorMessage(e)));
    void load();
    return subscribe(onCodeSharingChanged(() => void load()));
  }, [load]);

  const act = async (what: () => Promise<unknown>) => {
    setSaving(true);
    try {
      await what();
      setError(null);
      await load();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <PageTitle title="Code Sharing">
        Before a run or an explanation first sends a repository's code to a
        provider, Brainiac asks whether it may, for that repository or for every
        repository in one of its workspaces. A repository's own answer wins over
        its workspaces', and a No from any workspace wins over a Yes.
      </PageTitle>
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}
      <div className="settings-group">
        <div className="settings-row flex-wrap">
          <label className="flex min-w-55 flex-1 flex-col gap-1">
            <span className="font-medium">Repository or workspace</span>
            <select
              className="field"
              value={target}
              disabled={!snapshot || saving}
              onChange={(e) => setTarget(e.target.value)}
            >
              <option value="">Choose a repository or workspace…</option>
              <optgroup label="Repositories">
                {snapshot?.repositories.map((r) => (
                  <option key={r.id} value={`repository:${r.id}`}>
                    {r.name} · {r.display_path}
                  </option>
                ))}
              </optgroup>
              <optgroup label="Workspaces">
                {snapshot?.workspaces.map((w) => (
                  <option key={w.id} value={`workspace:${w.id}`}>
                    Every repository in {w.name}
                  </option>
                ))}
              </optgroup>
            </select>
          </label>
          <label className="flex flex-col gap-1">
            <span className="font-medium">Provider</span>
            <select
              className="field"
              value={provider}
              disabled={saving}
              onChange={(e) => setProvider(e.target.value as AgentProvider)}
            >
              {Object.entries(PROVIDER).map(([id, name]) => (
                <option key={id} value={id}>
                  {name}
                </option>
              ))}
            </select>
          </label>
          {[true, false].map((allowed) => (
            <button
              key={String(allowed)}
              type="button"
              className="btn btn-sm"
              disabled={!target || saving}
              onClick={() => {
                const workspace = target.startsWith("workspace:");
                const id = target.slice(target.indexOf(":") + 1);
                void act(() =>
                  ipc.answerCodeSharing({
                    repository_id: workspace ? "" : id,
                    workspace_id: workspace ? id : null,
                    provider,
                    allowed,
                  }),
                );
              }}
            >
              {allowed ? "Allow" : "Don’t allow"}
            </button>
          ))}
        </div>
      </div>
      <Hint>
        A workspace answer covers its current and future repositories. A
        repository’s own answer takes priority. Changing an answer controls
        future runs and explanations.
      </Hint>
      <div className="settings-group">
        {answers === null && !error && (
          <div className="settings-row text-muted">Loading…</div>
        )}
        {answers?.length === 0 && (
          <div className="settings-row text-muted">
            No answers saved yet. Choose a repository or workspace above.
          </div>
        )}
        {answers?.map((a) => (
          <div
            key={`${a.scope}:${a.scope_id}:${a.provider}`}
            className="settings-row"
          >
            <span className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-medium">
                {a.scope === "workspace" ? "Every repository in " : ""}
                {a.scope_name ??
                  (a.scope === "workspace"
                    ? "a removed workspace"
                    : "a removed repository")}
              </span>
              <Hint>
                {a.allowed ? "Allowed to send" : "Not allowed to send"} to{" "}
                {PROVIDER[a.provider] ?? a.provider} ·{" "}
                {relativeTime(a.answered_at)}
              </Hint>
            </span>
            <select
              className="field"
              aria-label={`Answer for ${a.scope_name ?? a.scope_id}`}
              disabled={saving}
              value={a.allowed ? "yes" : "no"}
              onChange={(e) =>
                void act(() =>
                  ipc.answerCodeSharing({
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
              disabled={saving}
              title="Remove this answer; workspace answers still apply"
              onClick={() =>
                void act(() =>
                  ipc.forgetCodeSharingAnswer(a.scope, a.scope_id, a.provider),
                )
              }
            >
              Ask again
            </button>
          </div>
        ))}
      </div>
      <Hint>
        What is sent: the repository's history up to the start commit, the
        prompt, and anything the agent reads. A host's Test sends no
        repository's code.
      </Hint>
    </>
  );
}
