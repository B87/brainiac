import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  type ChangeEntry,
  type ChangesResult,
  type CommitPage,
  type CommitSummary,
  type DiffResult,
  type DiffSelector,
  errorMessage,
  ipc,
  type RepositorySummary,
  type RepositoryTab,
} from "../lib/ipc";
import { createLatest } from "../lib/stale";
import ChangesList from "./ChangesList";
import CommitDetails from "./CommitDetails";
import DiffView from "./DiffView";
import HistoryList from "./HistoryList";

type Props = {
  repository: RepositorySummary;
  /** Increments when the backend reports this repository changed. */
  changeTick: number;
  onError: (message: string | null) => void;
};

export function selectorFor(entry: ChangeEntry): DiffSelector {
  switch (entry.group) {
    case "staged":
      return { kind: "index_vs_head", path: entry.path };
    case "untracked":
      return { kind: "untracked_preview", path: entry.path };
    default:
      return { kind: "worktree_vs_index", path: entry.path };
  }
}

export function entryKey(e: ChangeEntry): string {
  return `${e.group}:${e.path}`;
}

export default function RepositoryView({
  repository,
  changeTick,
  onError,
}: Props) {
  const [tab, setTab] = useState<RepositoryTab>(
    repository.last_tab ?? "changes",
  );
  const [changes, setChanges] = useState<ChangesResult | null>(null);
  const [selectedEntry, setSelectedEntry] = useState<string | null>(null);
  const [diff, setDiff] = useState<DiffResult | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [pages, setPages] = useState<CommitPage[]>([]);
  const [filter, setFilter] = useState("");
  const [selectedCommit, setSelectedCommit] = useState<CommitSummary | null>(
    null,
  );
  const [historyLoading, setHistoryLoading] = useState(false);

  // One stale-guard per data source, so a slow diff cannot clobber a newer one.
  const changesLatest = useRef(createLatest()).current;
  const diffLatest = useRef(createLatest()).current;
  const historyLatest = useRef(createLatest()).current;

  const loadChanges = useCallback(() => {
    void changesLatest.run(
      () => ipc.listChanges(repository.id),
      (result) => {
        if (result.repository_id !== repository.id) return;
        setChanges(result);
        setSelectedEntry((current) =>
          current && result.entries.some((e) => entryKey(e) === current)
            ? current
            : null,
        );
      },
      (e) => onError(errorMessage(e)),
    );
  }, [repository.id, changesLatest, onError]);

  const loadDiff = useCallback(
    (entry: ChangeEntry | null) => {
      if (!entry) {
        diffLatest.cancel();
        setDiff(null);
        setDiffLoading(false);
        return;
      }
      setDiffLoading(true);
      void diffLatest.run(
        () => ipc.getDiff(repository.id, selectorFor(entry)),
        (result) => {
          if (result.repository_id !== repository.id) return;
          setDiff(result);
          setDiffLoading(false);
        },
        (e) => {
          setDiffLoading(false);
          onError(errorMessage(e));
        },
      );
    },
    [repository.id, diffLatest, onError],
  );

  const loadHistory = useCallback(
    (cursor: string | null, replace: boolean) => {
      setHistoryLoading(true);
      void historyLatest.run(
        () =>
          ipc.listCommits({
            repository_id: repository.id,
            ref: null,
            filter: filter || null,
            cursor,
            limit: null,
          }),
        (page) => {
          if (page.repository_id !== repository.id) return;
          setPages((prev) => (replace ? [page] : [...prev, page]));
          setHistoryLoading(false);
        },
        (e) => {
          setHistoryLoading(false);
          onError(errorMessage(e));
        },
      );
    },
    [repository.id, filter, historyLatest, onError],
  );

  // Initial load and reload on backend change events.
  // biome-ignore lint/correctness/useExhaustiveDependencies: changeTick deliberately triggers a reload after backend events.
  useEffect(() => {
    loadChanges();
  }, [loadChanges, changeTick]);

  // Re-validate the displayed working-tree diff when the repository changes.
  const selected = useMemo(
    () => changes?.entries.find((e) => entryKey(e) === selectedEntry) ?? null,
    [changes, selectedEntry],
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: changeTick revalidates the diff even when the selected path is unchanged.
  useEffect(() => {
    loadDiff(selected);
  }, [selected, loadDiff, changeTick]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: changeTick refreshes history after backend events.
  useEffect(() => {
    if (tab !== "history") return;
    const t = setTimeout(() => loadHistory(null, true), filter ? 300 : 0);
    return () => clearTimeout(t);
  }, [tab, filter, loadHistory, changeTick]);

  const changeTab = (next: RepositoryTab) => {
    setTab(next);
    void ipc.setRepositoryTab(repository.id, next).catch(() => {});
  };

  const commits = useMemo(() => pages.flatMap((p) => p.items), [pages]);
  const nextCursor = pages.length ? pages[pages.length - 1].next_cursor : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-1 border-b px-2 py-1">
        <span className="mr-2 font-semibold">{repository.name}</span>
        <Tab
          label="Changes"
          count={changes?.counts.unique_paths}
          active={tab === "changes"}
          onClick={() => changeTab("changes")}
        />
        <Tab
          label="History"
          active={tab === "history"}
          onClick={() => changeTab("history")}
        />
        <Tab
          label="Branches"
          active={tab === "refs"}
          onClick={() => changeTab("refs")}
        />
        {tab === "history" && (
          <input
            className="selectable ml-auto w-64 rounded-md border px-2 py-0.5"
            placeholder="Filter commits by message or hash"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        )}
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="flex w-80 shrink-0 flex-col overflow-y-auto border-r">
          {tab === "changes" && (
            <ChangesList
              changes={changes}
              selectedKey={selectedEntry}
              onSelect={(e) => setSelectedEntry(entryKey(e))}
            />
          )}
          {tab === "history" && (
            <HistoryList
              commits={commits}
              anchor={pages[0]?.anchor_commit_id ?? null}
              refName={pages[0]?.ref ?? "HEAD"}
              selectedId={selectedCommit?.id ?? null}
              loading={historyLoading}
              hasMore={!!nextCursor}
              filterActive={!!filter}
              onSelect={setSelectedCommit}
              onLoadMore={() => nextCursor && loadHistory(nextCursor, false)}
            />
          )}
          {tab === "refs" && (
            <div className="muted p-3">Branches and tags arrive in v0.1.</div>
          )}
        </div>
        <div className="flex min-w-0 flex-1 flex-col overflow-auto">
          {tab === "changes" && (
            <DiffView
              diff={diff}
              loading={diffLoading}
              empty={
                changes
                  ? changes.entries.length === 0
                    ? "Working tree clean."
                    : "Select a file to see its diff."
                  : "Loading…"
              }
              onOpenInEditor={(path) =>
                void ipc
                  .openInEditor(repository.id, path)
                  .catch((e) => onError(errorMessage(e)))
              }
            />
          )}
          {tab === "history" && <CommitDetails commit={selectedCommit} />}
          {tab === "refs" && null}
        </div>
      </div>
    </div>
  );
}

function Tab({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count?: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={`rounded-md px-2 py-0.5 ${active ? "selected-bg" : ""}`}
      aria-pressed={active}
      onClick={onClick}
    >
      {label}
      {count !== undefined && count > 0 && (
        <span className="muted ml-1">{count}</span>
      )}
    </button>
  );
}
