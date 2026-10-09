import { useEffect, useId, useMemo, useState } from "react";
import {
  DEPTHS,
  depthLabel,
  estimateText,
  formatDuration,
  sameSubject,
  subjectLabel,
  usageText,
} from "../lib/explain";
import { relativeTime } from "../lib/format";
import {
  type ExplainDialog as DialogData,
  type ExplainDepth,
  type ExplainSubject,
  type ExplanationRecord,
  errorMessage,
  ipc,
} from "../lib/ipc";
import { requestSettings } from "../lib/settings";
import Dialog from "./Dialog";

function FieldLabel({ children, id }: { children: string; id?: string }) {
  return (
    <span id={id} className="text-[12px] font-medium text-fg-2">
      {children}
    </span>
  );
}

/**
 * **Explain** (SPEC.md, section 14, Explain): who reads the code, the
 * depth, questions, the time limit and what it usually takes; the first
 * time in a repository, whether its code may be sent; and what still stops
 * an explanation before the first one.
 */
export default function ExplainDialog({
  repositoryId,
  subject,
  onClose,
  onStarted,
  onOpen,
}: {
  repositoryId: string;
  subject: ExplainSubject;
  onClose: () => void;
  onStarted: (record: ExplanationRecord) => void;
  /** Open another explanation of this subject. */
  onOpen: (id: string) => void;
}) {
  const id = useId();
  const [data, setData] = useState<DialogData | null>(null);
  const [profileId, setProfileId] = useState("");
  const [hostId, setHostId] = useState("");
  const [depth, setDepth] = useState<ExplainDepth>("teach_me");
  const [questions, setQuestions] = useState(true);
  const [scope, setScope] = useState<string>("");
  const [details, setDetails] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useMemo(
    () => () =>
      ipc
        .getExplainDialog(repositoryId, subject)
        .then((d) => {
          setData(d);
          setProfileId(
            (p) => p || d.profile_id || d.profiles[0]?.profile_id || "",
          );
          setHostId((h) => h || d.host_id || "local");
          setDepth(d.depth);
          setQuestions(d.settings.questions);
        })
        .catch((e) => setError(errorMessage(e))),
    [repositoryId, subject],
  );
  useEffect(() => {
    void load();
  }, [load]);

  const profile = data?.profiles.find((p) => p.profile_id === profileId);
  const host = profile?.hosts.find((h) => h.host_id === hostId);
  const consent = data?.consents.find((c) => c.provider === profile?.provider);
  const estimate = data?.estimates.find(
    (e) => e.profile_id === profileId && e.depth === depth,
  );
  const minutes = data
    ? depth === "brief"
      ? data.settings.brief_minutes
      : depth === "teach_me"
        ? data.settings.teach_me_minutes
        : data.settings.deep_minutes
    : 0;
  const missing = host?.missing ?? [];
  // Explain again replaces only this subject's own; a branch's or a pull
  // request's with the same changes is listed with where it was made.
  const same = data?.existing.find(
    (e) =>
      e.profile_id === profileId &&
      e.depth === depth &&
      sameSubject(e.subject, subject),
  );
  const others = data?.existing.filter((e) => e !== same) ?? [];
  const unasked = consent?.state === "unasked";
  const denied = consent?.state === "denied";
  const blocked = data?.blocked ?? null;
  const ready =
    !!profile &&
    !!host &&
    missing.length === 0 &&
    !host.busy &&
    !denied &&
    !blocked;
  const [fetching, setFetching] = useState(false);
  const fetchNow = () => {
    setFetching(true);
    setError(null);
    ipc
      .fetchRepository(repositoryId)
      .then(() => load())
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setFetching(false));
  };

  const start = async (answer?: boolean) => {
    if (!profile || !host) return;
    setBusy(true);
    setError(null);
    try {
      if (answer !== undefined) {
        await ipc.answerExplain({
          repository_id: repositoryId,
          provider: profile.provider,
          allowed: answer,
          workspace_id: scope || null,
        });
        if (!answer) {
          await load();
          return;
        }
      }
      const record = await ipc.startExplanation({
        repository_id: repositoryId,
        subject,
        profile_id: profile.profile_id,
        host_id: host.host_id,
        depth,
        questions,
      });
      onStarted(record);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const providerName =
    profile?.provider === "openai"
      ? "OpenAI"
      : profile?.provider === "openrouter"
        ? "OpenRouter"
        : "Anthropic";

  return (
    <Dialog
      title={data ? `Explain ${data.title}` : "Explain"}
      width={600}
      onClose={onClose}
      footer={
        <>
          <span className="mr-auto text-[12px] text-muted">
            {estimateText(estimate, profile?.payment)}
          </span>
          <button type="button" className="btn btn-sm" onClick={onClose}>
            Cancel
          </button>
          {data?.fetch_first ? (
            <button
              type="button"
              className="btn btn-sm btn-primary"
              disabled={fetching}
              onClick={fetchNow}
            >
              {fetching ? "Fetching…" : "Fetch now"}
            </button>
          ) : unasked ? (
            <>
              <button
                type="button"
                className="btn btn-sm"
                disabled={busy || !profile}
                onClick={() => void start(false)}
              >
                Don't send
              </button>
              <button
                type="button"
                className="btn btn-sm btn-primary"
                disabled={!ready || busy}
                onClick={() => void start(true)}
              >
                {busy ? "Starting…" : "Allow and explain"}
              </button>
            </>
          ) : (
            <button
              type="button"
              className="btn btn-sm btn-primary"
              disabled={!ready || busy}
              onClick={() => void start()}
            >
              {busy ? "Starting…" : same ? "Explain again" : "Explain"}
            </button>
          )}
        </>
      }
    >
      <div className="flex flex-col gap-4 text-[13px]">
        {error && (
          <div role="alert" className="text-[12.5px] text-conflict">
            {error}
          </div>
        )}
        {!data && !error && <span className="text-muted">Loading…</span>}
        {data && data.profiles.length === 0 && (
          <p className="m-0 text-fg-2">
            Explanations are written by an agent, as runs are. Set one up in
            Settings → Agents first.
          </p>
        )}
        {data && profile && (
          <>
            <div className="flex flex-wrap gap-3.5">
              <label className="flex min-w-[180px] flex-1 flex-col gap-1">
                <FieldLabel>Agent</FieldLabel>
                <select
                  className="field"
                  value={profileId}
                  onChange={(e) => setProfileId(e.target.value)}
                >
                  {data.profiles.map((p) => (
                    <option key={p.profile_id} value={p.profile_id}>
                      {p.name}
                    </option>
                  ))}
                </select>
              </label>
              {profile.hosts.length > 1 && (
                <label className="flex min-w-[140px] flex-1 flex-col gap-1">
                  <FieldLabel>Runs on</FieldLabel>
                  <select
                    className="field"
                    value={hostId}
                    onChange={(e) => setHostId(e.target.value)}
                  >
                    {profile.hosts.map((h) => (
                      <option
                        key={h.host_id}
                        value={h.host_id}
                        disabled={!!h.busy}
                      >
                        {h.name}
                        {h.busy ? " (busy)" : ""}
                      </option>
                    ))}
                  </select>
                </label>
              )}
            </div>
            <div className="flex flex-col gap-1.5">
              <FieldLabel id={`${id}-depth`}>Depth</FieldLabel>
              <fieldset
                className="seg m-0 self-start border-0"
                aria-labelledby={`${id}-depth`}
              >
                {DEPTHS.map((d) => (
                  <button
                    key={d}
                    type="button"
                    aria-pressed={depth === d}
                    onClick={() => setDepth(d)}
                  >
                    {depthLabel(d)}
                  </button>
                ))}
              </fieldset>
              <span className="text-[12px] text-muted">
                Time limit {formatDuration(minutes * 60)} · change it in
                Settings → Explanations
              </span>
            </div>
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={questions}
                onChange={(e) => setQuestions(e.target.checked)}
              />
              Add questions to check yourself
            </label>

            {blocked && (
              <div
                role="alert"
                className="rounded-lg bg-panel px-3 py-2.5 text-[12.5px] text-fg-2"
              >
                {blocked}
              </div>
            )}
            {subject.kind === "pull_request" && !blocked && (
              <p className="m-0 text-[12.5px] text-fg-2">
                The agent reads the pull request's commits, from where its head
                left the target branch. The description and comments are not
                sent.
              </p>
            )}
            {data.head_author && !blocked && (
              <div className="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-[12.5px] text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
                <span className="font-medium">
                  {data.head_author} wrote this pull request, not you.
                </span>{" "}
                Its Claude Code settings and instructions in the repository run
                in the container with your token or key.
              </div>
            )}
            {host?.busy && (
              <p role="note" className="m-0 text-[12.5px] text-muted">
                {host.name} has a job in progress; it is not offered until the
                job ends.
              </p>
            )}
            {missing.length > 0 && (
              <div className="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-[12.5px] text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
                <p className="m-0">
                  An explanation needs what a run needs. Still to do, in this
                  order:
                </p>
                <ol className="m-0 pl-5">
                  {missing.map((m) => (
                    <li key={m}>{m}</li>
                  ))}
                </ol>
                <button
                  type="button"
                  className="btn btn-sm mt-2"
                  onClick={() => {
                    onClose();
                    requestSettings("runs");
                  }}
                >
                  Open Settings → Agents
                </button>
              </div>
            )}

            {denied && (
              <div className="rounded-lg bg-panel px-3 py-2.5 text-[12.5px] text-fg-2">
                This repository is answered No for explanations with{" "}
                {providerName}
                {consent?.workspace_name
                  ? `, by the workspace ${consent.workspace_name}`
                  : ""}
                . Nothing is sent.{" "}
                <button
                  type="button"
                  className="text-link hover:underline"
                  onClick={() => {
                    onClose();
                    requestSettings("explanations");
                  }}
                >
                  Change in Settings
                </button>
              </div>
            )}
            {unasked && (
              <div className="flex flex-col gap-2 rounded-lg border px-3 py-2.5 text-[12.5px] leading-relaxed">
                <p className="m-0 font-medium">
                  Send this repository's code to {providerName} for
                  explanations?
                </p>
                <p className="m-0 text-fg-2">
                  The agent reads the change, its history, and the code around
                  it in a container, acting without asking, with the token or
                  key and an open network.{" "}
                  <button
                    type="button"
                    className="text-link hover:underline"
                    aria-expanded={details}
                    onClick={() => setDetails(!details)}
                  >
                    Details
                  </button>
                </p>
                {details && (
                  <p className="m-0 text-fg-3">
                    As for a run (Settings → Agents): the repository's own code
                    and its Claude Code settings can read the token or key, and
                    so can whoever controls the engine or administers a remote
                    host. The agent can reach any site, including your network.
                    Only its explanation comes back; its container is removed
                    once the explanation is checked.
                  </p>
                )}
                {data.workspaces.length > 0 && (
                  <label className="flex flex-col gap-1">
                    <FieldLabel>Answer for</FieldLabel>
                    <select
                      className="field"
                      value={scope}
                      onChange={(e) => setScope(e.target.value)}
                    >
                      <option value="">This repository</option>
                      {data.workspaces.map((w) => (
                        <option key={w.id} value={w.id}>
                          Every repository in {w.name}, also those added later
                        </option>
                      ))}
                    </select>
                  </label>
                )}
              </div>
            )}

            {(same || others.length > 0) && (
              <div className="flex flex-col gap-1 text-[12.5px]">
                <FieldLabel>Explained already</FieldLabel>
                {[...(same ? [same] : []), ...others].map((e) => (
                  <button
                    key={e.id}
                    type="button"
                    className="flex items-center gap-2 text-left text-link hover:underline"
                    onClick={() => onOpen(e.id)}
                  >
                    {depthLabel(e.depth)} ·{" "}
                    {data.profiles.find((p) => p.profile_id === e.profile_id)
                      ?.name ?? e.agent}{" "}
                    ·{" "}
                    {e.state === "ready" ? relativeTime(e.created_at) : e.state}
                    {usageText({ ...e, duration_secs: e.duration_secs }) &&
                      ` · ${usageText(e)}`}
                    {!sameSubject(e.subject, subject) &&
                      ` · from ${subjectLabel(e.subject)}`}
                  </button>
                ))}
                {same && (
                  <span className="text-[12px] text-muted">
                    Explain again replaces the {depthLabel(depth)} one and runs
                    again, at its cost.
                  </span>
                )}
              </div>
            )}
          </>
        )}
      </div>
    </Dialog>
  );
}
