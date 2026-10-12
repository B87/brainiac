import { useEffect, useMemo, useRef, useState } from "react";
import { relativeTime, shortAgo, shortPath } from "../lib/format";
import {
  type AppSnapshot,
  type ChangesResult,
  type CommitSummary,
  errorMessage,
  ipc,
  type RepositorySummary,
  type SuggestedMove,
  type Workspace,
  type WorkspaceMember,
} from "../lib/ipc";
import { useKeys } from "../lib/keys";
import { usePref } from "../lib/prefs";
import {
  headLabel,
  KIND_LETTER,
  kindTone,
  plural,
  repoTone,
  TONE_LABEL,
  upstreamLabel,
} from "../lib/repo";
import { useSidePanel } from "../lib/sidePanel";
import { createLatest } from "../lib/stale";
import { totals, workspaceLayout } from "../lib/workspace";
import ActivityView from "./ActivityView";
import {
  ChevronDown,
  CloseIcon,
  FetchIcon,
  GridIcon,
  MoreIcon,
  PlusIcon,
  RefreshIcon,
  RescanIcon,
  SearchIcon,
} from "./icons";
import Popover from "./Popover";
import PullRequestsTab from "./PullRequestsTab";
import { MenuButton, type RepoFocus } from "./RepositoryView";
import type { WorkspaceTab } from "./Sidebar";
import SidePanelButton from "./SidePanelButton";

type Filter = "all" | "dirty" | "conflicted" | "problems";
type Sort = "name" | "commit";

type Props = {
  snapshot: AppSnapshot;
  /** Null shows every registered repository. */
  workspace: Workspace | null;
  pinned: boolean;
  /** New repositories found by the last rescan, keyed by workspace ID. */
  discovered: Discovered | null;
  onOpen: (repositoryId: string) => void;
  /** All repositories: a workspace's own dashboard. */
  onOpenWorkspace: (workspaceId: string) => void;
  onPalette: () => void;
  onRefreshAll: () => void;
  onAdd: () => void;
  onAddToWorkspace: () => void;
  onRescan: () => void;
  onTrack: (paths: string[]) => void;
  onDismissDiscovered: () => void;
  /** Accept Rescan's suggestion that these members moved. */
  onApplyMoves: (moves: SuggestedMove[]) => void;
  onDismissMoves: () => void;
  /** Ask for the folder a repository moved to. */
  onLocate: (repositoryId: string) => void;
  onRemoveMember: (canonicalPath: string) => void;
  onRename: (name: string) => void;
  onDeleteWorkspace: () => void;
  onTogglePin: () => void;
  /** Overview or Activity; only workspaces have Activity. */
  tab: WorkspaceTab;
  onTab: (tab: WorkspaceTab) => void;
  /** Repository IDs with a fetch running. */
  fetching: ReadonlySet<string>;
  onFetch: (repositoryIds: string[]) => void;
  onOpenFocused: (repositoryId: string, focus: RepoFocus) => void;
  /** Reload the snapshot after a change made from the Activity tab. */
  onChanged: () => void;
  onError: (message: string | null) => void;
  /** Pull requests (v0.3): open one, or Settings → Accounts. */
  onOpenPullRequest: (reference: string) => void;
  onOpenSettings: () => void;
};

/** Result of a rescan: untracked repositories, skipped plain folders, and suggested moves. */
export type Discovered = {
  workspaceId: string;
  repositories: Array<{ name: string; path: string }>;
  skipped: number;
  moves: SuggestedMove[];
};

type Row =
  | { kind: "repo"; repo: RepositorySummary; label: string; root: boolean }
  | { kind: "missing"; member: WorkspaceMember }
  | { kind: "group"; label: string; tracked: number };

export default function Dashboard(props: Props) {
  const { snapshot, workspace, onOpen } = props;
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<Sort>("name");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  /** All repositories only: every workspace, one of them, or none ("none"). */
  const [wsFilter, setWsFilter] = usePref<string>(
    "brainiac.allRepositories.workspace",
    "all",
  );
  const [wsMenuOpen, setWsMenuOpen] = useState(false);
  const queryRef = useRef<HTMLInputElement>(null);
  const tab: WorkspaceTab = workspace ? props.tab : "overview";
  const { open: panelOpen, toggle: togglePanel } = useSidePanel();

  const byId = useMemo(
    () => new Map(snapshot.repositories.map((r) => [r.id, r])),
    [snapshot.repositories],
  );

  /** The workspaces each repository belongs to, for All repositories. */
  const memberOf = useMemo(() => {
    const map = new Map<string, Workspace[]>();
    for (const w of snapshot.workspaces)
      for (const m of w.members)
        if (m.repository_id)
          map.set(m.repository_id, [...(map.get(m.repository_id) ?? []), w]);
    return map;
  }, [snapshot.workspaces]);
  // A workspace that was deleted since it was chosen shows everything.
  const shownWorkspace =
    wsFilter === "all" || wsFilter === "none"
      ? null
      : (snapshot.workspaces.find((w) => w.id === wsFilter) ?? null);
  const wsChoice = shownWorkspace
    ? shownWorkspace.id
    : wsFilter === "none"
      ? "none"
      : "all";

  // Every repository in scope, for the totals and filter counts.
  const scopeRepos = useMemo(() => {
    const scope = workspace ?? shownWorkspace;
    if (scope) {
      const ids = new Set(scope.members.map((m) => m.repository_id));
      return snapshot.repositories.filter((r) => ids.has(r.id));
    }
    if (wsChoice === "none")
      return snapshot.repositories.filter((r) => !memberOf.has(r.id));
    return snapshot.repositories;
  }, [snapshot.repositories, workspace, shownWorkspace, wsChoice, memberOf]);

  const t = totals(scopeRepos);

  const rows = useMemo<Row[]>(() => {
    const q = query.trim().toLowerCase();
    const keep = (r: RepositorySummary, label: string) => {
      if (
        q &&
        !label.toLowerCase().includes(q) &&
        !r.name.toLowerCase().includes(q) &&
        !r.display_path.toLowerCase().includes(q)
      )
        return false;
      if (filter === "dirty") return (r.counts?.unique_paths ?? 0) > 0;
      if (filter === "conflicted") return (r.counts?.conflicted ?? 0) > 0;
      if (filter === "problems")
        return (
          r.state === "stale" || r.state === "error" || r.state === "missing"
        );
      return true;
    };
    const order = (a: RepositorySummary, b: RepositorySummary) =>
      sort === "name"
        ? a.name.localeCompare(b.name)
        : (b.last_commit_at ?? "").localeCompare(a.last_commit_at ?? "");
    const toRows = (members: WorkspaceMember[], root = false): Row[] => {
      const repoRows: Array<Row & { kind: "repo" }> = [];
      const missing: Row[] = [];
      for (const m of members) {
        const r = m.repository_id ? byId.get(m.repository_id) : undefined;
        // A folder that is gone gets the relocate/remove row, not stale status columns.
        if (!r || r.state === "missing" || m.status !== "ok") {
          const matches = !q || m.display_name.toLowerCase().includes(q);
          if (matches && (filter === "all" || filter === "problems"))
            missing.push({ kind: "missing", member: m });
        } else if (keep(r, m.display_name))
          repoRows.push({ kind: "repo", repo: r, label: m.display_name, root });
      }
      repoRows.sort((a, b) => order(a.repo, b.repo));
      return [...repoRows, ...missing];
    };

    if (!workspace)
      return scopeRepos
        .filter((r) => keep(r, r.name))
        .sort(order)
        .map((r) => ({ kind: "repo", repo: r, label: r.name, root: false }));

    const layout = workspaceLayout(workspace);
    const out: Row[] = [];
    if (layout.root) out.push(...toRows([layout.root], true));
    if (layout.groupLabel && layout.grouped.length) {
      const grouped = toRows(layout.grouped);
      if (grouped.length)
        out.push(
          {
            kind: "group",
            label: layout.groupLabel,
            tracked: layout.grouped.length,
          },
          ...grouped,
        );
    }
    out.push(...toRows(layout.others));
    return out;
  }, [workspace, scopeRepos, byId, query, filter, sort]);

  const repoRows = rows.filter(
    (r): r is Row & { kind: "repo" } => r.kind === "repo",
  );
  const selected =
    repoRows.find((r) => r.repo.id === selectedId)?.repo ??
    repoRows[0]?.repo ??
    null;

  const move = (delta: number) => {
    const i = repoRows.findIndex((r) => r.repo === selected);
    const next =
      repoRows[Math.max(0, Math.min(repoRows.length - 1, i + delta))];
    if (next) setSelectedId(next.repo.id);
  };
  useKeys(
    {
      j: () => move(1),
      k: () => move(-1),
      ArrowDown: () => move(1),
      ArrowUp: () => move(-1),
      Enter: () => selected && onOpen(selected.id),
      "/": () => queryRef.current?.focus(),
    },
    tab === "overview",
  );
  useKeys({
    "mod+1": () => props.onTab("overview"),
    "mod+2": () => workspace && props.onTab("activity"),
    "mod+3": () => workspace && props.onTab("pull_requests"),
  });

  /** A missing member with a registration can be located; a plain folder cannot. */
  const locateMember = (m: WorkspaceMember) => {
    const id = m.repository_id;
    return id && m.status !== "not_git" ? () => props.onLocate(id) : undefined;
  };

  const discovered =
    workspace && props.discovered?.workspaceId === workspace.id
      ? props.discovered
      : null;
  const scanLabel = workspace ? workspaceLayout(workspace).groupLabel : null;
  const title = workspace?.name ?? "All repositories";

  if (snapshot.repositories.length === 0 && !workspace)
    return <Welcome onAdd={props.onAdd} />;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header
        data-tauri-drag-region
        className="@container flex h-12 shrink-0 items-center gap-3 border-b bg-header pr-3 pl-4 pl-lead"
      >
        {/* The breadcrumb never shrinks: the totals give way first. */}
        <nav
          aria-label="Location"
          className="flex shrink-0 items-center gap-0.5"
        >
          <button
            type="button"
            className="btn btn-ghost shrink-0 px-2 text-fg"
            aria-label={`Switch workspace: ${title}`}
            title="Switch repository or workspace (⌘K)"
            onClick={props.onPalette}
          >
            <span className="font-semibold">{title}</span>
            <ChevronDown size={11} className="text-muted" />
          </button>
          {!workspace && (
            <>
              <span aria-hidden="true" className="text-faint">
                ›
              </span>
              <WorkspaceMenu
                workspaces={snapshot.workspaces}
                choice={wsChoice}
                counts={(id) =>
                  id === "all"
                    ? snapshot.repositories.length
                    : id === "none"
                      ? snapshot.repositories.filter((r) => !memberOf.has(r.id))
                          .length
                      : (snapshot.workspaces
                          .find((w) => w.id === id)
                          ?.members.filter((m) => m.repository_id).length ?? 0)
                }
                open={wsMenuOpen}
                onOpen={setWsMenuOpen}
                onChoose={(id) => {
                  setWsMenuOpen(false);
                  setWsFilter(id);
                }}
                onOpenDashboard={(id) => {
                  setWsMenuOpen(false);
                  props.onOpenWorkspace(id);
                }}
              />
            </>
          )}
        </nav>
        {workspace ? (
          <div role="tablist" aria-label="Workspace views" className="seg ml-2">
            <button
              type="button"
              role="tab"
              aria-selected={tab === "overview"}
              title="Overview (⌘1)"
              onClick={() => props.onTab("overview")}
            >
              Overview
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={tab === "activity"}
              title="Activity (⌘2)"
              onClick={() => props.onTab("activity")}
            >
              Activity
              {workspace.unseen_activity > 0 && (
                <span className="badge">{workspace.unseen_activity}</span>
              )}
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={tab === "pull_requests"}
              title="Pull requests (⌘3)"
              onClick={() => props.onTab("pull_requests")}
            >
              PRs
            </button>
          </div>
        ) : (
          <span className="ml-1 min-w-0 overflow-hidden @max-[800px]:hidden">
            <Totals t={t} />
          </span>
        )}
        <div data-tauri-drag-region className="h-full flex-1" />
        <button
          type="button"
          className="btn"
          disabled={
            scopeRepos.length === 0 ||
            scopeRepos.every((r) => props.fetching.has(r.id))
          }
          aria-label="Fetch all"
          title="Fetch every repository shown: updates remote-tracking branches and tags only"
          onClick={() => props.onFetch(scopeRepos.map((r) => r.id))}
        >
          <FetchIcon
            size={13}
            className={
              scopeRepos.some((r) => props.fetching.has(r.id))
                ? "animate-pulse"
                : ""
            }
          />
          <span className="@max-[1000px]:hidden">
            {scopeRepos.some((r) => props.fetching.has(r.id))
              ? "Fetching…"
              : "Fetch all"}
          </span>
        </button>
        {tab === "overview" && workspace?.discovery_mode === "discovered" && (
          <button
            type="button"
            className="btn"
            aria-label={`Rescan ${scanLabel}`}
            title={`Look for new repositories in ${scanLabel}`}
            onClick={props.onRescan}
          >
            <RescanIcon size={13} />
            <span className="hidden xl:inline">Rescan {scanLabel}</span>
          </button>
        )}
        {tab === "activity" ? null : workspace ? (
          <button
            type="button"
            className="btn"
            aria-label="Add repositories"
            title="Add repositories to this workspace"
            onClick={props.onAddToWorkspace}
          >
            <PlusIcon size={13} />
            <span className="hidden xl:inline">Add repositories…</span>
          </button>
        ) : (
          <button
            type="button"
            className="btn"
            aria-label="Add…"
            title="Add a repository or workspace (⌘O)"
            onClick={props.onAdd}
          >
            <PlusIcon size={13} />
            <span className="@max-[1000px]:hidden">Add…</span>
          </button>
        )}
        <button
          type="button"
          className="btn btn-primary"
          aria-label="Refresh all"
          title="Refresh all (⌘R)"
          onClick={props.onRefreshAll}
        >
          <RefreshIcon size={13} />
          <span className="@max-[800px]:hidden">Refresh all</span>
          <span className="text-[11px] opacity-80 @max-[1000px]:hidden">
            ⌘R
          </span>
        </button>
        {workspace && (
          <div className="relative">
            <button
              type="button"
              className="btn btn-ghost icon-btn"
              aria-label="Workspace actions"
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              onClick={() => setMenuOpen(!menuOpen)}
            >
              <MoreIcon />
            </button>
            {menuOpen && (
              <Popover align="right" onClose={() => setMenuOpen(false)}>
                <MenuButton
                  onClick={() => {
                    setMenuOpen(false);
                    setRenaming(true);
                  }}
                >
                  Rename…
                </MenuButton>
                <MenuButton
                  onClick={() => {
                    setMenuOpen(false);
                    props.onTogglePin();
                  }}
                >
                  {props.pinned ? "Unpin" : "Pin to sidebar"}
                </MenuButton>
                <div className="menu-sep" />
                <MenuButton
                  onClick={() => {
                    setMenuOpen(false);
                    props.onDeleteWorkspace();
                  }}
                >
                  Delete workspace…
                </MenuButton>
              </Popover>
            )}
          </div>
        )}
        <SidePanelButton open={panelOpen} onToggle={togglePanel} />
      </header>

      {workspace && tab === "pull_requests" ? (
        <PullRequestsTab
          key={workspace.id}
          scope={{ kind: "workspace", workspace }}
          snapshot={snapshot}
          onOpen={props.onOpenPullRequest}
          onOpenSettings={props.onOpenSettings}
          onChanged={props.onChanged}
          onError={props.onError}
        />
      ) : workspace && tab === "activity" ? (
        <ActivityView
          snapshot={snapshot}
          workspace={workspace}
          fetching={props.fetching}
          onFetch={props.onFetch}
          onOpen={props.onOpenFocused}
          onChanged={props.onChanged}
          onError={props.onError}
        />
      ) : (
        <div className="flex min-h-0 flex-1">
          <section
            aria-label="Repositories"
            className={`@container flex min-w-0 flex-1 flex-col gap-3.5 px-6 @max-[760px]:px-4 ${workspace ? "pt-5 @max-[1000px]:pt-3" : "pt-3.5"}`}
          >
            {workspace && (
              // Below 1000px the title and totals share one line.
              <div className="flex items-end gap-4 @max-[1000px]:items-center">
                <div className="flex min-w-0 flex-1 flex-col gap-1 @max-[1000px]:flex-row @max-[1000px]:items-baseline @max-[1000px]:gap-3">
                  {renaming ? (
                    <RenameField
                      initial={workspace.name}
                      onDone={(name) => {
                        setRenaming(false);
                        if (name && name !== workspace.name)
                          props.onRename(name);
                      }}
                    />
                  ) : (
                    <h1 className="m-0 truncate text-[22px] font-semibold @max-[1000px]:shrink-0 @max-[1000px]:text-[16px]">
                      {title}
                    </h1>
                  )}
                  <span className="mono truncate text-[12px] text-muted @max-[1000px]:hidden">
                    {describe(workspace, scopeRepos.length)}
                  </span>
                  <span className="hidden min-w-0 @max-[1000px]:flex">
                    <Totals t={t} />
                  </span>
                </div>
                <div className="flex gap-[22px] @max-[1000px]:hidden">
                  <Kpi value={t.changedFiles} label="changed files" />
                  <Kpi
                    value={t.conflicts}
                    label="conflicts"
                    tone={t.conflicts ? "text-conflict" : "text-muted"}
                  />
                  <Kpi
                    value={`↑${t.ahead} ↓${t.behind}`}
                    label="vs local upstreams"
                  />
                </div>
              </div>
            )}

            {discovered && discovered.moves.length > 0 && (
              <MovesBanner
                workspace={workspace}
                moves={discovered.moves}
                onApply={props.onApplyMoves}
                onDismiss={props.onDismissMoves}
              />
            )}

            {discovered &&
              (discovered.repositories.length > 0 ||
                discovered.skipped > 0) && (
                <DiscoveredBanner
                  workspace={workspace}
                  discovered={discovered}
                  onTrack={props.onTrack}
                  onReview={props.onAddToWorkspace}
                  onDismiss={props.onDismissDiscovered}
                />
              )}

            <div className="flex items-center gap-2">
              <fieldset
                aria-label="Filter"
                className="m-0 flex gap-1.5 border-0 p-0"
              >
                <FilterChip
                  label="All"
                  count={scopeRepos.length}
                  active={filter === "all"}
                  onClick={() => setFilter("all")}
                />
                <FilterChip
                  label="With changes"
                  short="Changes"
                  dot="dirty"
                  count={t.dirtyRepos}
                  active={filter === "dirty"}
                  onClick={() => setFilter("dirty")}
                />
                <FilterChip
                  label="Conflicted"
                  dot="conflict"
                  count={t.conflictedRepos}
                  active={filter === "conflicted"}
                  onClick={() => setFilter("conflicted")}
                />
                <FilterChip
                  label="Stale or missing"
                  short="Stale"
                  dot="missing"
                  count={
                    t.problemRepos +
                    (workspace?.members.filter((m) => !m.repository_id)
                      .length ?? 0)
                  }
                  active={filter === "problems"}
                  onClick={() => setFilter("problems")}
                />
              </fieldset>
              <span className="flex-1" />
              <label className="search w-[220px] @max-[1000px]:w-[160px] @max-[640px]:w-[110px]">
                <SearchIcon />
                <input
                  ref={queryRef}
                  type="text"
                  placeholder="Filter by name or path"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                />
              </label>
              <button
                type="button"
                className="btn btn-sm h-7"
                onClick={() => setSort(sort === "name" ? "commit" : "name")}
                title="Change sort order"
              >
                <span className="@max-[760px]:hidden">Sort: </span>
                {sort === "name" ? "Name" : "Latest commit"}
                <ChevronDown size={10} className="text-muted" />
              </button>
            </div>

            <div className="mb-4 flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border">
              <div
                className={`${workspace ? GRID : GRID_ALL} section-label border-b bg-panel px-3.5 py-2`}
              >
                <span>Repository</span>
                {!workspace && (
                  <span className={WORKSPACE_CELL}>Workspace</span>
                )}
                <span>Branch</span>
                <span className={UPSTREAM_CELL}>Upstream</span>
                <span>Changes</span>
                <span className={COMMIT_CELL}>Commit</span>
                <span className={`text-right ${COMMIT_CELL}`}>Checked</span>
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto">
                {rows.length === 0 && (
                  <div className="p-4 text-muted">
                    {scopeRepos.length === 0 && workspace
                      ? "No repositories in this workspace yet. Use Add repositories…"
                      : "No repositories match."}
                  </div>
                )}
                {rows.map((row) =>
                  row.kind === "group" ? (
                    <div
                      key={`g:${row.label}`}
                      className="flex items-center gap-2 border-t bg-panel px-3.5 pt-2.5 pb-1.5"
                    >
                      <span className="mono text-[11.5px] text-fg-2">
                        {row.label}
                      </span>
                      <span className="text-[11.5px] text-muted">
                        {row.tracked} tracked
                        {discovered?.repositories.length
                          ? ` · ${discovered.repositories.length} not tracked`
                          : ""}
                      </span>
                    </div>
                  ) : row.kind === "missing" ? (
                    <MissingRow
                      key={row.member.canonical_path}
                      member={row.member}
                      path={displayPath(workspace, row.member.canonical_path)}
                      onRemove={() =>
                        props.onRemoveMember(row.member.canonical_path)
                      }
                      onLocate={locateMember(row.member)}
                    />
                  ) : (
                    <RepoRow
                      key={row.repo.id}
                      row={row}
                      path={displayPath(workspace, row.repo.display_path)}
                      selected={row.repo === selected}
                      workspaces={
                        workspace
                          ? undefined
                          : (memberOf.get(row.repo.id) ?? []).map((w) => w.name)
                      }
                      onSelect={() => setSelectedId(row.repo.id)}
                      onOpen={() => onOpen(row.repo.id)}
                    />
                  ),
                )}
              </div>
              <div className="flex flex-wrap gap-3.5 border-t bg-panel px-3.5 py-2 text-[11.5px] text-muted">
                <Legend letter="S" tone="ren" label="staged" />
                <Legend letter="M" tone="mod" label="unstaged" />
                <Legend letter="?" tone="other" label="untracked" />
                <Legend letter="!" tone="del" label="conflicted" />
                <span className="flex-1" />
                <span>
                  File totals count each path once.
                  {workspace?.root_repository_id &&
                    " The root's counts never include its member repositories."}
                </span>
              </div>
            </div>
          </section>

          {selected && panelOpen && (
            <Peek
              key={selected.id}
              repo={selected}
              fetching={props.fetching.has(selected.id)}
              onFetch={() => props.onFetch([selected.id])}
              onOpen={() => onOpen(selected.id)}
              onLocate={() => props.onLocate(selected.id)}
              onClose={togglePanel}
              onError={props.onError}
            />
          )}
        </div>
      )}
    </div>
  );
}

/**
 * The table's columns, dropped by priority as the table narrows (SPEC.md,
 * Main window): Checked and Commit, then the path line, then Workspace, then
 * Upstream, whose ↑↓ joins the Branch cell. Repository, Branch, and Changes
 * always stay. Measured on the Repositories section (`@container`).
 */
const GRID =
  "grid grid-cols-[minmax(0,1fr)_minmax(90px,150px)_74px_minmax(150px,206px)_64px_70px] @max-[1100px]:grid-cols-[minmax(160px,1fr)_minmax(90px,150px)_74px_minmax(150px,206px)] @max-[680px]:grid-cols-[minmax(160px,1fr)_minmax(90px,150px)_minmax(110px,206px)] gap-x-3.5";
/** All repositories adds the Workspace column after the name. */
const GRID_ALL =
  "grid grid-cols-[minmax(0,1fr)_minmax(80px,130px)_minmax(90px,150px)_74px_minmax(150px,206px)_64px_70px] @max-[1100px]:grid-cols-[minmax(160px,1fr)_minmax(80px,130px)_minmax(90px,150px)_74px_minmax(150px,206px)] @max-[760px]:grid-cols-[minmax(160px,1fr)_minmax(90px,150px)_74px_minmax(110px,206px)] @max-[680px]:grid-cols-[minmax(160px,1fr)_minmax(90px,150px)_minmax(110px,206px)] gap-x-3.5";
/** Cells that go, matching the columns above. */
const COMMIT_CELL = "@max-[1100px]:hidden";
const WORKSPACE_CELL = "@max-[760px]:hidden";
const UPSTREAM_CELL = "@max-[680px]:hidden";

/** Paths inside a discovered workspace's folder are shown relative to it. */
function displayPath(workspace: Workspace | null, path: string): string {
  const root = workspace?.discovery_root;
  if (root && path.startsWith(`${root}/`)) return path.slice(root.length + 1);
  return shortPath(path);
}

function describe(workspace: Workspace | null, count: number): string {
  if (!workspace) return plural(count, "repository", "repositories");
  if (workspace.discovery_mode === "manual")
    return `${plural(count, "repository", "repositories")}, picked by hand`;
  const root = workspace.discovery_root
    ? shortPath(workspace.discovery_root)
    : "";
  const label = workspaceLayout(workspace).groupLabel;
  return workspace.root_repository_id
    ? `${root} · root repository + repositories in ${label}`
    : `${root} · repositories in ${label}`;
}

/**
 * All repositories' totals, in the header where a workspace has its tabs.
 * The header narrows them first: shorter words, then conflicts only when
 * there are some, then (below 800px, the caller) none at all.
 */
function Totals({ t }: { t: ReturnType<typeof totals> }) {
  return (
    <span className="tabular flex min-w-0 items-center gap-3 truncate text-[12px] text-muted">
      <span>
        <span className="font-semibold text-fg">{t.changedFiles}</span> changed
        <span className="@max-[1100px]:hidden"> files</span>
      </span>
      <span className={t.conflicts ? "" : "@max-[1000px]:hidden"}>
        <span
          className={`font-semibold ${t.conflicts ? "text-conflict" : "text-fg"}`}
        >
          {t.conflicts}
        </span>{" "}
        {t.conflicts === 1 ? "conflict" : "conflicts"}
      </span>
      <span title="Ahead and behind their upstreams, as of the last fetch">
        <span className="font-semibold text-fg">
          ↑{t.ahead} ↓{t.behind}
        </span>
        <span className="@max-[1100px]:hidden"> vs upstreams</span>
      </span>
    </span>
  );
}

/** Shows every repository, one workspace's, or those in none. */
function WorkspaceMenu({
  workspaces,
  choice,
  counts,
  open,
  onOpen,
  onChoose,
  onOpenDashboard,
}: {
  workspaces: Workspace[];
  /** "all", "none", or a workspace ID. */
  choice: string;
  counts: (id: string) => number;
  open: boolean;
  onOpen: (open: boolean) => void;
  onChoose: (id: string) => void;
  /** The chosen workspace's own dashboard: Activity and PRs live there. */
  onOpenDashboard: (id: string) => void;
}) {
  const chosen = workspaces.find((w) => w.id === choice);
  const label =
    choice === "all"
      ? "All workspaces"
      : choice === "none"
        ? "Not in a workspace"
        : (workspaces.find((w) => w.id === choice)?.name ?? "All workspaces");
  const item = (id: string, name: string) => (
    <MenuButton key={id} onClick={() => onChoose(id)}>
      <span className="w-3.5 shrink-0 text-accent">
        {choice === id ? "✓" : ""}
      </span>
      <span className="flex-1 truncate">{name}</span>
      <span className="tabular text-muted">{counts(id)}</span>
    </MenuButton>
  );
  return (
    <div className="relative shrink-0">
      <button
        type="button"
        className={`btn btn-ghost px-2 ${choice === "all" ? "text-fg-2" : "text-fg"}`}
        aria-label={`Workspace: ${label}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => onOpen(!open)}
      >
        <GridIcon size={12} className="text-fg-2" />
        {label}
        <ChevronDown size={10} className="text-muted" />
      </button>
      {open && (
        <Popover onClose={() => onOpen(false)}>
          {item("all", "All workspaces")}
          <div className="menu-sep" />
          {workspaces.map((w) => item(w.id, w.name))}
          {workspaces.length > 0 && <div className="menu-sep" />}
          {item("none", "Not in a workspace")}
          {chosen && (
            <>
              <div className="menu-sep" />
              <MenuButton onClick={() => onOpenDashboard(chosen.id)}>
                <span className="w-3.5 shrink-0" />
                <span className="flex-1 truncate">
                  Open {chosen.name}'s dashboard
                </span>
              </MenuButton>
            </>
          )}
        </Popover>
      )}
    </div>
  );
}

function Kpi({
  value,
  label,
  tone = "",
}: {
  value: number | string;
  label: string;
  tone?: string;
}) {
  return (
    <div className="flex flex-col items-end">
      <span className={`tabular text-xl font-semibold ${tone}`}>{value}</span>
      <span className="text-[11.5px] text-muted">{label}</span>
    </div>
  );
}

/**
 * A state filter. As the section narrows its label shortens to `short`, then
 * (with a dot to stand for it) only the count shows; the name stays in the
 * tooltip and the accessible name.
 */
function FilterChip({
  label,
  short,
  count,
  dot,
  active,
  onClick,
}: {
  label: string;
  short?: string;
  count: number;
  dot?: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="chip shrink-0"
      aria-pressed={active}
      aria-label={`${label} ${count}`}
      title={label}
      onClick={onClick}
    >
      {dot && <span className="dot" data-state={dot} />}
      <span className="@max-[760px]:hidden">{label}</span>
      <span
        className={`hidden @max-[760px]:inline ${dot ? "@max-[640px]:hidden" : ""}`}
      >
        {short ?? label}
      </span>
      <span className="count">{count}</span>
    </button>
  );
}

function Legend({
  letter,
  tone,
  label,
}: {
  letter: string;
  tone: string;
  label: string;
}) {
  return (
    <span>
      <b className="mono" style={{ color: `var(--k-${tone}-fg)` }}>
        {letter}
      </b>{" "}
      {label}
    </span>
  );
}

function CountChip({
  letter,
  tone,
  n,
}: {
  letter: string;
  tone: string;
  n: number;
}) {
  return (
    <span className="kind count-chip text-[11px]" data-tone={tone}>
      <b>{letter}</b>
      {n}
    </span>
  );
}

function RepoRow({
  row,
  path,
  selected,
  workspaces,
  onSelect,
  onOpen,
}: {
  row: Row & { kind: "repo" };
  path: string;
  selected: boolean;
  /** All repositories: the workspaces it is in, for the Workspace column. */
  workspaces?: string[];
  onSelect: () => void;
  onOpen: () => void;
}) {
  const r = row.repo;
  const tone = repoTone(r);
  const c = r.counts;
  const up = upstreamLabel(r);
  const files = c?.unique_paths ?? 0;
  return (
    <button
      type="button"
      className={`list-row ${workspaces ? GRID_ALL : GRID} min-h-[52px] items-center border-t border-t-line-soft pr-3.5 pl-[11px] @max-[860px]:min-h-9`}
      aria-current={selected}
      onClick={onSelect}
      onDoubleClick={onOpen}
      title={`${path}\nDouble-click or press Enter to open`}
    >
      <span className="flex min-w-0 items-center gap-2.5">
        <span className="dot" data-state={tone} title={TONE_LABEL[tone]} />
        <span className="flex min-w-0 flex-col gap-px">
          <span className="flex items-center gap-1.5">
            <span className="truncate font-semibold">{row.label}</span>
            {row.root && <span className="tag-box">ROOT</span>}
          </span>
          <span className="meta mono truncate text-[11px] text-muted @max-[860px]:hidden">
            {path}
          </span>
          {r.error && r.state === "error" && (
            <span className="truncate text-[11px] text-conflict">
              {r.error.message}
            </span>
          )}
        </span>
      </span>
      {workspaces && (
        <span
          className={`truncate text-[12px] text-fg-2 ${WORKSPACE_CELL}`}
          title={workspaces.join(", ") || "Not in a workspace"}
        >
          {workspaces.length === 0 ? (
            <span className="text-faint">—</span>
          ) : (
            <>
              {workspaces[0]}
              {workspaces.length > 1 && (
                <span className="text-muted"> +{workspaces.length - 1}</span>
              )}
            </>
          )}
        </span>
      )}
      <span
        className={`mono truncate text-[12px] ${r.head?.kind === "branch" ? "" : "text-fg-3"}`}
      >
        {headLabel(r)}
        {/* Upstream's column is gone: its ↑↓ comes here. */}
        {up && up !== "in sync" && (
          <span
            className={`hidden @max-[680px]:inline ${r.upstream?.behind ? "text-dirty" : "text-link"}`}
          >
            {" "}
            {up}
          </span>
        )}
      </span>
      <span
        className={`mono text-[12px] ${UPSTREAM_CELL} ${
          !up
            ? "text-faint"
            : up === "in sync"
              ? "text-clean"
              : r.upstream?.behind
                ? "text-dirty"
                : "text-link"
        }`}
      >
        {up ?? "—"}
      </span>
      <span className="flex items-center gap-1.5">
        <span
          className={`tabular min-w-11 text-[12px] font-semibold ${files ? "" : "font-normal text-faint"}`}
        >
          {c ? (files ? plural(files, "file") : "clean") : "—"}
        </span>
        {!!c?.conflicted && (
          <CountChip letter="!" tone="del" n={c.conflicted} />
        )}
        {!!c?.staged && <CountChip letter="S" tone="ren" n={c.staged} />}
        {!!c?.unstaged && <CountChip letter="M" tone="mod" n={c.unstaged} />}
        {!!c?.untracked && (
          <CountChip letter="?" tone="other" n={c.untracked} />
        )}
      </span>
      <span className={`text-[12px] text-fg-2 ${COMMIT_CELL}`}>
        {shortAgo(r.last_commit_at)}
      </span>
      <span
        className={`text-right text-[12px] ${COMMIT_CELL} ${tone === "stale" ? "text-dirty" : "text-muted"}`}
      >
        {shortAgo(r.last_checked_at) || "—"}
        {tone === "stale" && " · stale"}
      </span>
    </button>
  );
}

function MissingRow({
  member,
  path,
  onRemove,
  onLocate,
}: {
  member: WorkspaceMember;
  path: string;
  onRemove: () => void;
  /** Absent for a non-Git folder, which has no registration to relocate. */
  onLocate?: () => void;
}) {
  return (
    <div className="flex min-h-[52px] items-center gap-3.5 border-t border-l-[3px] border-t-line-soft border-l-transparent pr-3.5 pl-[11px]">
      <span className="flex min-w-0 flex-1 items-center gap-2.5">
        <span className="dot" data-state="missing" />
        <span className="flex min-w-0 flex-col gap-px">
          <span className="font-medium text-fg-2">{member.display_name}</span>
          <span className="truncate text-[11.5px] text-muted">
            {member.status === "not_git"
              ? "Not a Git repository: "
              : "Folder not found at "}
            <span className="mono">{path}</span>
            {member.status !== "not_git" &&
              ". Registration and history are kept."}
          </span>
        </span>
      </span>
      {onLocate && (
        <button
          type="button"
          className="btn btn-sm btn-primary"
          onClick={onLocate}
        >
          Locate…
        </button>
      )}
      <button type="button" className="btn btn-sm" onClick={onRemove}>
        Remove from workspace
      </button>
    </div>
  );
}

/** Rescan's suggestion that missing members moved to untracked folders. */
function MovesBanner({
  workspace,
  moves,
  onApply,
  onDismiss,
}: {
  workspace: Workspace | null;
  moves: SuggestedMove[];
  onApply: (moves: SuggestedMove[]) => void;
  onDismiss: () => void;
}) {
  return (
    <div className="flex items-center gap-3 rounded-lg border border-info-line bg-info-bg px-3 py-2.5 text-info-fg">
      <RescanIcon size={15} className="shrink-0" />
      <span className="flex-1">
        {moves.length === 1 ? (
          <>
            {moves[0].name} seems to have moved to{" "}
            <span className="mono">{displayPath(workspace, moves[0].to)}</span>.
          </>
        ) : (
          `${moves.length} repositories seem to have moved: ${moves
            .map((m) => `${m.name} → ${displayPath(workspace, m.to)}`)
            .join(", ")}.`
        )}
      </span>
      <button
        type="button"
        className="btn btn-sm btn-primary"
        onClick={() => onApply(moves)}
      >
        {moves.length === 1 ? "Update" : "Update all"}
      </button>
      <button
        type="button"
        className="btn btn-sm btn-ghost px-1.5"
        aria-label="Dismiss"
        onClick={onDismiss}
      >
        <CloseIcon />
      </button>
    </div>
  );
}

function DiscoveredBanner({
  workspace,
  discovered,
  onTrack,
  onReview,
  onDismiss,
}: {
  workspace: Workspace | null;
  discovered: Discovered;
  onTrack: (paths: string[]) => void;
  onReview: () => void;
  onDismiss: () => void;
}) {
  const repos = discovered.repositories;
  const skipped =
    discovered.skipped > 0
      ? ` ${discovered.skipped === 1 ? "One plain folder was" : `${discovered.skipped} plain folders were`} skipped (not a Git repository).`
      : "";
  return (
    <div className="flex items-center gap-3 rounded-lg border border-info-line bg-info-bg px-3 py-2.5 text-info-fg">
      <RescanIcon size={15} className="shrink-0" />
      <span className="flex-1">
        {repos.length === 0 ? (
          "No new repositories found."
        ) : repos.length === 1 ? (
          <>
            New repository found in{" "}
            <span className="mono">
              {displayPath(workspace, repos[0].path)}
            </span>
            .
          </>
        ) : (
          `${repos.length} new repositories found: ${repos.map((r) => r.name).join(", ")}.`
        )}
        {skipped}
      </span>
      {repos.length > 0 && (
        <button
          type="button"
          className="btn btn-sm btn-primary"
          onClick={() => onTrack(repos.map((r) => r.path))}
        >
          {repos.length === 1 ? `Track ${repos[0].name}` : "Track all"}
        </button>
      )}
      {repos.length > 1 && (
        <button type="button" className="btn btn-sm" onClick={onReview}>
          Review
        </button>
      )}
      <button
        type="button"
        className="btn btn-sm btn-ghost px-1.5"
        aria-label="Dismiss"
        onClick={onDismiss}
      >
        <CloseIcon />
      </button>
    </div>
  );
}

function RenameField({
  initial,
  onDone,
}: {
  initial: string;
  onDone: (name: string | null) => void;
}) {
  const [value, setValue] = useState(initial);
  return (
    <input
      // biome-ignore lint/a11y/noAutofocus: the field appears because the user chose Rename.
      autoFocus
      aria-label="Workspace name"
      className="text-input h-9 w-80 text-[18px] font-semibold"
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onBlur={() => onDone(value.trim() || null)}
      onKeyDown={(e) => {
        if (e.key === "Enter") onDone(value.trim() || null);
        if (e.key === "Escape") onDone(null);
      }}
    />
  );
}

/** Side panel for the selected row: status, last commit, and a peek at its changes. */
function Peek({
  repo: r,
  fetching,
  onFetch,
  onOpen,
  onLocate,
  onClose,
  onError,
}: {
  repo: RepositorySummary;
  fetching: boolean;
  onFetch: () => void;
  onOpen: () => void;
  onLocate: () => void;
  onClose: () => void;
  onError: (message: string | null) => void;
}) {
  const [changes, setChanges] = useState<ChangesResult | null>(null);
  const [last, setLast] = useState<CommitSummary | null | undefined>(undefined);
  const latest = useRef(createLatest()).current;
  const commitLatest = useRef(createLatest()).current;
  const tone = repoTone(r);
  const available =
    r.state !== "missing" && !(r.state === "error" && !r.counts);

  // Reload the peek when the summary's observation changes.
  useEffect(() => {
    if (!available || !r.last_checked_at) return;
    void latest.run(
      () => ipc.listChanges(r.id),
      (result) => {
        if (result.repository_id === r.id) setChanges(result);
      },
      () => setChanges(null),
    );
    void commitLatest.run(
      () =>
        ipc.listCommits({
          repository_id: r.id,
          ref: null,
          filter: null,
          cursor: null,
          limit: 1,
        }),
      (page) => setLast(page.items[0] ?? null),
      () => setLast(null),
    );
  }, [r.id, r.last_checked_at, available, latest, commitLatest]);

  const run = (p: Promise<unknown>) =>
    void p.catch((e) => onError(errorMessage(e)));
  const entries = changes?.entries ?? [];
  const up = upstreamLabel(r);

  return (
    <aside
      aria-label="Selected repository"
      className="flex w-[340px] shrink-0 flex-col border-l bg-panel"
    >
      <div className="flex flex-col gap-1.5 border-b px-[18px] pt-[18px] pb-3.5">
        <div className="flex items-center gap-2">
          <span className="dot" data-state={tone} />
          <h2 className="m-0 flex-1 truncate text-[17px] font-semibold">
            {r.name}
          </h2>
          <button
            type="button"
            className="btn btn-sm btn-ghost px-1.5"
            aria-label="Hide side panel"
            title="Hide side panel (⌥⌘B)"
            onClick={onClose}
          >
            <CloseIcon />
          </button>
        </div>
        <span className="mono selectable text-[11.5px] break-all text-muted">
          {shortPath(r.display_path)}
        </span>
      </div>
      <dl className="m-0 grid grid-cols-[76px_minmax(0,1fr)] gap-x-2.5 gap-y-2 border-b px-[18px] py-3.5 text-[12.5px]">
        <dt className="text-muted">Branch</dt>
        <dd className="mono m-0 truncate text-[12px]">{headLabel(r)}</dd>
        <dt className="text-muted">Upstream</dt>
        <dd className="m-0">
          {r.upstream ? (
            <>
              <span className="mono text-[12px]">{r.upstream.ref}</span> ·{" "}
              <span className={up === "in sync" ? "text-clean" : "text-dirty"}>
                {up === "in sync"
                  ? "in sync"
                  : [
                      r.upstream.ahead ? `${r.upstream.ahead} ahead` : "",
                      r.upstream.behind ? `${r.upstream.behind} behind` : "",
                    ]
                      .filter(Boolean)
                      .join(", ")}
              </span>
              <br />
              <span className="text-[11.5px] text-muted">
                As of the last fetch, {relativeTime(r.last_fetch_at)}.{" "}
                <button
                  type="button"
                  className="text-link hover:underline disabled:opacity-50"
                  disabled={fetching}
                  onClick={onFetch}
                >
                  {fetching ? "Fetching…" : "Fetch now"}
                </button>
              </span>
              {r.fetch_error && (
                <span className="block text-[11.5px] text-conflict">
                  ! {r.fetch_error.message}
                </span>
              )}
            </>
          ) : (
            <span className="text-muted">none</span>
          )}
        </dd>
        <dt className="text-muted">Last commit</dt>
        <dd className="m-0 min-w-0">
          {last ? (
            <>
              <span className="line-clamp-2">{last.subject}</span>
              <span className="text-[11.5px] text-muted">
                <span className="mono">{last.short_id}</span> ·{" "}
                {relativeTime(last.committed_at)}
              </span>
            </>
          ) : (
            <span className="text-muted">
              {last === null || !available ? "—" : "…"}
            </span>
          )}
        </dd>
        <dt className="text-muted">Checked</dt>
        <dd className="m-0">
          {relativeTime(r.last_checked_at)}
          {tone === "stale" && <span className="text-dirty"> · stale</span>}
        </dd>
        {r.error && (
          <>
            <dt className="text-muted">Problem</dt>
            <dd className="selectable m-0 text-conflict">{r.error.message}</dd>
          </>
        )}
      </dl>
      <div className="flex items-center gap-2 px-[18px] pt-3 pb-1">
        <span className="section-label text-fg-2">Changes</span>
        <span className="text-[11px] text-muted">
          {changes
            ? plural(new Set(entries.map((e) => e.path)).size, "file")
            : ""}
        </span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-2">
        {changes && entries.length === 0 && (
          <div className="px-2.5 py-1 text-muted">Working tree clean.</div>
        )}
        {entries.slice(0, 50).map((e) => (
          <div
            key={`${e.group}:${e.path}`}
            className="flex h-[30px] shrink-0 items-center gap-2 rounded-md px-2.5"
          >
            <span className="kind" data-tone={kindTone(e.kind)}>
              {KIND_LETTER[e.kind]}
            </span>
            <span
              className="mono min-w-0 flex-1 truncate text-[12px]"
              title={e.path}
            >
              {e.path}
            </span>
            <span className="text-[11px] text-muted">{e.group}</span>
          </div>
        ))}
        {entries.length > 50 && (
          <div className="px-2.5 py-1 text-[12px] text-muted">
            and {entries.length - 50} more
          </div>
        )}
      </div>
      <div className="flex flex-col gap-2 border-t px-[18px] pt-3.5 pb-[18px]">
        {r.state === "missing" ? (
          <button
            type="button"
            className="btn btn-primary h-8"
            onClick={onLocate}
          >
            Locate…
          </button>
        ) : (
          <button
            type="button"
            className="btn btn-primary h-8"
            onClick={onOpen}
          >
            Open repository
            <span className="text-[11px] opacity-80">↵</span>
          </button>
        )}
        <div className="flex gap-2">
          <button
            type="button"
            className="btn flex-1"
            onClick={() => run(ipc.openInEditor(r.id))}
          >
            Open in editor
          </button>
          <button
            type="button"
            className="btn flex-1"
            onClick={() => run(ipc.revealInFinder(r.id))}
          >
            Reveal in Finder
          </button>
        </div>
      </div>
    </aside>
  );
}

function Welcome({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        data-tauri-drag-region
        className="h-12 shrink-0 border-b bg-header"
      />
      <div className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
        <div className="text-lg font-semibold">No repositories yet</div>
        <p className="m-0 max-w-md text-fg-2">
          Open a local Git repository, or create a workspace from a folder of
          repositories. Brainiac reads them: it never commits, checks out, or
          pulls, and it fetches only when you ask it to.
        </p>
        <button type="button" className="btn btn-primary" onClick={onAdd}>
          Add repository or workspace… <span className="opacity-80">⌘O</span>
        </button>
      </div>
    </div>
  );
}
