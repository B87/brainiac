import { useCallback, useEffect, useRef, useState } from "react";
import { shortPath } from "../lib/format";
import {
  type AppSnapshot,
  type ChangesResult,
  errorMessage,
  ipc,
  type RefsResult,
  type RepositorySummary,
  type RepositoryTab,
  type Task,
} from "../lib/ipc";
import { useKeys } from "../lib/keys";
import { headLabel, plural, repoTone, TONE_LABEL } from "../lib/repo";
import { EDITOR_PRESETS, editorPreset } from "../lib/settings";
import { useSidePanel } from "../lib/sidePanel";
import { createLatest } from "../lib/stale";
import BranchControl from "./BranchControl";
import BranchesTab from "./BranchesTab";
import ChangesTab from "./ChangesTab";
import HistoryTab, { type HistoryScope } from "./HistoryTab";
import { ChevronDown, ExternalIcon, MoreIcon } from "./icons";
import Popover from "./Popover";
import PullRequestsTab from "./PullRequestsTab";
import RepositoryNotesTab from "./RepositoryNotesTab";
import SidePanelButton from "./SidePanelButton";
import { requestTerminal } from "./terminal/actions";

/** The tabs: id, label, and the full name when the label shortens it. */
const TABS: Array<[RepositoryTab, string, string?]> = [
  ["changes", "Changes"],
  ["history", "History"],
  ["refs", "Branches", "Branches and tags"],
  ["notes", "Notes"],
  ["pull_requests", "PRs", "Pull requests"],
];

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
  /** Open pull requests waiting on your review, for the PRs tab. */
  awaitingReview: number;
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
  awaitingReview,
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
  const [tabMenuOpen, setTabMenuOpen] = useState(false);
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

  const showRefHistory = (ref: HistoryScope | null) => {
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

  const preset = editorPreset(snapshot.settings.editor);
  const editorName = EDITOR_PRESETS.find((p) => p.id === preset)?.label ?? null;

  const changeCount =
    changes?.counts.unique_paths ?? repository.counts?.unique_paths;

  /** A tab's count: uncommitted changes, or pull requests waiting on your review. */
  const tabCount = (id: RepositoryTab) =>
    id === "changes" && changeCount ? (
      <span
        className="rounded-lg bg-dirty px-1.5 text-[11px] font-semibold text-app"
        title={`${changeCount} changed`}
      >
        {changeCount}
      </span>
    ) : id === "pull_requests" && awaitingReview > 0 ? (
      <span
        className="badge"
        title={`${plural(awaitingReview, "pull request")} waiting on your review`}
      >
        {awaitingReview}
      </span>
    ) : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header
        data-tauri-drag-region
        className="@container flex h-12 shrink-0 items-center gap-2 border-b bg-header pr-3 pl-3 pl-lead"
      >
        <nav
          aria-label="Location"
          className="flex min-w-0 shrink items-center gap-0.5"
        >
          <button
            type="button"
            className="btn btn-ghost shrink-0 px-2 text-fg"
            aria-label={`Switch repository: ${repository.name}`}
            title="Switch repository or workspace (⌘K)"
            onClick={onPalette}
          >
            <span className="font-semibold">{repository.name}</span>
            <ChevronDown size={11} className="text-muted" />
          </button>
          <span aria-hidden="true" className="text-faint">
            ›
          </span>
          <BranchControl
            repository={repository}
            fetching={fetching}
            disabled={unavailable}
            onFetch={onFetch}
            onShowUpstream={(ref) =>
              showRefHistory({
                full_name: `refs/remotes/${ref}`,
                name: ref,
                kind: "remote_branch",
              })
            }
          />
        </nav>
        <div
          role="tablist"
          aria-label="Repository views"
          className="seg ml-2 shrink-0 @max-[820px]:hidden"
        >
          {TABS.map(([id, label, full], i) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              title={`${full ?? label} (⌘${i + 1})`}
              onClick={() => changeTab(id)}
            >
              {label}
              {tabCount(id)}
            </button>
          ))}
        </div>
        <div className="relative ml-1 hidden shrink-0 @max-[820px]:block">
          <button
            type="button"
            className="btn"
            aria-label={`Repository view: ${TABS.find((x) => x[0] === tab)?.[1]}`}
            aria-haspopup="menu"
            aria-expanded={tabMenuOpen}
            onClick={() => setTabMenuOpen(!tabMenuOpen)}
          >
            {TABS.find((x) => x[0] === tab)?.[1]}
            {TABS.filter(([id]) => id !== tab).map(([id]) => (
              <span key={id} className="contents">
                {tabCount(id)}
              </span>
            ))}
            <ChevronDown size={10} className="text-muted" />
          </button>
          {tabMenuOpen && (
            <Popover onClose={() => setTabMenuOpen(false)}>
              {TABS.map(([id, label, full], i) => (
                <MenuButton
                  key={id}
                  onClick={() => {
                    setTabMenuOpen(false);
                    changeTab(id);
                  }}
                >
                  <span className="flex-1">{full ?? label}</span>
                  {tabCount(id)}
                  <span className="kbd">⌘{i + 1}</span>
                </MenuButton>
              ))}
            </Popover>
          )}
        </div>
        <div data-tauri-drag-region className="h-full min-w-2 flex-1" />
        <OpenIn
          editor={editorName}
          onEditor={() => openInEditor()}
          onTerminal={() =>
            requestTerminal({ kind: "repository", id: repository.id })
          }
          onFinder={() => run(ipc.revealInFinder(repository.id))}
          onCopyPath={() =>
            void navigator.clipboard?.writeText(repository.display_path)
          }
        />
        {(tab === "pull_requests" || tab === "refs") && (
          <SidePanelButton open={panelOpen} onToggle={togglePanel} />
        )}
        <div className="relative shrink-0">
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
                disabled={fetching || unavailable}
                onClick={() => {
                  setMenuOpen(false);
                  onFetch();
                }}
              >
                <span className="flex-1">Fetch now</span>
                <span className="kbd">⇧⌘F</span>
              </MenuButton>
              <MenuButton
                onClick={() => {
                  setMenuOpen(false);
                  onRefresh();
                }}
              >
                <span className="flex-1">Refresh</span>
                <span className="kbd">⌘R</span>
              </MenuButton>
              <div className="menu-sep" />
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
  disabled,
  children,
}: {
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      className="menu-item"
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/**
 * Open in: one click opens the editor; its menu reveals the folder or copies
 * its path (SPEC.md, Main window v0.1). Narrow, it keeps only its icon.
 */
function OpenIn({
  editor,
  onEditor,
  onTerminal,
  onFinder,
  onCopyPath,
}: {
  /** The editor's name, such as "VS Code"; null for a custom one. */
  editor: string | null;
  onEditor: () => void;
  /** A terminal tab in the repository's folder (SPEC.md, section 15). */
  onTerminal: () => void;
  onFinder: () => void;
  onCopyPath: () => void;
}) {
  const [open, setOpen] = useState(false);
  const label = `Open in ${editor ?? "editor"}`;
  return (
    <div className="relative flex shrink-0">
      <button
        type="button"
        className="btn rounded-r-none"
        aria-label={label}
        title={`${label} (⇧⌘O)`}
        onClick={onEditor}
      >
        <ExternalIcon size={13} />
        <span className="@max-[1100px]:hidden">{label}</span>
      </button>
      <button
        type="button"
        className="btn rounded-l-none border-l-0 px-1.5"
        aria-label="Open in another app"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <ChevronDown size={10} />
      </button>
      {open && (
        <Popover align="right" onClose={() => setOpen(false)}>
          <MenuButton
            onClick={() => {
              setOpen(false);
              onEditor();
            }}
          >
            <span className="flex-1">{editor ?? "Editor"}</span>
            <span className="kbd">⇧⌘O</span>
          </MenuButton>
          <MenuButton
            onClick={() => {
              setOpen(false);
              onTerminal();
            }}
          >
            New terminal
          </MenuButton>
          <MenuButton
            onClick={() => {
              setOpen(false);
              onFinder();
            }}
          >
            Reveal in Finder
          </MenuButton>
          <div className="menu-sep" />
          <MenuButton
            onClick={() => {
              setOpen(false);
              onCopyPath();
            }}
          >
            Copy path
          </MenuButton>
        </Popover>
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
