import { useEffect, useState } from "react";
import {
  durationLabel,
  profileChoice,
  runMissing,
  TIME_LIMITS,
} from "../lib/agentRuns";
import {
  type AgentSettings,
  type Briefing,
  errorMessage,
  ipc,
  type RepositorySummary,
} from "../lib/ipc";
import Dialog from "./Dialog";

/**
 * Add or edit a briefing (docs/design/scheduled-runs.md): which repository,
 * agent and host, what to check, and the time limit. Run now starts it.
 */
export default function BriefingDialog({
  briefing,
  repositories,
  onClose,
  onSaved,
}: {
  briefing: Briefing | null;
  repositories: RepositorySummary[];
  onClose: () => void;
  onSaved: (b: Briefing) => void;
}) {
  const [settings, setSettings] = useState<AgentSettings | null>(null);
  const [name, setName] = useState(briefing?.name ?? "");
  const [repo, setRepo] = useState(
    briefing?.repository_id ?? repositories[0]?.id ?? "",
  );
  const [profileId, setProfileId] = useState(briefing?.profile_id ?? "");
  const [hostId, setHostId] = useState(briefing?.host_id ?? "");
  const [start, setStart] = useState(briefing?.start ?? "HEAD");
  const [prompt, setPrompt] = useState(briefing?.prompt ?? "");
  const [limit, setLimit] = useState(briefing?.time_limit_minutes ?? 30);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    ipc
      .getAgentSettings()
      .then((s) => {
        if (!alive) return;
        setSettings(s);
        setProfileId((p) => p || (s.profiles[0]?.id ?? ""));
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, []);

  const hosts = settings?.hosts ?? [];
  const host =
    hosts.find((h) => (h.kind === "local" ? "" : h.id) === hostId) ?? null;
  const profile = settings?.profiles.find((p) => p.id === profileId) ?? null;
  const blocked = profile && host ? runMissing(profile, host) : [];

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      onSaved(
        await ipc.saveBriefing({
          id: briefing?.id ?? null,
          expected_version: briefing?.version ?? 0,
          name,
          repository_id: repo,
          profile_id: profileId,
          host_id: hostId,
          start,
          prompt,
          time_limit_minutes: limit,
        }),
      );
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };
  const ready =
    name.trim() !== "" &&
    prompt.trim() !== "" &&
    repo !== "" &&
    profileId !== "";

  return (
    <Dialog
      title={briefing ? `Edit ${briefing.name}` : "New briefing"}
      width={620}
      onClose={onClose}
      footer={
        <>
          {error && (
            <span className="mr-auto text-[12px] text-conflict" role="alert">
              {error}
            </span>
          )}
          <button type="button" className="btn btn-sm" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-sm btn-primary"
            disabled={!ready || busy}
            onClick={() => void save()}
          >
            Save
          </button>
        </>
      }
    >
      <form
        aria-label={briefing ? `Edit ${briefing.name}` : "New briefing"}
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (ready && !busy) void save();
        }}
      >
        <label className="flex flex-col gap-1">
          <span className="section-label">Name</span>
          <input
            className="field"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Morning check"
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="section-label">Repository</span>
          <select
            className="field"
            value={repo}
            onChange={(e) => setRepo(e.target.value)}
          >
            {repositories.map((r) => (
              <option key={r.id} value={r.id}>
                {r.name}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1">
          <span className="section-label">Start from</span>
          <input
            className="field mono"
            value={start}
            onChange={(e) => setStart(e.target.value)}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="section-label">What to check</span>
          <textarea
            className="field min-h-[110px] resize-y leading-relaxed"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="Run the checks described in the repository's skill and report what changed since yesterday."
          />
        </label>
        <div className="flex gap-3">
          <label className="flex min-w-0 flex-1 flex-col gap-1">
            <span className="section-label">Agent</span>
            <select
              className="field"
              value={profileId}
              onChange={(e) => setProfileId(e.target.value)}
            >
              {settings?.profiles.map((p) => (
                <option key={p.id} value={p.id}>
                  {profileChoice(p)}
                </option>
              ))}
            </select>
          </label>
          <label className="flex min-w-0 flex-1 flex-col gap-1">
            <span className="section-label">Runs on</span>
            <select
              className="field"
              value={hostId}
              onChange={(e) => setHostId(e.target.value)}
            >
              {hosts.map((h) => (
                <option key={h.id} value={h.kind === "local" ? "" : h.id}>
                  {h.kind === "local" ? "This Mac" : h.name}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1">
            <span className="section-label">Time limit</span>
            <select
              className="field"
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
            >
              {[...new Set([...TIME_LIMITS, limit])]
                .sort((a, b) => a - b)
                .map((m) => (
                  <option key={m} value={m}>
                    {durationLabel(m)}
                  </option>
                ))}
            </select>
          </label>
        </div>
        {blocked.length > 0 && (
          <p className="m-0 text-[12px] text-conflict">
            Not ready to run there: {blocked[0]}
          </p>
        )}
        <p className="m-0 text-[12px] text-muted">
          Run now starts a run that works without asking, inside its container,
          with the repository's code and your prompt sent to the agent's
          provider. The network is unrestricted. It reads this repository's
          commit and writes a briefing; Brainiac checks that file before showing
          it, and only proposes tasks. Nothing is created or sent anywhere until
          you approve it.
        </p>
      </form>
    </Dialog>
  );
}
