import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { relativeTime, shortAgo } from "../lib/format";
import {
  type ActivityItem,
  type ActivitySettings,
  type AppSnapshot,
  errorMessage,
  ipc,
  type RepositoryFreshness,
  type TeamPulse,
  type Workspace,
  type WorkspaceActivity,
} from "../lib/ipc";
import { olderThan, plural } from "../lib/repo";
import { createLatest } from "../lib/stale";
import { Avatar } from "./HistoryTab";
import {
  AdvancedIcon,
  BellIcon,
  BranchIcon,
  ClockIcon,
  FetchIcon,
  RewrittenIcon,
  TagIcon,
} from "./icons";
import type { RepoFocus } from "./RepositoryView";

type Props = {
  snapshot: AppSnapshot;
  workspace: Workspace;
  /** Repository IDs with a fetch running. */
  fetching: ReadonlySet<string>;
  onFetch: (repositoryIds: string[]) => void;
  onOpen: (repositoryId: string, focus: RepoFocus) => void;
  /** Settings or seen markers changed; the snapshot (sidebar badge) needs a reload. */
  onChanged: () => void;
  onError: (message: string | null) => void;
};

/** A repository not fetched for this long gets a warning. */
const STALE_FETCH_DAYS = 2;

export default function ActivityView({
  snapshot,
  workspace,
  fetching,
  onFetch,
  onOpen,
  onChanged,
  onError,
}: Props) {
  const [activity, setActivity] = useState<WorkspaceActivity | null>(null);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState(false);
  const latest = useRef(createLatest()).current;

  const load = useCallback(() => {
    setLoading(true);
    void latest.run(
      () => ipc.getWorkspaceActivity(workspace.id),
      (a) => {
        setLoading(false);
        if (a.workspace_id !== workspace.id) return;
        setActivity(a);
        if (pendingSaves.current === 0) setDraft(null);
      },
      (e) => {
        setLoading(false);
        onError(errorMessage(e));
      },
    );
  }, [workspace.id, latest, onError]);

  // Reload when news arrives, something is marked seen, or a fetch finishes.
  const memberIds = useMemo(
    () => new Set(workspace.members.flatMap((m) => m.repository_id ?? [])),
    [workspace.members],
  );
  const fetchedKey = snapshot.repositories
    .filter((r) => memberIds.has(r.id))
    .map((r) => `${r.last_fetch_at}:${r.fetch_error?.message ?? ""}`)
    .join("|");
  // biome-ignore lint/correctness/useExhaustiveDependencies: these values signal new data, they are not read.
  useEffect(() => {
    load();
  }, [load, workspace.unseen_activity, fetchedKey]);

  // Edits show at once and are saved one after another, each building on the
  // previous one, so two quick toggles cannot overwrite each other.
  const [draft, setDraft] = useState<ActivitySettings | null>(null);
  const settings = draft ?? activity?.settings ?? workspace.activity;
  const latestSettings = useRef(settings);
  latestSettings.current = settings;
  const saving = useRef<Promise<void>>(Promise.resolve());
  const pendingSaves = useRef(0);
  const update = (patch: Partial<ActivitySettings>) => {
    const next = { ...latestSettings.current, ...patch };
    latestSettings.current = next;
    setDraft(next);
    pendingSaves.current++;
    saving.current = saving.current.then(async () => {
      try {
        await ipc.updateActivitySettings(workspace.id, next);
      } catch (e) {
        onError(errorMessage(e));
        setDraft(null);
      }
      pendingSaves.current--;
      if (pendingSaves.current === 0) {
        onChanged();
        load();
      }
    });
  };
  const markSeen = async (ids?: string[]) => {
    try {
      await ipc.markActivitySeen(workspace.id, ids);
      onChanged();
      load();
    } catch (e) {
      onError(errorMessage(e));
    }
  };

  const items = activity?.items ?? [];
  const unread = items.filter((i) => !i.seen);
  const seen = items.filter((i) => i.seen);
  const unreadRepos = new Set(unread.map((i) => i.repository_id)).size;
  const ids = [...memberIds];
  const anyFetching = ids.some((id) => fetching.has(id));

  return (
    <div className="flex min-h-0 flex-1">
      <section
        aria-label="Activity feed"
        className="relative flex min-w-0 flex-1 flex-col gap-3 overflow-y-auto px-6 pt-[18px] pb-6"
      >
        {loading && activity && <div className="progress-line" />}
        <FreshnessBanner
          freshness={activity?.freshness ?? []}
          settings={settings}
          fetching={anyFetching}
          onFetch={() => onFetch(ids)}
        />

        <div className="flex flex-wrap items-center gap-2">
          <span className="section-label text-fg-2">
            New since you last looked
          </span>
          <span className="text-[11.5px] text-muted">
            {unread.length
              ? `${plural(unread.length, "update")} in ${plural(unreadRepos, "repository", "repositories")}`
              : "nothing new"}
          </span>
          <span className="flex-1" />
          {!editing && (
            <>
              <span className="text-[12px] text-muted">Watching</span>
              {settings.watched_branches.map((b) => (
                <span key={`b:${b}`} className="watch-chip mono">
                  <BranchIcon size={11} />
                  {b}
                </span>
              ))}
              {settings.watched_tags.map((t) => (
                <span key={`t:${t}`} className="watch-chip mono">
                  <TagIcon size={11} />
                  {t}
                </span>
              ))}
              <button
                type="button"
                className="btn btn-sm btn-ghost text-link"
                onClick={() => setEditing(true)}
              >
                Edit
              </button>
            </>
          )}
          {unread.length > 0 && (
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => void markSeen()}
            >
              Mark all as seen
            </button>
          )}
        </div>
        {editing && (
          <WatchEditor
            settings={settings}
            onCancel={() => setEditing(false)}
            onSave={(next) => {
              setEditing(false);
              update({
                watched_branches: next.watched_branches,
                watched_tags: next.watched_tags,
              });
            }}
          />
        )}

        {!activity && <FeedSkeleton />}
        {activity && items.length === 0 && (
          <div className="rounded-lg border border-dashed px-4 py-6 text-center text-fg-2">
            Nothing has moved on the watched branches yet.
            <div className="mt-1 text-[12px] text-muted">
              Brainiac compares remote-tracking branches after each fetch and
              lists merges, rewrites, and new tags here.
            </div>
          </div>
        )}
        <div className="flex flex-col gap-2.5">
          {unread.map((item) => (
            <ItemCard
              key={item.id}
              item={item}
              warnConflicts={settings.warn_conflicts}
              onShow={() => onOpen(item.repository_id, historyFocus(item))}
              onSeen={() => void markSeen([item.id])}
            />
          ))}
          {seen.length > 0 && (
            <div className="flex items-center gap-2.5 py-1 text-[11px] font-semibold tracking-wide text-muted">
              <span className="h-px flex-1 bg-line" />
              SEEN · EARLIER
              <span className="h-px flex-1 bg-line" />
            </div>
          )}
          {seen.map((item) => (
            <ItemCard
              key={item.id}
              item={item}
              warnConflicts={settings.warn_conflicts}
              onShow={() => onOpen(item.repository_id, historyFocus(item))}
            />
          ))}
        </div>
      </section>

      <aside
        aria-label="Team pulse and settings"
        className="flex w-[320px] shrink-0 flex-col overflow-y-auto border-l bg-panel"
      >
        <Pulse pulse={activity?.pulse ?? null} name={workspace.name} />
        <div className="flex flex-col gap-2.5 border-b px-[18px] py-3.5">
          <span className="section-label text-fg-2">Keep up to date</span>
          <Toggle
            checked={settings.auto_fetch}
            label={`Auto-fetch watched branches every ${snapshot.settings.auto_fetch_interval_minutes} min`}
            hint="Updates remote-tracking branches only, never your files or branches. Off by default. SSH keys that ask for approval will ask on each fetch."
            onChange={(v) => update({ auto_fetch: v })}
          />
        </div>
        <div className="flex flex-col gap-2.5 border-b px-[18px] py-3.5">
          <span className="flex items-center gap-1.5">
            <BellIcon size={12} className="text-muted" />
            <span className="section-label text-fg-2">Let me know</span>
          </span>
          <Toggle
            checked={settings.notify_moves}
            label="Notify when a watched branch moves"
            hint="A macOS notification, at most one per repository per hour"
            onChange={(v) => update({ notify_moves: v })}
          />
          <Toggle
            checked={settings.morning_digest}
            label="Morning digest at 9:00"
            hint="What landed since you last looked, while Brainiac is running"
            onChange={(v) => update({ morning_digest: v })}
          />
          <Toggle
            checked={settings.warn_conflicts}
            label="Warn when news touches files I'm changing"
            hint="Compares incoming paths with your working tree"
            onChange={(v) => update({ warn_conflicts: v })}
          />
        </div>
        <LastFetched
          freshness={activity?.freshness ?? []}
          fetching={fetching}
          onFetch={(id) => onFetch([id])}
        />
      </aside>
    </div>
  );
}

function historyFocus(item: ActivityItem): RepoFocus {
  return {
    tab: "history",
    ref: {
      full_name: item.full_ref,
      name: item.ref_name,
      kind: item.full_ref.startsWith("refs/tags/") ? "tag" : "remote_branch",
    },
    commitId: item.new_id,
  };
}

function FreshnessBanner({
  freshness,
  settings,
  fetching,
  onFetch,
}: {
  freshness: RepositoryFreshness[];
  settings: ActivitySettings;
  fetching: boolean;
  onFetch: () => void;
}) {
  // Freshness arrives oldest first.
  const oldest = freshness[0];
  const stale =
    oldest &&
    (!oldest.last_fetch_at ||
      olderThan(oldest.last_fetch_at, STALE_FETCH_DAYS));
  return (
    <div className="flex items-center gap-2.5 rounded-lg border border-info-line bg-info-bg px-3 py-2 text-[12.5px] text-info-fg">
      <ClockIcon size={15} className="shrink-0" />
      <span className="flex-1">
        News arrives when a fetch brings it in: yours, your editor's, a pull, or
        Brainiac's Fetch
        {settings.auto_fetch ? " (auto-fetch is on)" : ""}.{" "}
        {stale && (
          <span className="font-medium text-dirty">
            {oldest.name}{" "}
            {oldest.last_fetch_at
              ? `was last fetched ${relativeTime(oldest.last_fetch_at)}.`
              : "has never been fetched."}
          </span>
        )}
      </span>
      {(stale || fetching) && (
        <button
          type="button"
          className="btn btn-sm"
          disabled={fetching || freshness.length === 0}
          onClick={onFetch}
        >
          <FetchIcon size={12} className={fetching ? "animate-pulse" : ""} />
          {fetching ? "Fetching…" : "Fetch all now"}
        </button>
      )}
    </div>
  );
}

function WatchEditor({
  settings,
  onSave,
  onCancel,
}: {
  settings: ActivitySettings;
  onSave: (s: ActivitySettings) => void;
  onCancel: () => void;
}) {
  const [branches, setBranches] = useState(
    settings.watched_branches.join(", "),
  );
  const [tags, setTags] = useState(settings.watched_tags.join(", "));
  const list = (v: string) =>
    v
      .split(/[,\s]+/)
      .map((x) => x.trim())
      .filter(Boolean);
  return (
    <form
      className="flex flex-wrap items-end gap-3 rounded-lg border bg-panel px-3.5 py-3"
      onSubmit={(e) => {
        e.preventDefault();
        onSave({
          ...settings,
          watched_branches: list(branches),
          watched_tags: list(tags),
        });
      }}
    >
      <label className="flex min-w-[220px] flex-1 flex-col gap-1 text-[12px] text-fg-2">
        Branches (names or globs, without the remote)
        <input
          className="text-input mono"
          value={branches}
          placeholder="main, develop, release/*"
          onChange={(e) => setBranches(e.target.value)}
        />
      </label>
      <label className="flex min-w-[160px] flex-col gap-1 text-[12px] text-fg-2">
        Tags (globs)
        <input
          className="text-input mono"
          value={tags}
          placeholder="v*"
          onChange={(e) => setTags(e.target.value)}
        />
      </label>
      <button type="submit" className="btn btn-primary btn-sm h-7">
        Save
      </button>
      <button type="button" className="btn btn-sm h-7" onClick={onCancel}>
        Cancel
      </button>
      <span className="w-full text-[11.5px] text-muted">
        Refs that already exist join silently; only later moves become news.
      </span>
    </form>
  );
}

function FeedSkeleton() {
  return (
    <div
      className="flex flex-col gap-2.5"
      role="status"
      aria-label="Loading activity"
    >
      {[0, 1, 2].map((i) => (
        <div key={i} className="flex gap-3 rounded-lg border px-3.5 py-3">
          <div className="skeleton h-6 w-6" />
          <div className="flex flex-1 flex-col gap-2">
            <div className="skeleton w-1/2" />
            <div className="skeleton h-2 w-1/4" />
          </div>
        </div>
      ))}
    </div>
  );
}

const KIND_ICON = {
  advanced: { icon: <AdvancedIcon size={13} />, tone: "ren" },
  rewritten: { icon: <RewrittenIcon size={13} />, tone: "del" },
  created: { icon: <BranchIcon size={13} />, tone: "add" },
  tagged: { icon: <TagIcon size={13} />, tone: "mod" },
} as const;

function what(item: ActivityItem): string {
  const commits = plural(item.total_commits, "commit");
  switch (item.kind) {
    case "advanced":
      return item.merges
        ? `gained ${commits}, ${plural(item.merges, "merge")}`
        : `gained ${commits}`;
    case "rewritten":
      return "was rewritten";
    case "created":
      return item.total_commits
        ? `was created with ${commits} of its own`
        : "was created";
    case "tagged":
      return "was tagged";
  }
}

function who(item: ActivityItem): string {
  const people = item.authors.slice(0, 3).join(", ");
  const more =
    item.authors.length > 3 ? ` and ${item.authors.length - 3} more` : "";
  switch (item.kind) {
    case "tagged": {
      const since =
        item.previous_tag && item.commits_since_previous_tag !== null
          ? ` · ${plural(item.commits_since_previous_tag, "commit")} since ${item.previous_tag}`
          : "";
      return `${people ? `Release by ${people}` : "Release"}${since}`;
    }
    case "rewritten":
      return `${plural(item.replaced, "commit")} replaced, ${item.total_commits} added${people ? ` · ${people}` : ""}`;
    default:
      return `${people}${more}`;
  }
}

function ItemCard({
  item,
  warnConflicts,
  onShow,
  onSeen,
}: {
  item: ActivityItem;
  warnConflicts: boolean;
  onShow: () => void;
  onSeen?: () => void;
}) {
  const k = KIND_ICON[item.kind];
  const conflicts = warnConflicts ? item.conflict_paths : [];
  const hidden = Math.max(0, item.total_commits - item.commits.length);
  return (
    <article
      className={`flex flex-col gap-2 rounded-lg border bg-panel px-3.5 py-3 ${item.seen ? "opacity-75" : ""}`}
      aria-label={`${item.repository_name} ${item.ref_name} ${what(item)}`}
    >
      <div className="flex items-center gap-2.5">
        <span
          className="h-2 w-2 shrink-0 rounded-full"
          style={{ background: item.seen ? "transparent" : "var(--link)" }}
          title={item.seen ? undefined : "Unread"}
        />
        <span className="kind h-6 w-6 rounded-md" data-tone={k.tone}>
          {k.icon}
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-px">
          <span className="truncate">
            <span className="font-semibold">{item.repository_name}</span>{" "}
            <span className="text-muted">·</span>{" "}
            <span className="mono text-[12px] text-link">{item.ref_name}</span>{" "}
            <span className="text-fg-3">{what(item)}</span>
          </span>
          <span className="truncate text-[12px] text-muted">{who(item)}</span>
        </div>
        <span
          className="shrink-0 text-[12px] text-muted"
          title={`Noticed ${new Date(item.observed_at).toLocaleString()}`}
        >
          {shortAgo(item.observed_at)}
        </span>
      </div>
      {item.commits.length > 0 && item.kind !== "tagged" && (
        <div className="flex flex-col gap-1 pl-[52px] text-[12.5px]">
          {item.commits.map((c) => (
            <div key={c.id} className="flex min-w-0 items-center gap-2">
              <Avatar name={c.author_name} size={16} />
              <span className="mono shrink-0 text-[11.5px] text-muted">
                {c.short_id}
              </span>
              <span className="truncate text-fg-3">
                {c.is_merge && <span className="tag-box mr-1.5">merge</span>}
                {c.subject}
              </span>
            </div>
          ))}
          {hidden > 0 && (
            <span className="text-[12px] text-muted">
              and {plural(hidden, "more commit")}
            </span>
          )}
        </div>
      )}
      {item.kind === "rewritten" && (
        <Alert tone="danger">
          History changed under existing commits. A pull may need care.
        </Alert>
      )}
      {conflicts.length > 0 && (
        <Alert tone="warn">
          Touches{" "}
          <span className="mono">{conflicts.slice(0, 3).join(", ")}</span>
          {conflicts.length > 3 ? ` and ${conflicts.length - 3} more` : ""},
          which you are also changing in your working tree.
        </Alert>
      )}
      {item.drift && (
        <Alert tone="info">
          ↓ Your branch <span className="mono">{item.drift.branch}</span> is now{" "}
          {plural(item.drift.behind, "commit")} behind{" "}
          <span className="mono">{item.drift.base}</span>.
        </Alert>
      )}
      <div className="flex gap-2 pl-[52px]">
        <button type="button" className="btn btn-sm" onClick={onShow}>
          Show in History
        </button>
        {onSeen && (
          <button type="button" className="btn btn-sm" onClick={onSeen}>
            Mark seen
          </button>
        )}
      </div>
    </article>
  );
}

function Alert({
  tone,
  children,
}: {
  tone: "warn" | "info" | "danger";
  children: React.ReactNode;
}) {
  const style =
    tone === "warn"
      ? { background: "var(--k-mod-bg)", color: "var(--k-mod-fg)" }
      : tone === "danger"
        ? { background: "var(--k-del-bg)", color: "var(--k-del-fg)" }
        : { background: "var(--k-ren-bg)", color: "var(--k-ren-fg)" };
  return (
    <div
      className="ml-[52px] flex items-start gap-2 rounded-md px-2.5 py-1.5 text-[12px]"
      style={style}
    >
      {tone !== "info" && <span className="font-bold">!</span>}
      <span>{children}</span>
    </div>
  );
}

function Pulse({ pulse, name }: { pulse: TeamPulse | null; name: string }) {
  const top = pulse?.authors[0]?.commits ?? 0;
  return (
    <div className="flex flex-col gap-2.5 border-b px-[18px] pt-4 pb-3.5">
      <span className="section-label text-fg-2">This week in {name}</span>
      <div className="flex gap-[18px]">
        <Stat value={pulse?.commits} label="commits" />
        <Stat value={pulse?.merges} label="merges" />
        <Stat value={pulse?.releases} label="releases" />
      </div>
      {pulse?.authors.map((a) => (
        <div key={a.name} className="flex items-center gap-2 text-[12.5px]">
          <Avatar name={a.name} size={20} />
          <span className="w-[96px] truncate">{a.name}</span>
          <span className="h-1.5 flex-1 overflow-hidden rounded-full bg-control">
            <span
              className="block h-1.5 rounded-full bg-link"
              style={{ width: `${top ? (a.commits / top) * 100 : 0}%` }}
            />
          </span>
          <span className="tabular w-6 text-right text-muted">{a.commits}</span>
        </div>
      ))}
      <span className="text-[11.5px] text-muted">
        {pulse?.repositories.length
          ? `Mostly in ${pulse.repositories.join(", ")}. `
          : ""}
        Watched branches, last 7 days, counted from local refs.
      </span>
    </div>
  );
}

function Stat({ value, label }: { value: number | undefined; label: string }) {
  return (
    <div className="flex flex-col">
      <span className="tabular text-xl font-semibold">{value ?? "—"}</span>
      <span className="text-[11.5px] text-muted">{label}</span>
    </div>
  );
}

function Toggle({
  checked,
  label,
  hint,
  onChange,
}: {
  checked: boolean;
  label: string;
  hint: string;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex items-start gap-2 text-[12.5px]">
      <input
        type="checkbox"
        className="mt-0.5"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="flex flex-col gap-px">
        <span>{label}</span>
        <span className="text-[11.5px] text-muted">{hint}</span>
      </span>
    </label>
  );
}

function LastFetched({
  freshness,
  fetching,
  onFetch,
}: {
  freshness: RepositoryFreshness[];
  fetching: ReadonlySet<string>;
  onFetch: (id: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1.5 px-[18px] py-3.5">
      <span className="section-label text-fg-2">Last fetched</span>
      {freshness.map((f) => {
        const old =
          !f.last_fetch_at || olderThan(f.last_fetch_at, STALE_FETCH_DAYS);
        const busy = fetching.has(f.repository_id);
        return (
          <div
            key={f.repository_id}
            className="group flex h-6 items-center gap-2 text-[12.5px]"
          >
            <span className="flex-1 truncate">{f.name}</span>
            {f.fetch_error && (
              <span
                className="text-[11.5px] text-conflict"
                title={f.fetch_error.message}
              >
                ! failed
              </span>
            )}
            <span
              className={`text-[12px] ${old ? "text-dirty" : "text-fg-3"}`}
              title={f.last_fetch_at ?? undefined}
            >
              {busy
                ? "fetching…"
                : f.last_fetch_at
                  ? relativeTime(f.last_fetch_at)
                  : "never"}
            </span>
            <button
              type="button"
              className="btn btn-sm btn-ghost h-5 w-5 px-0 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
              aria-label={`Fetch ${f.name}`}
              title={`Fetch ${f.name}`}
              disabled={busy}
              onClick={() => onFetch(f.repository_id)}
            >
              <FetchIcon size={12} />
            </button>
          </div>
        );
      })}
      {freshness.length === 0 && (
        <span className="text-[12px] text-muted">No repositories yet.</span>
      )}
    </div>
  );
}
