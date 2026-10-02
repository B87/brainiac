import { useState } from "react";
import { shortPath } from "../lib/format";
import type {
  AppSnapshot,
  RepositorySummary,
  VaultState,
  Workspace,
  WorkspaceMember,
} from "../lib/ipc";
import { repoTone, TONE_LABEL } from "../lib/repo";
import {
  totals,
  workspaceLayout,
  workspaceRepositories,
} from "../lib/workspace";
import {
  ChevronDown,
  ChevronRight,
  GridIcon,
  NoteIcon,
  PlusIcon,
  TaskIcon,
  TodayIcon,
} from "./icons";
import type { RepoFocus } from "./RepositoryView";
import type { TaskScope } from "./TasksView";

/** A workspace dashboard's tabs. */
export type WorkspaceTab = "overview" | "activity";

/** What the main area shows. */
export type View =
  | { kind: "all" }
  | { kind: "workspace"; id: string; tab?: WorkspaceTab }
  | { kind: "repository"; id: string; workspaceId?: string; focus?: RepoFocus }
  | { kind: "today" }
  | { kind: "tasks"; scope?: TaskScope }
  | { kind: "notes"; noteId?: string };

type Props = {
  snapshot: AppSnapshot | null;
  vault: VaultState | null;
  view: View;
  onView: (view: View) => void;
  onAdd: () => void;
};

type PinItem =
  | { kind: "workspace"; workspace: Workspace }
  | { kind: "repository"; repo: RepositorySummary };

const COLLAPSED_KEY = "brainiac.sidebar.collapsed";

function readCollapsed(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]"));
  } catch {
    return new Set();
  }
}

export default function Sidebar({
  snapshot,
  vault,
  view,
  onView,
  onAdd,
}: Props) {
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const repos = snapshot?.repositories ?? [];
  const workspaces = snapshot?.workspaces ?? [];
  const byId = new Map(repos.map((r) => [r.id, r]));
  const wsById = new Map(workspaces.map((w) => [w.id, w]));

  const pins = (snapshot?.pins ?? []).flatMap((p): PinItem[] => {
    if (p.entity_type === "workspace") {
      const w = wsById.get(p.entity_id);
      return w ? [{ kind: "workspace", workspace: w }] : [];
    }
    const r = byId.get(p.entity_id);
    return r ? [{ kind: "repository", repo: r }] : [];
  });
  const pinnedRepoIds = new Set(
    pins.flatMap((p) => (p.kind === "repository" ? [p.repo.id] : [])),
  );
  const recent = (snapshot?.recent_repository_ids ?? [])
    .filter((id) => !pinnedRepoIds.has(id))
    .map((id) => byId.get(id))
    .filter((r): r is RepositorySummary => !!r)
    .slice(0, 5);
  const inWorkspace = new Set(
    workspaces.flatMap((w) => w.members.map((m) => m.repository_id)),
  );
  const standalone = repos
    .filter((r) => !inWorkspace.has(r.id))
    .sort((a, b) => a.name.localeCompare(b.name));

  const toggle = (id: string) => {
    const next = new Set(collapsed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setCollapsed(next);
    try {
      localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...next]));
    } catch {
      // Preference only.
    }
  };

  const repoCurrent = (id: string) =>
    view.kind === "repository" && view.id === id;

  return (
    <nav
      aria-label="Repositories and workspaces"
      className="flex w-[232px] shrink-0 flex-col border-r bg-sidebar"
    >
      {/* Room for the window's traffic lights; dragging here moves the window. */}
      <div data-tauri-drag-region className="h-12 shrink-0" />
      <div className="flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2.5 py-1">
        <button
          type="button"
          className="side-row"
          aria-current={view.kind === "today"}
          onClick={() => onView({ kind: "today" })}
        >
          <TodayIcon className="shrink-0" />
          <span className="flex-1">Today</span>
        </button>
        <button
          type="button"
          className="side-row"
          aria-current={view.kind === "tasks"}
          onClick={() => onView({ kind: "tasks" })}
        >
          <TaskIcon className="shrink-0" />
          <span className="flex-1">Tasks</span>
        </button>
        <button
          type="button"
          className="side-row"
          aria-current={view.kind === "notes"}
          onClick={() => onView({ kind: "notes" })}
        >
          <NoteIcon className="shrink-0" />
          <span className="flex-1">Notes</span>
          {vault?.index.state === "indexing" && (
            <span className="text-[11px] text-muted" title="Indexing notes">
              {vault.index.done}/{vault.index.total}
            </span>
          )}
        </button>
        {vault && !vault.vault && (
          <button
            type="button"
            className="side-row text-link"
            onClick={() => onView({ kind: "notes" })}
          >
            <PlusIcon size={12} className="shrink-0" />
            <span className="flex-1">Set up your vault</span>
          </button>
        )}

        <div className="mx-2 my-2 h-px bg-line" />
        <button
          type="button"
          className="side-row"
          aria-current={view.kind === "all"}
          onClick={() => onView({ kind: "all" })}
        >
          <GridIcon className="shrink-0" />
          <span className="flex-1">All repositories</span>
          <span className="text-[12px] text-muted">{repos.length}</span>
        </button>

        <SectionLabel
          title="Workspaces"
          action={
            <button
              type="button"
              className="rounded p-0.5 text-muted hover:text-fg"
              aria-label="New workspace"
              title="New workspace"
              onClick={onAdd}
            >
              <PlusIcon size={12} />
            </button>
          }
        />
        {workspaces.length === 0 && (
          <button type="button" className="side-row text-muted" onClick={onAdd}>
            <span className="text-[12px]">
              Group repositories into a workspace…
            </span>
          </button>
        )}
        {workspaces.map((w) => (
          <WorkspaceTree
            key={w.id}
            workspace={w}
            byId={byId}
            open={!collapsed.has(w.id)}
            view={view}
            onToggle={() => toggle(w.id)}
            onView={onView}
          />
        ))}

        {pins.length > 0 && <SectionLabel title="Pinned" />}
        {pins.map((p) =>
          p.kind === "workspace" ? (
            <WorkspaceRow
              key={`w:${p.workspace.id}`}
              workspace={p.workspace}
              byId={byId}
              current={view.kind === "workspace" && view.id === p.workspace.id}
              onClick={() => onView({ kind: "workspace", id: p.workspace.id })}
            />
          ) : (
            <RepoRow
              key={`r:${p.repo.id}`}
              repo={p.repo}
              current={repoCurrent(p.repo.id)}
              onClick={() => onView({ kind: "repository", id: p.repo.id })}
            />
          ),
        )}

        {recent.length > 0 && <SectionLabel title="Recent" />}
        {recent.map((r) => (
          <RepoRow
            key={r.id}
            repo={r}
            current={repoCurrent(r.id)}
            onClick={() => onView({ kind: "repository", id: r.id })}
          />
        ))}

        {standalone.length > 0 && <SectionLabel title="Not in a workspace" />}
        {standalone.map((r) => (
          <RepoRow
            key={r.id}
            repo={r}
            current={repoCurrent(r.id)}
            onClick={() => onView({ kind: "repository", id: r.id })}
          />
        ))}
      </div>
      <div className="border-t p-2.5">
        <button
          type="button"
          className="side-row h-[30px] text-fg-2"
          onClick={onAdd}
        >
          <PlusIcon className="shrink-0" />
          <span className="flex-1 truncate">Add repository or workspace</span>
          <span className="text-[11px] text-muted">⌘O</span>
        </button>
      </div>
    </nav>
  );
}

function SectionLabel({
  title,
  action,
}: {
  title: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="section-label flex items-center justify-between px-2 pt-3.5 pb-1">
      {title}
      {action}
    </div>
  );
}

function WorkspaceTree({
  workspace: w,
  byId,
  open,
  view,
  onToggle,
  onView,
}: {
  workspace: Workspace;
  byId: Map<string, RepositorySummary>;
  open: boolean;
  view: View;
  onToggle: () => void;
  onView: (view: View) => void;
}) {
  const layout = workspaceLayout(w);
  const member = (m: WorkspaceMember, indent: number, root = false) => (
    <MemberRow
      key={m.canonical_path}
      member={m}
      repo={m.repository_id ? byId.get(m.repository_id) : undefined}
      root={root}
      indent={indent}
      current={view.kind === "repository" && view.id === m.repository_id}
      onClick={() =>
        m.repository_id &&
        onView({ kind: "repository", id: m.repository_id, workspaceId: w.id })
      }
    />
  );
  return (
    <>
      <WorkspaceRow
        workspace={w}
        byId={byId}
        open={open}
        onToggle={onToggle}
        current={view.kind === "workspace" && view.id === w.id}
        onClick={() =>
          onView({
            kind: "workspace",
            id: w.id,
            // Unread news opens the Activity tab, so it is seen.
            tab: w.unseen_activity > 0 ? "activity" : undefined,
          })
        }
      />
      {open && (
        <>
          {layout.root && member(layout.root, 26, true)}
          {layout.groupLabel && layout.grouped.length > 0 && (
            <div
              className="mono flex h-6 items-center pr-2 pl-[26px] text-[11px] text-muted"
              title={w.discovery_root ? shortPath(w.discovery_root) : undefined}
            >
              {layout.groupLabel}
            </div>
          )}
          {layout.grouped.map((m) => member(m, 38))}
          {layout.others.map((m) => member(m, 26))}
          {w.members.length === 0 && (
            <div className="py-1 pl-[26px] text-[12px] text-muted">
              No repositories yet
            </div>
          )}
        </>
      )}
    </>
  );
}

function WorkspaceRow({
  workspace: w,
  byId,
  open,
  current,
  onToggle,
  onClick,
}: {
  workspace: Workspace;
  byId: Map<string, RepositorySummary>;
  open?: boolean;
  current: boolean;
  onToggle?: () => void;
  onClick: () => void;
}) {
  const t = totals(workspaceRepositories(w, byId));
  return (
    <div className="relative">
      <button
        type="button"
        className={`side-row gap-1.5 ${onToggle ? "pl-[26px]" : ""}`}
        aria-current={current}
        onClick={onClick}
        title={w.discovery_root ? shortPath(w.discovery_root) : undefined}
      >
        {!onToggle && <GridIcon size={13} className="shrink-0 text-muted" />}
        <span className="flex-1 truncate font-medium">{w.name}</span>
        {w.unseen_activity > 0 && (
          <span
            className="badge"
            title={`${w.unseen_activity} unread on the watched branches`}
          >
            {w.unseen_activity} new
          </span>
        )}
        {w.unseen_activity > 0 &&
        t.conflictedRepos === 0 ? null : t.conflictedRepos > 0 ? (
          <span
            className="text-[12px] font-semibold text-conflict"
            title={`${t.conflictedRepos} with conflicts`}
          >
            ! {t.conflictedRepos}
          </span>
        ) : t.dirtyRepos > 0 ? (
          <span className="text-[12px] text-dirty">{t.dirtyRepos} changed</span>
        ) : (
          <span className="text-[12px] text-muted">{w.members.length}</span>
        )}
      </button>
      {onToggle && (
        <button
          type="button"
          className="absolute top-1 left-1.5 rounded p-[3px] text-muted hover:text-fg"
          aria-label={open ? `Collapse ${w.name}` : `Expand ${w.name}`}
          aria-expanded={open}
          onClick={onToggle}
        >
          {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </button>
      )}
    </div>
  );
}

function MemberRow({
  member: m,
  repo,
  root,
  indent,
  current,
  onClick,
}: {
  member: WorkspaceMember;
  repo: RepositorySummary | undefined;
  root: boolean;
  indent: number;
  current: boolean;
  onClick: () => void;
}) {
  if (!repo) {
    return (
      <div
        className="side-row text-muted"
        style={{ paddingLeft: indent }}
        title={shortPath(m.canonical_path)}
      >
        <span className="dot" data-state="missing" />
        <span className="flex-1 truncate">{m.display_name}</span>
        <span className="text-[12px] italic">
          {m.status === "not_git" ? "not Git" : "missing"}
        </span>
      </div>
    );
  }
  return (
    <RepoRow
      repo={repo}
      label={m.display_name}
      root={root}
      indent={indent}
      current={current}
      onClick={onClick}
    />
  );
}

function RepoRow({
  repo,
  label,
  root = false,
  indent,
  current,
  onClick,
}: {
  repo: RepositorySummary;
  label?: string;
  root?: boolean;
  indent?: number;
  current: boolean;
  onClick: () => void;
}) {
  const tone = repoTone(repo);
  const count = repo.counts?.unique_paths ?? 0;
  return (
    <button
      type="button"
      className={`side-row ${tone === "missing" ? "text-muted" : ""}`}
      style={indent ? { paddingLeft: indent } : undefined}
      aria-current={current}
      onClick={onClick}
      title={`${shortPath(repo.display_path)} — ${TONE_LABEL[tone]}`}
    >
      <span className="dot" data-state={tone} />
      <span className="flex-1 truncate">{label ?? repo.name}</span>
      {root && <span className="tag-box">ROOT</span>}
      {tone === "missing" ? (
        <span className="text-[12px] italic">missing</span>
      ) : repo.counts?.conflicted ? (
        <span className="text-[12px] font-semibold text-conflict">
          ! {repo.counts.conflicted}
        </span>
      ) : count > 0 ? (
        <span className="text-[12px] text-dirty">{count}</span>
      ) : tone === "stale" ? (
        <span className="text-[12px] text-muted">stale</span>
      ) : null}
    </button>
  );
}
