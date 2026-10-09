import { useCallback, useEffect, useRef, useState } from "react";
import { relativeTime, shortPath } from "../lib/format";
import {
  type AppSnapshot,
  type ChangesResult,
  errorMessage,
  ipc,
  type RefEntry,
  type RefsResult,
  type RepositorySummary,
  type RepositoryTab,
  type Task,
} from "../lib/ipc";
import { useKeys } from "../lib/keys";
import {
  headLabel,
  repoTone,
  TONE_LABEL,
  upstreamLabel,
  upstreamTarget,
} from "../lib/repo";
import { useSidePanel } from "../lib/sidePanel";
import { createLatest } from "../lib/stale";
import BranchesTab from "./BranchesTab";
import ChangesTab from "./ChangesTab";
import HistoryTab, { type HistoryScope } from "./HistoryTab";
import {
  BranchIcon,
  ChevronDown,
  ExternalIcon,
  FetchIcon,
  FolderIcon,
  MoreIcon,
  RefreshIcon,
} from "./icons";
import Popover from "./Popover";
import PullRequestsTab from "./PullRequestsTab";
import RepositoryNotesTab from "./RepositoryNotesTab";
import SidePanelButton from "./SidePanelButton";

/** Where the viewer opens, for links from the activity feed. */
export type RepoFocus = {
  tab: RepositoryTab;
  ref?: HistoryScope | null;
  commitId?: string | null;
};

type Props = {
  repository: RepositorySummary;
  /** Increments when the backend reports this repository changed. */
  changeTick: number;
  pinned: boolean;
  /** Initial tab, ref, and commit; otherwise the last tab used. */
  focus?: RepoFocus;
  /** A fetch of this repository is running. */
  fetching: boolean;
  onFetch: () => void;
  onTogglePin: () => void;
  onPalette: () => void;
  onRefresh: () => void;
  onRemove: () => void;
  /** Ask for the folder this repository moved to. */
  onLocate: () => void;
  onError: (message: string | null) => void;
  /** For the Notes tab. */
  snapshot: AppSnapshot;
  hasVault: boolean;
  onOpenNote: (noteId: string) => void;
  onNewNote: () => void;
  onEditTask: (task: Task) => void;
  /** A new query tab on a linked database connection (v0.4). */
  onNewQuery: (connectionId: string) => void;
  /** Pull requests (v0.3): open one, or Settings → Accounts. */
  onOpenPullRequest: (reference: string) => void;
  onOpenSettings: () => void;
  /** Reload the snapshot after the forge or a workspace switch changed. */
  onChanged: () => void;
};

export default function RepositoryView({
  repository,
  changeTick,
  pinned,
  focus,
  fetching,
  onFetch,
  onTogglePin,
  onPalette,
  onRefresh,
  onRemove,
  onLocate,
  onError,
  snapshot,
  hasVault,
  onOpenNote,
  onNewNote,
  onEditTask,
  onNewQuery,
  onOpenPullRequest,
  onOpenSettings,
  onChanged,
}: Props) {
  const [tab, setTab] = useState<RepositoryTab>(
    focus?.tab ?? repository.last_tab ?? "changes",
  );
  const [changes, setChanges] = useState<ChangesResult | null>(null);
  const [changesLoading, setChangesLoading] = useState(false);
  /** Branch or tag shown in History; null means HEAD. */
  const [historyRef, setHistoryRef] = useState<HistoryScope | null>(
    focus?.ref ?? null,
  );
  const [refs, setRefs] = useState<RefsResult | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const { open: panelOpen, toggle: togglePanel } = useSidePanel();

  // One stale-guard per data source, so a slow response cannot clobber a newer one.
  const changesLatest = useRef(createLatest()).current;
  const refsLatest = useRef(createLatest()).current;
  const unavailable =
    repository.state === "missing" ||
    (repository.state === "error" && !repository.counts);

  const loadChanges = useCallback(() => {
    setChangesLoading(true);
    void changesLatest.run(
      () => ipc.listChanges(repository.id),
      (result) => {
        setChangesLoading(false);
        if (result.repository_id === repository.id) setChanges(result);
      },
      (e) => {
        setChangesLoading(false);
        onError(errorMessage(e));
      },
    );
  }, [repository.id, changesLatest, onError]);

  const loadRefs = useCallback(() => {
    void refsLatest.run(
      () => ipc.listRefs(repository.id),
      (result) => {
        if (result.repository_id === repository.id) setRefs(result);
      },
      (e) => onError(errorMessage(e)),
    );
  }, [repository.id, refsLatest, onError]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: changeTick deliberately triggers a reload after backend events.
  useEffect(() => {
    if (!unavailable) loadChanges();
  }, [loadChanges, changeTick, unavailable]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: changeTick refreshes refs after backend events.
  useEffect(() => {
    if (tab === "refs" && !unavailable) loadRefs();
  }, [tab, loadRefs, changeTick, unavailable]);

  const changeTab = (next: RepositoryTab) => {
    setTab(next);
    void ipc.setRepositoryTab(repository.id, next).catch(() => {});
  };

  const showRefHistory = (ref: RefEntry | null) => {
    setHistoryRef(ref);
    changeTab("history");
  };

  useKeys({
    "mod+1": () => changeTab("changes"),
    "mod+2": () => changeTab("history"),
    "mod+3": () => changeTab("refs"),
    "mod+4": () => changeTab("notes"),
    "mod+5": () => changeTab("pull_requests"),
  });

  const run = (p: Promise<unknown>) =>
    void p.catch((e) => onError(errorMessage(e)));
  const openInEditor = (path?: string, line?: number) =>
    run(ipc.openInEditor(repository.id, path, line));

  const changeCount =
    changes?.counts.unique_paths ?? repository.counts?.unique_paths;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center gap-3 border-b bg-header pr-3 pl-4 pl-lead"
      >
        <button
          type="button"
          className="btn bg-control pr-2 pl-2.5 text-fg"
          aria-label="Switch repository"
          title="Switch repository or workspace (⌘K)"
          onClick={onPalette}
        >
          <span className="font-semibold">{repository.name}</span>
          <ChevronDown size={11} className="text-muted" />
          <span className="kbd">⌘K</span>
        </button>
        <div role="tablist" aria-label="Repository views" className="seg">
          <button
            type="button"
            role="tab"
            aria-selected={tab === "changes"}
            onClick={() => changeTab("changes")}
          >
            Changes
            {!!changeCount && (
              <span className="rounded-lg bg-dirty px-1.5 text-[11px] font-semibold text-app">
                {changeCount}
              </span>
            )}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "history"}
            onClick={() => changeTab("history")}
          >
            History
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "refs"}
            onClick={() => changeTab("refs")}
          >
            Branches &amp; tags
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "notes"}
            onClick={() => changeTab("notes")}
          >
            Notes
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "pull_requests"}
            title="Pull requests (⌘5)"
            onClick={() => changeTab("pull_requests")}
          >
            Pull requests
          </button>
        </div>
        <div data-tauri-drag-region className="h-full flex-1" />
        <BranchPill repository={repository} />
        <button
          type="button"
          className="btn"
          disabled={fetching || unavailable}
          title={fetchTitle(repository)}
          onClick={onFetch}
        >
          <FetchIcon size={13} className={fetching ? "animate-pulse" : ""} />
          {fetching ? "Fetching…" : "Fetch"}
          {repository.fetch_error && !fetching && (
            <span className="text-conflict" title="Last fetch failed">
              !
            </span>
          )}
        </button>
        <button
          type="button"
          className="btn icon-btn"
          aria-label="Refresh"
          title="Refresh (⌘R)"
          onClick={onRefresh}
        >
          <RefreshIcon />
        </button>
        <button
          type="button"
          className="btn icon-btn"
          aria-label="Reveal in Finder"
          title="Reveal in Finder"
          onClick={() => run(ipc.revealInFinder(repository.id))}
        >
          <FolderIcon />
        </button>
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => openInEditor()}
        >
          <ExternalIcon size={13} />
          Open in editor
        </button>
        {(tab === "pull_requests" || tab === "refs") && (
          <SidePanelButton open={panelOpen} onToggle={togglePanel} />
        )}
        <div className="relative">
          <button
            type="button"
            className="btn btn-ghost icon-btn"
            aria-label="More actions"
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
                  onTogglePin();
                }}
              >
                {pinned ? "Unpin" : "Pin to sidebar"}
              </MenuButton>
              <MenuButton
                onClick={() => {
                  setMenuOpen(false);
                  void navigator.clipboard?.writeText(repository.display_path);
                }}
              >
                Copy path
              </MenuButton>
              <MenuButton
                onClick={() => {
                  setMenuOpen(false);
                  onLocate();
                }}
              >
                Locate Folder…
              </MenuButton>
              <div className="menu-sep" />
              <MenuButton
                onClick={() => {
                  setMenuOpen(false);
                  onRemove();
                }}
              >
                Remove from Brainiac…
              </MenuButton>
            </Popover>
          )}
        </div>
      </header>

      {tab === "pull_requests" ? (
        <PullRequestsTab
          scope={{ kind: "repository", repository }}
          snapshot={snapshot}
          onOpen={onOpenPullRequest}
          onOpenSettings={onOpenSettings}
          onChanged={onChanged}
          onError={onError}
        />
      ) : tab === "notes" ? (
        <RepositoryNotesTab
          repository={repository}
          snapshot={snapshot}
          hasVault={hasVault}
          onOpenNote={onOpenNote}
          onNewNote={onNewNote}
          onEditTask={onEditTask}
          onError={onError}
          onNewQuery={onNewQuery}
        />
      ) : unavailable ? (
        <Unavailable
          repository={repository}
          onRemove={onRemove}
          onRefresh={onRefresh}
          onLocate={onLocate}
        />
      ) : tab === "changes" ? (
        <ChangesTab
          repositoryId={repository.id}
          changes={changes}
          refreshing={changesLoading}
          onError={onError}
          onOpenInEditor={openInEditor}
        />
      ) : tab === "history" ? (
        <HistoryTab
          repositoryId={repository.id}
          headName={headLabel(repository)}
          historyRef={historyRef}
          onHistoryRef={setHistoryRef}
          initialCommitId={focus?.commitId ?? null}
          refs={refs}
          onNeedRefs={loadRefs}
          changeTick={changeTick}
          onError={onError}
          onOpenInEditor={openInEditor}
        />
      ) : (
        <BranchesTab
          repositoryId={repository.id}
          refs={refs}
          onShowHistory={showRefHistory}
          onError={onError}
          onOpenInEditor={openInEditor}
        />
      )}
    </div>
  );
}

export function MenuButton({
  onClick,
  children,
}: {
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      className="menu-item"
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/** Current branch, its upstream, and the local ahead/behind comparison. */
function BranchPill({ repository: r }: { repository: RepositorySummary }) {
  const up = upstreamLabel(r);
  const branch = headLabel(r);
  const target = r.upstream ? upstreamTarget(r.upstream.ref, branch) : null;
  return (
    <div
      className="mono flex h-[30px] max-w-[min(560px,38vw)] min-w-0 items-center gap-2 rounded-[7px] border border-control-line px-2.5 text-[12px]"
      title={
        r.upstream
          ? `${branch} → ${r.upstream.ref}${up ? ` · ${up}` : ""}\nAs of the last fetch, ${relativeTime(r.last_fetch_at)}.`
          : `${branch}\nNo upstream configured`
      }
    >
      <BranchIcon size={13} className="shrink-0 text-fg-2" />
      <span className="min-w-0 truncate">{branch}</span>
      {target && (
        <span className="hidden max-w-[180px] shrink-0 truncate text-muted xl:inline">
          → {target}
        </span>
      )}
      {up && (
        <span
          className={`shrink-0 whitespace-nowrap ${up === "in sync" ? "text-clean" : "text-link"}`}
        >
          {up}
        </span>
      )}
    </div>
  );
}

function Unavailable({
  repository: r,
  onRemove,
  onRefresh,
  onLocate,
}: {
  repository: RepositorySummary;
  onRemove: () => void;
  onRefresh: () => void;
  onLocate: () => void;
}) {
  return (
    <div className="flex flex-1 flex-col items-start gap-3 p-8">
      <div className="flex items-center gap-2 text-lg font-semibold">
        <span className="dot" data-state={repoTone(r)} />
        {TONE_LABEL[repoTone(r)]}
      </div>
      <p className="selectable m-0 max-w-xl text-fg-2">
        {r.state === "missing" ? (
          <>
            Nothing found at{" "}
            <span className="mono">{shortPath(r.display_path)}</span>. The
            registration is kept: locate the folder where it moved to, move it
            back, or remove the registration. Your files are never touched.
          </>
        ) : (
          (r.error?.message ?? "This repository could not be read.")
        )}
      </p>
      {r.error?.details && (
        <pre className="selectable m-0 max-w-full overflow-auto rounded-md bg-panel p-3 text-[11.5px] whitespace-pre-wrap text-muted">
          {r.error.details}
        </pre>
      )}
      <div className="flex gap-2">
        {r.state === "missing" && (
          <button type="button" className="btn btn-primary" onClick={onLocate}>
            Locate…
          </button>
        )}
        <button type="button" className="btn" onClick={onRefresh}>
          Check again
        </button>
        <button type="button" className="btn" onClick={onRemove}>
          Remove from Brainiac…
        </button>
      </div>
    </div>
  );
}

function fetchTitle(r: RepositorySummary): string {
  const when = r.last_fetch_at
    ? `Last fetched ${relativeTime(r.last_fetch_at)}.`
    : "Not fetched yet.";
  const failed = r.fetch_error
    ? `\nLast fetch failed: ${r.fetch_error.message}`
    : "";
  return `Fetch the remote: updates remote-tracking branches and tags only, never your files or branches. ${when}${failed}`;
}
