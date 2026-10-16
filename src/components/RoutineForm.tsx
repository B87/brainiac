import { useEffect, useState } from "react";
import {
  durationLabel,
  profileChoice,
  runMissing,
  TIME_LIMITS,
} from "../lib/agentRuns";
import {
  type AgentSettings,
  errorMessage,
  ipc,
  type RepositorySummary,
  type Routine,
} from "../lib/ipc";

/**
 * A routine's definition (docs/design/routines-ux.md, Definition): what to
 * check, where it runs, what it can use, and its memory. Used for a new
 * routine and for editing one; it saves with the version it was read at.
 */
export default function RoutineForm({
  routine,
  repositories,
  onSaved,
  onCancel,
  onError,
}: {
  routine: Routine | null;
  repositories: RepositorySummary[];
  onSaved: (r: Routine) => void;
  onCancel?: () => void;
  onError: (message: string | null) => void;
}) {
  const [settings, setSettings] = useState<AgentSettings | null>(null);
  const [name, setName] = useState(routine?.name ?? "");
  const [repo, setRepo] = useState(
    routine?.repository_id ?? repositories[0]?.id ?? "",
  );
  const [profileId, setProfileId] = useState(routine?.profile_id ?? "");
  const [hostId, setHostId] = useState(routine?.host_id ?? "");
  const [start, setStart] = useState(routine?.start ?? "HEAD");
  const [prompt, setPrompt] = useState(routine?.prompt ?? "");
  const [limit, setLimit] = useState(routine?.time_limit_minutes ?? 30);
  const [memory, setMemory] = useState(routine?.memory ?? "");
  const [previous, setPrevious] = useState(routine?.previous_results ?? 3);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [secretNames, setSecretNames] = useState<string[]>([]);
  const [lacking, setLacking] = useState<string[]>([]);

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

  // What the agent's runs get, by name, and which of it the host does not hold.
  useEffect(() => {
    setSecretNames([]);
    setLacking([]);
    if (!profileId) return;
    let alive = true;
    (async () => {
      const [ids, all] = await Promise.all([
        ipc.getProfileSecrets(profileId),
        ipc.listNamedSecrets(),
      ]);
      const chosen = all.filter((n) => ids.includes(n.id));
      if (!alive) return;
      setSecretNames(chosen.map((n) => n.env_name));
      if (chosen.length === 0 || !hostId) return;
      const held = await ipc.getHostSecrets(hostId);
      if (!alive || held.problem) return;
      setLacking(
        held.rows
          .filter(
            (r) =>
              ids.includes(r.secret_id) &&
              (r.state === "not_shared" ||
                r.state === "missing_on_host" ||
                r.state === "failed"),
          )
          .map((r) => r.env_name),
      );
    })().catch(() => {});
    return () => {
      alive = false;
    };
  }, [profileId, hostId]);

  const ready =
    name.trim() !== "" &&
    prompt.trim() !== "" &&
    repo !== "" &&
    profileId !== "";
  const save = async () => {
    setBusy(true);
    setError(null);
    onError(null);
    try {
      onSaved(
        await ipc.saveRoutine({
          id: routine?.id ?? null,
          expected_version: routine?.version ?? 0,
          name,
          repository_id: repo,
          profile_id: profileId,
          host_id: hostId,
          start,
          prompt,
          time_limit_minutes: limit,
          memory,
          previous_results: previous,
        }),
      );
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      aria-label={routine ? `Edit ${routine.name}` : "New routine"}
      className="flex flex-col gap-4 px-5 py-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (ready && !busy) void save();
      }}
    >
      <Group title="What to check">
        <Field label="Name" htmlFor="routine-name">
          <input
            id="routine-name"
            className="field"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Morning check"
          />
        </Field>
        <Field label="Repository and start" htmlFor="routine-repo">
          <div className="flex flex-wrap gap-2">
            <select
              id="routine-repo"
              className="field w-auto"
              value={repo}
              onChange={(e) => setRepo(e.target.value)}
            >
              {repositories.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name}
                </option>
              ))}
            </select>
            <input
              className="field mono w-32"
              aria-label="Start from"
              value={start}
              onChange={(e) => setStart(e.target.value)}
            />
          </div>
          <Hint>A branch or commit, resolved to a commit at each start.</Hint>
        </Field>
        <Field label="Prompt" htmlFor="routine-prompt">
          <textarea
            id="routine-prompt"
            className="field min-h-[110px] resize-y leading-relaxed"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="Run the checks described in the repository's skill and report what changed since yesterday."
          />
        </Field>
      </Group>

      <Group title="Where it runs">
        <Field label="Agent, host and time limit">
          <div className="flex flex-wrap gap-2">
            <select
              className="field w-auto"
              aria-label="Agent"
              value={profileId}
              onChange={(e) => setProfileId(e.target.value)}
            >
              {settings?.profiles.map((p) => (
                <option key={p.id} value={p.id}>
                  {profileChoice(p)}
                </option>
              ))}
            </select>
            <select
              className="field w-auto"
              aria-label="Runs on"
              value={hostId}
              onChange={(e) => setHostId(e.target.value)}
            >
              {hosts.map((h) => (
                <option key={h.id} value={h.kind === "local" ? "" : h.id}>
                  {h.kind === "local" ? "This Mac" : h.name}
                </option>
              ))}
            </select>
            <select
              className="field w-auto"
              aria-label="Time limit"
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
          </div>
          {blocked.length > 0 ? (
            <span className="text-[12px] text-conflict">
              Not ready to run there: {blocked[0]}
            </span>
          ) : (
            profile &&
            host && <Hint>The agent passed its test on {host.name}.</Hint>
          )}
        </Field>
      </Group>

      <Group title="What it can use" note="set on the agent's page">
        <Field label="Host secrets">
          {secretNames.length ? (
            <div className="flex flex-wrap gap-1.5">
              {secretNames.map((n) => (
                <code key={n} className="mono rounded bg-panel px-1.5 py-px">
                  {n}
                </code>
              ))}
            </div>
          ) : (
            <span className="text-muted">None</span>
          )}
          {lacking.length > 0 ? (
            <span className="text-[12px] text-conflict">
              The host does not hold {lacking.join(", ")}: share{" "}
              {lacking.length === 1 ? "it" : "them"} on its page, or the run
              will not start.
            </span>
          ) : (
            <Hint>Sealed copies on the host; the agent could read them.</Hint>
          )}
        </Field>
        <Field label="Tools">
          <span>
            {profile?.tools
              ? "Python and Google Cloud CLI"
              : "The standard run image"}
          </span>
        </Field>
      </Group>

      <Group title="Memory">
        <Field label="Notes for the next run" htmlFor="routine-memory">
          <textarea
            id="routine-memory"
            className="field min-h-[70px] resize-y leading-relaxed"
            value={memory}
            onChange={(e) => setMemory(e.target.value)}
            placeholder="Empty. The agent can rewrite this after each run; you can edit it too."
          />
          <Hint>
            Part of the prompt, treated as data. Do not keep secrets here.
          </Hint>
        </Field>
        <Field label="Earlier briefings shown to a run" htmlFor="routine-prev">
          <select
            id="routine-prev"
            className="field w-auto"
            value={previous}
            onChange={(e) => setPrevious(Number(e.target.value))}
          >
            {Array.from({ length: 11 }, (_, n) => n).map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </Field>
      </Group>

      <div className="flex items-center gap-2">
        <button
          type="submit"
          className="btn btn-primary"
          disabled={!ready || busy}
        >
          {routine ? "Save" : "Create Routine"}
        </button>
        {onCancel && (
          <button type="button" className="btn" onClick={onCancel}>
            {routine ? "Revert" : "Cancel"}
          </button>
        )}
        {error && (
          <span className="text-[12px] text-conflict" role="alert">
            {error}
          </span>
        )}
      </div>
    </form>
  );
}

function Group({
  title,
  note,
  children,
}: {
  title: string;
  note?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="overflow-hidden rounded-lg border">
      <div className="flex items-baseline gap-2 border-b bg-panel px-3 py-2">
        <h3 className="m-0 text-[13px] font-semibold">{title}</h3>
        {note && <span className="text-[12px] text-muted">{note}</span>}
      </div>
      <div className="flex flex-col">{children}</div>
    </section>
  );
}

function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1 border-b px-3 py-2.5 last:border-b-0">
      <label htmlFor={htmlFor} className="text-[12.5px] font-medium">
        {label}
      </label>
      {children}
    </div>
  );
}

function Hint({ children }: { children: React.ReactNode }) {
  return <span className="text-[12px] text-muted">{children}</span>;
}
