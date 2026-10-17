import { useEffect, useState } from "react";
import {
  durationLabel,
  profileChoice,
  profileName,
  runMissing,
  TIME_LIMITS,
} from "../lib/agentRuns";
import {
  type AgentHost,
  type AgentProfile,
  type AgentSettings,
  errorMessage,
  ipc,
  type RepositorySummary,
  type Routine,
} from "../lib/ipc";
import AccessPicker, { toSend, useHostCopies } from "./AccessPicker";

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
  onSchedule,
}: {
  routine: Routine | null;
  repositories: RepositorySummary[];
  onSaved: (r: Routine) => void;
  onCancel?: () => void;
  onError: (message: string | null) => void;
  /** Opens the Schedule tab, where approval is. */
  onSchedule?: () => void;
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
  const [secretIds, setSecretIds] = useState<string[]>(
    routine?.secret_ids ?? [],
  );
  const [tools, setTools] = useState(routine?.tools ?? false);

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
  const blocked = profile && host ? runMissing(profile, host, tools) : [];

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
          secret_ids: secretIds,
          tools,
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
    <div className="flex flex-wrap items-start gap-4 px-5 py-4">
      <form
        aria-label={routine ? `Edit ${routine.name}` : "New routine"}
        className="flex min-w-0 flex-[999_1_440px] flex-col gap-4"
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

        <Group title="What it can use">
          <Field label="Access">
            <AccessPicker
              hostId={hostId}
              hostName={host?.name ?? "the host"}
              chosen={secretIds}
              onChange={setSecretIds}
              sentWith="the approval or Run Now"
            />
          </Field>
          <Field label="Tools" htmlFor="routine-tools">
            <span className="flex items-center gap-2">
              <input
                id="routine-tools"
                type="checkbox"
                checked={tools}
                onChange={(e) => setTools(e.target.checked)}
              />
              <span>Python and the Google Cloud CLI</span>
            </span>
            <Hint>
              Its runs use the image with them, which each host builds with its
              image.
            </Hint>
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
          <Field
            label="Earlier briefings shown to a run"
            htmlFor="routine-prev"
          >
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
      <Readiness
        host={host}
        profile={profile}
        tools={tools}
        secretIds={secretIds}
        blocked={blocked}
        onSchedule={routine ? onSchedule : undefined}
      />
    </div>
  );
}

/**
 * What a schedule on this host needs (docs/design/routines-ux.md, Definition):
 * what is ready, what approving does for you (→), and what only you can fix.
 */
function Readiness({
  host,
  profile,
  tools,
  secretIds,
  blocked,
  onSchedule,
}: {
  host: AgentHost | null;
  profile: AgentProfile | null;
  tools: boolean;
  secretIds: string[];
  blocked: string[];
  onSchedule?: () => void;
}) {
  const remote = host?.kind === "ssh";
  const held = useHostCopies(remote && host ? host.id : "");
  if (!host || !profile) return null;
  const sending = toSend(secretIds, held);
  const current = (i: AgentHost["image"]) => !!i?.current;
  type Line = { mark: "ok" | "auto" | "fix"; text: string; detail: string };
  const lines: Line[] = [];
  if (!remote) {
    lines.push({
      mark: "fix",
      text: "This Mac runs it with Run Now",
      detail:
        "A schedule needs a run host, which stays on while this Mac sleeps.",
    });
  } else {
    lines.push(
      profile.missing.length === 0
        ? {
            mark: "ok",
            text: `${profileName(profile)} has its key`,
            detail: "Sent sealed with the approval",
          }
        : {
            mark: "fix",
            text: "The agent is not ready",
            detail: profile.missing[0],
          },
    );
    if (host.upgrade_available)
      lines.push({
        mark: "auto",
        text: "Run controller upgrade",
        detail: "Done at approval",
      });
    if (!current(host.image) || (tools && !current(host.tools_image)))
      lines.push({
        mark: "auto",
        text: tools ? "Image with Python and gcloud" : "The image",
        detail: "Built at approval, about 4 minutes",
      });
    // Approval tests the agent there when no test of it still matches.
    const test = host.tests.find((t) => t.profile_id === profile.id);
    if (profile.missing.length === 0 && !test?.current)
      lines.push({
        mark: "auto",
        text: `${profileName(profile)}'s test on ${host.name}`,
        detail: test
          ? "Run again at approval: the key, the image, or the engine changed"
          : "Run at approval, about a minute",
      });
    if (secretIds.length > 0)
      lines.push(
        sending.length > 0
          ? {
              mark: "auto",
              text: `${secretIds.length} ${secretIds.length === 1 ? "secret" : "secrets"}`,
              detail: `${sending.join(", ")} sent at approval`,
            }
          : held
            ? {
                mark: "ok",
                text: `${secretIds.length} ${secretIds.length === 1 ? "secret" : "secrets"}`,
                detail: `Already on ${host.name}`,
              }
            : {
                mark: "auto",
                text: "Secrets",
                detail: "Sent at approval if the host lacks them",
              },
      );
    // The image and the test are what approval does; the rest is yours.
    for (const b of blocked.filter(
      (m) =>
        !/image|Python|^Pass a test of |^Test .* again:/.test(m) &&
        !profile.missing.includes(m),
    ))
      lines.push({ mark: "fix", text: "Only you can fix this", detail: b });
  }
  const MARK = { ok: "✓", auto: "→", fix: "!" } as const;
  const TONE = {
    ok: "text-clean",
    auto: "text-link",
    fix: "text-conflict",
  } as const;
  return (
    <aside
      aria-label="To schedule it"
      className="flex min-w-60 flex-[1_1_260px] flex-col gap-2 rounded-lg border bg-panel px-3.5 py-3"
    >
      <h3 className="m-0 text-[13px] font-semibold">
        {remote ? `To schedule it on ${host.name}` : "Where it runs"}
      </h3>
      <ul className="m-0 flex list-none flex-col gap-2 p-0">
        {lines.map((l) => (
          <li key={l.text + l.detail} className="flex gap-2">
            <span aria-hidden="true" className={`w-3 ${TONE[l.mark]}`}>
              {MARK[l.mark]}
            </span>
            <span className="flex flex-col">
              <span>{l.text}</span>
              <span className="text-[12px] text-muted">{l.detail}</span>
            </span>
          </li>
        ))}
      </ul>
      {remote && onSchedule && (
        <button
          type="button"
          className="btn btn-sm btn-primary"
          onClick={onSchedule}
        >
          Review and Approve…
        </button>
      )}
      {remote && (
        <span className="text-[12px] text-muted">
          The arrows are what approval does for you; a ! is something only you
          can fix.
        </span>
      )}
    </aside>
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
