import { useCallback, useEffect, useState } from "react";
import { relativeTime } from "../lib/format";
import {
  type CodeAnswer,
  errorMessage,
  ipc,
  onCodeSharingChanged,
  subscribe,
} from "../lib/ipc";
import { Hint, Lede } from "./SettingsPanes";

const PROVIDER: Record<string, string> = {
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
    void load();
    return subscribe(onCodeSharingChanged(() => void load()));
  }, [load]);

  const act = async (what: () => Promise<unknown>) => {
    try {
      await what();
      setError(null);
      await load();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <>
      <Lede>
        Before a run or an explanation first sends a repository's code to a
        provider, Brainiac asks whether it may, for that repository or for every
        repository in one of its workspaces. A repository's own answer wins over
        its workspaces', and a No from any workspace wins over a Yes.
      </Lede>
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}
      <div className="settings-group">
        {answers === null && !error && (
          <div className="settings-row text-muted">Loading…</div>
        )}
        {answers?.length === 0 && (
          <div className="settings-row text-muted">
            No repository asked yet. The first run or Explain in each asks.
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
                {a.allowed ? "Sent" : "Never sent"} to{" "}
                {PROVIDER[a.provider] ?? a.provider} ·{" "}
                {relativeTime(a.answered_at)}
              </Hint>
            </span>
            <select
              className="field"
              aria-label={`Answer for ${a.scope_name ?? a.scope_id}`}
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
              title="Ask again at the next run or Explain"
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
