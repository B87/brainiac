import { useCallback, useEffect, useId, useRef, useState } from "react";
import { relativeTime } from "../lib/format";
import {
  type AppSnapshot,
  errorMessage,
  type ForgeKind,
  ipc,
  onPullRequestChanged,
  type PullRequest,
  type PullRequestGroup,
  type PullRequestList,
  type RepositorySummary,
  type Reviewer,
  subscribe,
  type Workspace,
} from "../lib/ipc";
import { usePref } from "../lib/prefs";
import {
  ageLabel,
  budgetLabel,
  byLongestWait,
  CHECK_LABEL,
  checkedOutPullRequest,
  checksLabel,
  matchesFilter,
  oldestFetch,
  PROVIDER_LABEL,
  type PullRequestFilter,
  REPOSITORY_FILTERS,
  REVIEW_LABEL,
  repositoryName,
  STATE_LABEL,
  sizeLabel,
  WORKSPACE_FILTERS,
} from "../lib/pullRequests";
import { plural } from "../lib/repo";
import { useSidePanel } from "../lib/sidePanel";
import { createLatest } from "../lib/stale";
import { Avatar } from "./HistoryTab";
import {
  CheckIcon,
  CircleIcon,
  CommentIcon,
  CrossIcon,
  RefreshIcon,
} from "./icons";

/** A workspace's or one repository's pull requests. */
export type Scope =
  | { kind: "workspace"; workspace: Workspace }
  | { kind: "repository"; repository: RepositorySummary };

type Props = {
  scope: Scope;
  snapshot: AppSnapshot;
  onOpen: (reference: string) => void;
  onOpenSettings: () => void;
  /** The switch or the forge changed; the snapshot needs a reload. */
  onChanged: () => void;
  onError: (message: string | null) => void;
};

/** Lists are read again after this long (SPEC.md, Staying up to date). */
const LIST_MAX_AGE = 300;
/** Changes within this window cause a single reload. */
const RELOAD_DELAY_MS = 400;

export default function PullRequestsTab({
  scope,
  snapshot,
  onOpen,
  onOpenSettings,
  onChanged,
  onError,
}: Props) {
  const { open: panelOpen } = useSidePanel();
  const scopeId =
    scope.kind === "workspace" ? scope.workspace.id : scope.repository.id;
  const [filter, setFilter] = usePref<PullRequestFilter>(
    `brainiac.prFilter.${scope.kind}`,
    "all",
  );
  const [list, setList] = useState<PullRequestList | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const latest = useRef(createLatest()).current;
  const closed = filter === "closed" && scope.kind === "repository";

  const load = useCallback(
    (maxAge: number) => {
      setRefreshing(true);
      void latest.run(
        () =>
          ipc.listPullRequests({
            workspace_id: scope.kind === "workspace" ? scopeId : null,
            repository_id: scope.kind === "repository" ? scopeId : null,
            closed,
            max_age_seconds: maxAge,
          }),
        (result) => {
          setRefreshing(false);
          setList(result);
        },
        (e) => {
          setRefreshing(false);
          onError(errorMessage(e));
        },
      );
    },
    [scope.kind, scopeId, closed, latest, onError],
  );

  useEffect(() => {
    setList(null);
    load(LIST_MAX_AGE);
  }, [load]);

  // A pull request read anew somewhere (the background sync, another view)
  // is in the cache: show it without asking the provider again.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    return subscribe(
      onPullRequestChanged(() => {
        if (timer) clearTimeout(timer);
        timer = setTimeout(() => load(LIST_MAX_AGE), RELOAD_DELAY_MS);
      }),
    );
  }, [load]);

  const guard = (p: Promise<unknown>) =>
    p
      .then(() => {
        onChanged();
        load(LIST_MAX_AGE);
      })
      .catch((e) => onError(errorMessage(e)));

  const filters =
    scope.kind === "workspace" ? WORKSPACE_FILTERS : REPOSITORY_FILTERS;
  const groups = list?.groups ?? [];
  const shown = groups.map((g) => ({
    ...g,
    pull_requests: g.pull_requests
      .filter((p) => matchesFilter(p, filter))
      .sort(byLongestWait),
  }));
  const total = shown.reduce((n, g) => n + g.pull_requests.length, 0);
  const asOf = oldestFetch(groups);

  const repository = scope.kind === "repository" ? scope.repository : null;
  const checkedOut =
    repository && groups[0]
      ? checkedOutPullRequest(repository, groups[0].pull_requests)
      : null;

  return (
    <div className="flex min-h-0 flex-1">
      <section
        aria-label="Pull requests"
        className="flex min-w-0 flex-1 flex-col gap-3.5 overflow-y-auto px-6 pt-5 pb-6"
      >
        <div className="flex flex-wrap items-center gap-2">
          <fieldset aria-label="Filter" className="seg seg-sm">
            {filters.map((f) => (
              <button
                key={f.value}
                type="button"
                aria-pressed={filter === f.value}
                onClick={() => setFilter(f.value)}
              >
                {f.label}
              </button>
            ))}
          </fieldset>
          <span className="text-[11.5px] text-muted">
            {list?.enabled ? plural(total, "pull request") : ""}
          </span>
          <span className="flex-1" />
          {list?.enabled && (
            <>
              {asOf && (
                <span
                  className="text-[12px] text-muted"
                  title={new Date(asOf).toLocaleString()}
                >
                  Updated {relativeTime(asOf)}
                </span>
              )}
              <button
                type="button"
                className="btn btn-sm"
                disabled={refreshing}
                onClick={() => load(0)}
              >
                <RefreshIcon
                  size={12}
                  className={refreshing ? "animate-pulse" : ""}
                />
                {refreshing ? "Checking…" : "Refresh"}
              </button>
            </>
          )}
        </div>

        {!list && <TableSkeleton />}

        {list && !list.enabled && scope.kind === "workspace" && (
          <TurnOn
            workspace={scope.workspace}
            onTurnOn={() =>
              guard(ipc.updateWorkspacePullRequests(scope.workspace.id, true))
            }
          />
        )}
        {list && !list.enabled && repository && (
          <Untracked
            repository={repository}
            snapshot={snapshot}
            onTurnOn={(id) => guard(ipc.updateWorkspacePullRequests(id, true))}
          />
        )}
        {list?.enabled && repository && !repository.forge && (
          <div className="rounded-lg border border-dashed px-4 py-6 text-center text-fg-2">
            This repository's <span className="mono">origin</span> is on neither
            GitHub nor Bitbucket Cloud.
            <div className="mt-1 text-[12px] text-muted">
              Choose the repository its pull requests live in, in the side
              panel.
            </div>
          </div>
        )}

        {list && list.missing_accounts.length > 0 && (
          <div className="flex items-center gap-2.5 rounded-lg border border-info-line bg-info-bg px-3 py-2 text-[12.5px] text-info-fg">
            <span className="flex-1">
              No{" "}
              {list.missing_accounts.map((k) => PROVIDER_LABEL[k]).join(" or ")}{" "}
              account yet. Pull requests on{" "}
              {list.missing_accounts.length === 1 ? "it" : "them"} are not
              shown.
            </span>
            <button
              type="button"
              className="btn btn-sm"
              onClick={onOpenSettings}
            >
              Add Account…
            </button>
          </div>
        )}

        {list?.enabled &&
          shown.map((group) => (
            <Group
              key={group.repository_id}
              group={group}
              grouped={scope.kind === "workspace"}
              closed={closed}
              checkedOut={checkedOut}
              ahead={repository?.upstream?.ahead ?? 0}
              onOpen={onOpen}
            />
          ))}
        {list?.enabled && groups.length > 0 && total === 0 && (
          <div className="rounded-lg border border-dashed px-4 py-6 text-center text-fg-2">
            {filter === "all"
              ? "No open pull requests."
              : "No pull requests match this filter."}
          </div>
        )}
      </section>

      {panelOpen && (
        <aside
          aria-label="Pull request settings"
          className="flex w-[300px] shrink-0 flex-col overflow-y-auto border-l bg-panel"
        >
          {scope.kind === "workspace" ? (
            <WorkspacePanel
              workspace={scope.workspace}
              list={list}
              onToggle={(on) =>
                guard(ipc.updateWorkspacePullRequests(scope.workspace.id, on))
              }
              onOpenSettings={onOpenSettings}
            />
          ) : (
            <RepositoryPanel
              repository={scope.repository}
              list={list}
              onChanged={() => {
                onChanged();
                load(LIST_MAX_AGE);
              }}
              onError={onError}
              onOpenSettings={onOpenSettings}
            />
          )}
        </aside>
      )}
    </div>
  );
}

function TableSkeleton() {
  return (
    <output
      className="flex flex-col gap-2.5"
      aria-label="Loading pull requests"
    >
      {[0, 1, 2, 3].map((i) => (
        <div key={i} className="flex items-center gap-4 px-2 py-2">
          <div className="skeleton w-2/5" />
          <div className="skeleton h-5 w-5 rounded-full" />
          <div className="skeleton w-1/6" />
          <div className="skeleton w-1/12" />
          <div className="skeleton w-1/12" />
        </div>
      ))}
    </output>
  );
}

function TurnOn({
  workspace,
  onTurnOn,
}: {
  workspace: Workspace;
  onTurnOn: () => void;
}) {
  return (
    <div className="flex flex-col items-start gap-2 rounded-lg border bg-panel px-4 py-4">
      <span className="font-medium">
        Pull requests are off for {workspace.name}.
      </span>
      <span className="text-[12.5px] text-fg-2">
        Turning them on lists the open pull requests of its repositories on
        GitHub and Bitbucket Cloud, with each one's reviewers, checks, and
        threads, and keeps them up to date every five minutes while Brainiac is
        open. Nothing is written anywhere until you comment, review, or merge.
      </span>
      <button type="button" className="btn btn-primary" onClick={onTurnOn}>
        Turn On Pull Requests
      </button>
    </div>
  );
}

function Untracked({
  repository,
  snapshot,
  onTurnOn,
}: {
  repository: RepositorySummary;
  snapshot: AppSnapshot;
  onTurnOn: (workspaceId: string) => void;
}) {
  const containing = snapshot.workspaces.filter((w) =>
    w.members.some((m) => m.repository_id === repository.id),
  );
  return (
    <div className="flex flex-col items-start gap-2 rounded-lg border bg-panel px-4 py-4">
      <span className="font-medium">
        No workspace tracks this repository's pull requests.
      </span>
      {containing.length === 0 ? (
        <span className="text-[12.5px] text-fg-2">
          Add the repository to a workspace, then turn pull requests on there.
        </span>
      ) : (
        <div className="flex flex-wrap gap-2">
          {containing.map((w) => (
            <button
              key={w.id}
              type="button"
              className="btn"
              onClick={() => onTurnOn(w.id)}
            >
              Turn On for {w.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function Group({
  group,
  grouped,
  closed,
  checkedOut,
  ahead,
  onOpen,
}: {
  group: PullRequestGroup;
  grouped: boolean;
  closed: boolean;
  checkedOut: PullRequest | null;
  ahead: number;
  onOpen: (reference: string) => void;
}) {
  const rows = checkedOut
    ? [
        checkedOut,
        ...group.pull_requests.filter(
          (p) => p.reference !== checkedOut.reference,
        ),
      ]
    : group.pull_requests;
  return (
    <div className="flex flex-col gap-1.5">
      {grouped && (
        <div className="flex items-center gap-2 pt-1">
          <span className="font-semibold">{group.repository_name}</span>
          <span className="mono text-[11.5px] text-muted">
            {repositoryName(group.forge)}
          </span>
          <span className="text-[11.5px] text-muted">
            · {PROVIDER_LABEL[group.kind]}
          </span>
          {group.fetched_at && (
            <span className="text-[11.5px] text-muted">
              · {relativeTime(group.fetched_at)}
            </span>
          )}
        </div>
      )}
      {group.error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {group.error.message}
          {group.pull_requests.length > 0 && group.fetched_at && (
            <span className="text-muted">
              {" "}
              Showing what was read {relativeTime(group.fetched_at)}.
            </span>
          )}
        </div>
      )}
      {rows.length > 0 && (
        <table className="pr-table">
          <thead>
            <tr>
              <th>Pull request</th>
              <th>Author</th>
              <th>Reviewers</th>
              <th>Checks</th>
              <th title="Unresolved threads">Threads</th>
              <th>Size</th>
              <th>{closed ? "Closed" : "Age"}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((pr) => (
              <Row
                key={pr.reference}
                pr={pr}
                closed={closed}
                checkedOut={checkedOut?.reference === pr.reference}
                ahead={ahead}
                onOpen={() => onOpen(pr.reference)}
              />
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function Row({
  pr,
  closed,
  checkedOut,
  ahead,
  onOpen,
}: {
  pr: PullRequest;
  closed: boolean;
  checkedOut: boolean;
  ahead: number;
  onOpen: () => void;
}) {
  const unresolved = pr.counts.unresolved_threads;
  return (
    <tr
      data-waiting={pr.awaiting_my_review}
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter") onOpen();
      }}
      aria-label={`${pr.title} #${pr.number}`}
    >
      <td>
        <div className="flex flex-col gap-px">
          <span className="flex items-center gap-2">
            <span className="truncate font-medium">{pr.title}</span>
            <span className="mono text-[11.5px] text-muted">#{pr.number}</span>
            {(pr.state !== "open" || closed) && (
              <span className="pr-state" data-state={pr.state}>
                {STATE_LABEL[pr.state]}
              </span>
            )}
            {pr.awaiting_my_review && (
              <span className="badge">Needs your review</span>
            )}
          </span>
          {checkedOut && (
            <span className="text-[11.5px] text-muted">
              Checked out
              {ahead > 0 && ` · ${plural(ahead, "commit")} not pushed yet`}
            </span>
          )}
        </div>
      </td>
      <td>
        <span className="flex items-center gap-1.5 whitespace-nowrap">
          <Avatar name={pr.author.display_name ?? pr.author.login} size={18} />
          <span className={pr.mine ? "font-medium" : ""}>
            {pr.author.login}
          </span>
        </span>
      </td>
      <td>
        <Reviewers reviewers={pr.reviewers} />
      </td>
      <td>
        <span className="flex items-center gap-1.5 whitespace-nowrap">
          {pr.checks.state && (
            <StateIcon
              state={pr.checks.state}
              label={CHECK_LABEL[pr.checks.state]}
            />
          )}
          <span className={pr.checks.state ? "" : "text-muted"}>
            {checksLabel(pr.checks)}
          </span>
        </span>
      </td>
      <td className="tabular">
        {unresolved === null ? (
          <span className="text-muted">—</span>
        ) : unresolved === 0 ? (
          <span className="text-muted">0</span>
        ) : (
          <span className="flex items-center gap-1">
            <CommentIcon size={12} className="text-muted" />
            {unresolved}
          </span>
        )}
      </td>
      <td className="tabular whitespace-nowrap text-fg-2">{sizeLabel(pr)}</td>
      <td
        className="tabular whitespace-nowrap text-fg-2"
        title={new Date(
          closed && pr.closed_at ? pr.closed_at : pr.created_at,
        ).toLocaleString()}
      >
        {closed && pr.closed_at ? relativeTime(pr.closed_at) : ageLabel(pr)}
      </td>
    </tr>
  );
}

function Reviewers({ reviewers }: { reviewers: Reviewer[] }) {
  if (reviewers.length === 0) return <span className="text-muted">—</span>;
  return (
    <span className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
      {reviewers.map((r) => (
        <span
          key={r.user.id}
          className="flex items-center gap-1 whitespace-nowrap"
          title={`${r.user.display_name ?? r.user.login}: ${REVIEW_LABEL[r.state]}`}
        >
          <StateIcon state={r.state} label={REVIEW_LABEL[r.state]} />
          <span className={r.is_me ? "font-medium" : ""}>{r.user.login}</span>
        </span>
      ))}
    </span>
  );
}

/** A state with an icon as well as a color. */
export function StateIcon({ state, label }: { state: string; label: string }) {
  const icon =
    state === "approved" || state === "success" ? (
      <CheckIcon size={12} />
    ) : state === "changes_requested" || state === "failure" ? (
      <CrossIcon size={12} />
    ) : state === "commented" ? (
      <CommentIcon size={12} />
    ) : (
      <CircleIcon size={12} />
    );
  return (
    <span
      className="state-icon"
      data-state={state}
      role="img"
      aria-label={label}
    >
      {icon}
    </span>
  );
}

function Budgets({ list }: { list: PullRequestList | null }) {
  if (!list || list.budgets.length === 0) return null;
  return (
    <div className="flex flex-col gap-1.5 border-b px-[18px] py-3.5">
      <span className="section-label text-fg-2">Requests</span>
      {list.budgets.map((b) => (
        <span key={b.kind} className="text-[12px] text-fg-2">
          {PROVIDER_LABEL[b.kind]}: {budgetLabel(b)}
          {b.retry_at && (
            <span className="block text-conflict">
              Paused until{" "}
              {new Date(b.retry_at).toLocaleTimeString([], {
                hour: "2-digit",
                minute: "2-digit",
              })}
              .
            </span>
          )}
        </span>
      ))}
    </div>
  );
}

function AccountsPanel({
  kinds,
  list,
  onOpenSettings,
}: {
  kinds: ForgeKind[];
  list: PullRequestList | null;
  onOpenSettings: () => void;
}) {
  if (kinds.length === 0) return null;
  return (
    <div className="flex flex-col gap-1.5 border-b px-[18px] py-3.5">
      <span className="section-label text-fg-2">Accounts</span>
      {kinds.map((k) => {
        const missing = list?.missing_accounts.includes(k);
        return (
          <span key={k} className="flex items-center gap-2 text-[12px]">
            <span className="flex-1">
              {PROVIDER_LABEL[k]}:{" "}
              {missing ? (
                <span className="text-conflict">no account</span>
              ) : (
                <span className="text-fg-2">connected</span>
              )}
            </span>
            <button
              type="button"
              className="btn btn-sm btn-ghost text-link"
              onClick={onOpenSettings}
            >
              {missing ? "Add…" : "Settings"}
            </button>
          </span>
        );
      })}
    </div>
  );
}

function WorkspacePanel({
  workspace,
  list,
  onToggle,
  onOpenSettings,
}: {
  workspace: Workspace;
  list: PullRequestList | null;
  onToggle: (on: boolean) => void;
  onOpenSettings: () => void;
}) {
  const kinds = Array.from(new Set((list?.groups ?? []).map((g) => g.kind)));
  return (
    <>
      <div className="flex flex-col gap-2.5 border-b px-[18px] py-3.5">
        <span className="section-label text-fg-2">{workspace.name}</span>
        <label className="flex items-start gap-2 text-[12.5px]">
          <input
            type="checkbox"
            className="mt-0.5"
            checked={workspace.pull_requests}
            onChange={(e) => onToggle(e.target.checked)}
          />
          <span className="flex flex-col gap-px">
            <span>Track pull requests</span>
            <span className="text-[11.5px] text-muted">
              Reads the open pull requests of this workspace's repositories
              every five minutes while Brainiac is open. Off by default.
            </span>
          </span>
        </label>
      </div>
      <AccountsPanel
        kinds={kinds}
        list={list}
        onOpenSettings={onOpenSettings}
      />
      <Budgets list={list} />
    </>
  );
}

function RepositoryPanel({
  repository,
  list,
  onChanged,
  onError,
  onOpenSettings,
}: {
  repository: RepositorySummary;
  list: PullRequestList | null;
  onChanged: () => void;
  onError: (message: string | null) => void;
  onOpenSettings: () => void;
}) {
  const [choosing, setChoosing] = useState(false);
  const forge = repository.forge;
  const kinds = forge ? [forge.kind] : [];
  const set = (
    target: { kind: ForgeKind; owner: string; name: string } | null,
  ) =>
    ipc
      .setRepositoryForge({ repository_id: repository.id, forge: target })
      .then(() => {
        setChoosing(false);
        onChanged();
      })
      .catch((e) => onError(errorMessage(e)));
  return (
    <>
      <div className="flex flex-col gap-2 border-b px-[18px] py-3.5">
        <span className="section-label text-fg-2">Pull requests come from</span>
        {forge ? (
          <span className="text-[12.5px]">
            <span className="mono">{forge.reference}</span>
            <span className="block text-[11.5px] text-muted">
              {forge.source === "origin"
                ? "Where origin points."
                : "Chosen by hand; kept when origin changes."}
            </span>
          </span>
        ) : (
          <span className="text-[12.5px] text-muted">
            Nothing yet: <span className="mono">origin</span> is on neither
            provider.
          </span>
        )}
        {!choosing && (
          <div className="flex gap-2">
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => setChoosing(true)}
            >
              {forge ? "Change…" : "Choose Repository…"}
            </button>
            {forge?.source === "override" && (
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => void set(null)}
              >
                Use origin
              </button>
            )}
          </div>
        )}
        {choosing && (
          <ForgeChooser
            initial={
              forge
                ? { kind: forge.kind, owner: forge.owner, name: forge.name }
                : null
            }
            onCancel={() => setChoosing(false)}
            onChoose={(t) => void set(t)}
          />
        )}
      </div>
      <div className="flex flex-col gap-1.5 border-b px-[18px] py-3.5">
        <span className="section-label text-fg-2">Tracked by</span>
        <span className="text-[12.5px] text-fg-2">
          {list && list.tracked_by.length > 0
            ? list.tracked_by.join(", ")
            : "No workspace yet."}
        </span>
      </div>
      <AccountsPanel
        kinds={kinds}
        list={list}
        onOpenSettings={onOpenSettings}
      />
      <Budgets list={list} />
    </>
  );
}

function ForgeChooser({
  initial,
  onCancel,
  onChoose,
}: {
  initial: { kind: ForgeKind; owner: string; name: string } | null;
  onCancel: () => void;
  onChoose: (target: { kind: ForgeKind; owner: string; name: string }) => void;
}) {
  const id = useId();
  const [kind, setKind] = useState<ForgeKind>(initial?.kind ?? "github");
  const [owner, setOwner] = useState(initial?.owner ?? "");
  const [name, setName] = useState(initial?.name ?? "");
  const ok = owner.trim() !== "" && name.trim() !== "";
  return (
    <form
      className="flex flex-col gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        if (ok) onChoose({ kind, owner: owner.trim(), name: name.trim() });
      }}
    >
      <fieldset aria-label="Provider" className="seg seg-sm self-start">
        {(["github", "bitbucket_cloud"] as ForgeKind[]).map((k) => (
          <button
            key={k}
            type="button"
            aria-pressed={kind === k}
            onClick={() => setKind(k)}
          >
            {PROVIDER_LABEL[k]}
          </button>
        ))}
      </fieldset>
      <label
        className="flex flex-col gap-1 text-[12px]"
        htmlFor={`${id}-owner`}
      >
        {kind === "github" ? "Owner (user or organization)" : "Workspace"}
        <input
          id={`${id}-owner`}
          className="text-input mono"
          value={owner}
          onChange={(e) => setOwner(e.target.value)}
        />
      </label>
      <label className="flex flex-col gap-1 text-[12px]" htmlFor={`${id}-name`}>
        Repository
        <input
          id={`${id}-name`}
          className="text-input mono"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </label>
      <div className="flex gap-2">
        <button type="submit" className="btn btn-sm btn-primary" disabled={!ok}>
          Use This Repository
        </button>
        <button type="button" className="btn btn-sm" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
