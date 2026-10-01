/** How a workspace's members are laid out in the sidebar and dashboard (SPEC.md, Workspaces and repositories). */
import type { RepositorySummary, Workspace, WorkspaceMember } from "./ipc";

export type WorkspaceLayout = {
  /** The member whose repository is the workspace root, shown first. */
  root: WorkspaceMember | null;
  /** Discovery folder label, such as `services/`; null for manual workspaces. */
  groupLabel: string | null;
  /** Discovered members, shown under `groupLabel`. */
  grouped: WorkspaceMember[];
  /** Manually added members; the whole list for a manual workspace. */
  others: WorkspaceMember[];
};

function baseName(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.at(-1) ?? path;
}

/** Label of the folder a discovered workspace scans, with a trailing slash. */
export function discoveryLabel(ws: Workspace): string | null {
  if (ws.discovery_mode !== "discovered") return null;
  if (ws.discovery_path) return `${ws.discovery_path.replace(/\/+$/, "")}/`;
  return ws.discovery_root ? `${baseName(ws.discovery_root)}/` : null;
}

export function workspaceLayout(ws: Workspace): WorkspaceLayout {
  const byName = (a: WorkspaceMember, b: WorkspaceMember) =>
    a.display_name.localeCompare(b.display_name);
  const isRoot = (m: WorkspaceMember) =>
    !!ws.root_repository_id && m.repository_id === ws.root_repository_id;
  const root = ws.members.find(isRoot) ?? null;
  const rest = ws.members.filter((m) => !isRoot(m));
  const label = discoveryLabel(ws);
  if (!label)
    return { root, groupLabel: null, grouped: [], others: rest.sort(byName) };
  return {
    root,
    groupLabel: label,
    grouped: rest.filter((m) => m.origin === "discovered").sort(byName),
    others: rest.filter((m) => m.origin !== "discovered").sort(byName),
  };
}

/** Members in display order: root, discovered group, then manual additions. */
export function orderedMembers(ws: Workspace): WorkspaceMember[] {
  const l = workspaceLayout(ws);
  return [...(l.root ? [l.root] : []), ...l.grouped, ...l.others];
}

/** Repositories of a workspace, deduplicated, in display order. */
export function workspaceRepositories(
  ws: Workspace,
  byId: Map<string, RepositorySummary>,
): RepositorySummary[] {
  const seen = new Set<string>();
  const out: RepositorySummary[] = [];
  for (const m of orderedMembers(ws)) {
    const r = m.repository_id ? byId.get(m.repository_id) : undefined;
    if (r && !seen.has(r.id)) {
      seen.add(r.id);
      out.push(r);
    }
  }
  return out;
}

export type Totals = {
  changedFiles: number;
  conflicts: number;
  dirtyRepos: number;
  conflictedRepos: number;
  problemRepos: number;
  ahead: number;
  behind: number;
  upToDate: number;
};

/** Aggregate counts; each repository's unique changed paths are summed, never nested repos twice. */
export function totals(repos: RepositorySummary[]): Totals {
  const t: Totals = {
    changedFiles: 0,
    conflicts: 0,
    dirtyRepos: 0,
    conflictedRepos: 0,
    problemRepos: 0,
    ahead: 0,
    behind: 0,
    upToDate: 0,
  };
  for (const r of repos) {
    const c = r.counts;
    t.changedFiles += c?.unique_paths ?? 0;
    t.conflicts += c?.conflicted ?? 0;
    if (c && c.unique_paths > 0) t.dirtyRepos++;
    if (c?.conflicted) t.conflictedRepos++;
    if (r.state === "stale" || r.state === "missing" || r.state === "error")
      t.problemRepos++;
    else if (r.state === "fresh") t.upToDate++;
    t.ahead += r.upstream?.ahead ?? 0;
    t.behind += r.upstream?.behind ?? 0;
  }
  return t;
}
