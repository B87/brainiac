import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { absoluteTime, relativeTime } from "../lib/format";
import {
  type ChangedFile,
  type Comment,
  type Conversation,
  errorMessage,
  ipc,
  onPullRequestChanged,
  type PullRequest,
  type PullRequestChecks,
  type PullRequestDiff,
  type PullRequestFiles,
  subscribe,
  type Thread,
} from "../lib/ipc";
import { useKeys } from "../lib/keys";
import { usePref } from "../lib/prefs";
import {
  anchorLabel,
  CHECK_LABEL,
  checksLabel,
  fileKey,
  isGenerated,
  isViewed,
  loadViewed,
  PROVIDER_LABEL,
  REVIEW_LABEL,
  STATE_LABEL,
  saveViewed,
  sinceReviewLabel,
  sizeLabel,
  threadCounts,
  unresolvedThreads,
  type ViewedMarks,
} from "../lib/pullRequests";
import { plural, splitPath } from "../lib/repo";
import { createLatest } from "../lib/stale";
import DiffView from "./DiffView";
import { Avatar } from "./HistoryTab";
import {
  ChevronLeft,
  CommentIcon,
  ExternalIcon,
  FolderIcon,
  RefreshIcon,
  SidebarIcon,
} from "./icons";
import { Markdown } from "./Markdown";
import { StateIcon } from "./PullRequestsTab";

type Tab = "overview" | "files" | "checks";
/** All changes, or only those since the account's last review. */
type Scope = "all" | "since";

type Props = {
  reference: string;
  onBack: () => void;
  onError: (message: string | null) => void;
};

/** The pull request on screen is read again after this long (SPEC.md, Staying up to date). */
const DETAIL_MAX_AGE = 60;
const REFRESH_MS = 60_000;

/** One pull request: its overview, the files it changes, and its checks (SPEC.md, Pull request). */
export default function PullRequestView({ reference, onBack, onError }: Props) {
  const [tab, setTab] = useState<Tab>("overview");
  const [scope, setScope] = useState<Scope>("all");
  const [pr, setPr] = useState<PullRequest | null>(null);
  const [conversation, setConversation] = useState<Conversation | null>(null);
  const [files, setFiles] = useState<PullRequestFiles | null>(null);
  const [sinceFiles, setSinceFiles] = useState<PullRequestFiles | null>(null);
  const [sinceError, setSinceError] = useState<string | null>(null);
  const [checks, setChecks] = useState<PullRequestChecks | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const latest = useRef(createLatest()).current;
  const conversationLatest = useRef(createLatest()).current;
  const filesLatest = useRef(createLatest()).current;
  const sinceLatest = useRef(createLatest()).current;
  const checksLatest = useRef(createLatest()).current;

  const load = useCallback(
    (maxAge: number) => {
      setRefreshing(true);
      void latest.run(
        () => ipc.getPullRequest(reference, maxAge),
        (result) => {
          setRefreshing(false);
          setPr(result);
        },
        (e) => {
          setRefreshing(false);
          onError(errorMessage(e));
        },
      );
      void conversationLatest.run(
        () => ipc.getPullRequestConversation(reference, maxAge),
        setConversation,
        (e) => onError(errorMessage(e)),
      );
    },
    [reference, latest, conversationLatest, onError],
  );

  useEffect(() => {
    load(DETAIL_MAX_AGE);
    const timer = setInterval(() => load(DETAIL_MAX_AGE), REFRESH_MS);
    return () => clearInterval(timer);
  }, [load]);

  useEffect(
    () =>
      subscribe(
        onPullRequestChanged((e) => {
          if (e.reference === reference) load(DETAIL_MAX_AGE);
        }),
      ),
    [reference, load],
  );

  // Files and checks follow the head commit: read again when it moved.
  const head = pr?.head_sha ?? null;
  useEffect(() => {
    if (tab !== "files" || !head) return;
    void filesLatest.run(
      () => ipc.listPullRequestFiles(reference, false),
      setFiles,
      (e) => onError(errorMessage(e)),
    );
  }, [tab, head, reference, filesLatest, onError]);
  const reviewed = pr?.reviewed_sha ?? null;
  useEffect(() => {
    if (tab !== "files" || scope !== "since" || !head || !reviewed) return;
    setSinceError(null);
    void sinceLatest.run(
      () => ipc.listPullRequestFiles(reference, true),
      (result) => {
        setSinceFiles(result);
        setSinceError(null);
      },
      (e) => {
        setSinceFiles(null);
        setSinceError(errorMessage(e));
      },
    );
  }, [tab, scope, head, reviewed, reference, sinceLatest]);
  useEffect(() => {
    if (tab !== "checks" || !head) return;
    void checksLatest.run(
      () => ipc.getPullRequestChecks(reference, DETAIL_MAX_AGE),
      setChecks,
      (e) => onError(errorMessage(e)),
    );
  }, [tab, head, reference, checksLatest, onError]);

  useKeys({
    "mod+1": () => setTab("overview"),
    "mod+2": () => setTab("files"),
    "mod+3": () => setTab("checks"),
    Escape: onBack,
  });

  const [, repoAndNumber] = reference.match(/^[^/]+\/(.+)$/) ?? [
    null,
    reference,
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center gap-3 border-b bg-header pr-3 pl-3"
      >
        <button
          type="button"
          className="btn"
          onClick={onBack}
          title="Back (Esc)"
          aria-label="Back"
        >
          <ChevronLeft size={13} />
        </button>
        <span className="mono truncate text-[12.5px] text-fg-2">
          {repoAndNumber}
        </span>
        {pr && (
          <span className="text-[11.5px] text-muted">
            {PROVIDER_LABEL[pr.kind]}
          </span>
        )}
        <div role="tablist" aria-label="Pull request views" className="seg">
          <button
            type="button"
            role="tab"
            aria-selected={tab === "overview"}
            title="Overview (⌘1)"
            onClick={() => setTab("overview")}
          >
            Overview
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "files"}
            title="Files changed (⌘2)"
            onClick={() => setTab("files")}
          >
            Files Changed
            {pr?.counts.changed_files !== null &&
              pr?.counts.changed_files !== undefined && (
                <span className="text-muted">{pr.counts.changed_files}</span>
              )}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "checks"}
            title="Checks (⌘3)"
            onClick={() => setTab("checks")}
          >
            Checks
          </button>
        </div>
        <div data-tauri-drag-region className="h-full flex-1" />
        <button
          type="button"
          className="btn"
          disabled={refreshing}
          onClick={() => load(0)}
          title="Read the pull request again"
        >
          <RefreshIcon
            size={13}
            className={refreshing ? "animate-pulse" : ""}
          />
        </button>
        {pr && (
          <button
            type="button"
            className="btn"
            onClick={() => void openUrl(pr.web_url).catch(() => {})}
            title={`Open on ${PROVIDER_LABEL[pr.kind]}`}
          >
            <ExternalIcon size={13} />
            Open on {PROVIDER_LABEL[pr.kind]}
          </button>
        )}
      </header>

      {!pr ? (
        <div className="p-6 text-muted">Loading…</div>
      ) : tab === "files" ? (
        <FilesTab
          key={reference}
          pr={pr}
          files={files}
          sinceFiles={sinceFiles}
          sinceError={sinceError}
          scope={scope}
          onScope={setScope}
          conversation={conversation}
          onError={onError}
        />
      ) : tab === "checks" ? (
        <ChecksTab pr={pr} checks={checks} />
      ) : (
        <Overview
          pr={pr}
          conversation={conversation}
          onShowSinceReview={() => {
            setScope("since");
            setTab("files");
          }}
        />
      )}
    </div>
  );
}

function Overview({
  pr,
  conversation,
  onShowSinceReview,
}: {
  pr: PullRequest;
  conversation: Conversation | null;
  onShowSinceReview: () => void;
}) {
  const since = sinceReviewLabel(pr);
  const unresolved = conversation
    ? unresolvedThreads(conversation.threads)
    : pr.counts.unresolved_threads;
  return (
    <div className="flex min-h-0 flex-1">
      <section
        aria-label="Overview"
        className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-6 py-5"
      >
        <div className="flex flex-col gap-2">
          <h2 className="m-0 flex items-center gap-2.5 text-[18px] font-semibold">
            <span className="selectable">{pr.title}</span>
            <span className="mono text-[13px] font-normal text-muted">
              #{pr.number}
            </span>
          </h2>
          <div className="flex flex-wrap items-center gap-2 text-[12.5px] text-fg-2">
            <span className="pr-state" data-state={pr.state}>
              {STATE_LABEL[pr.state]}
            </span>
            <span className="flex items-center gap-1.5">
              <Avatar
                name={pr.author.display_name ?? pr.author.login}
                size={18}
              />
              {pr.author.login}
            </span>
            <span className="text-muted">·</span>
            <span>
              wants to merge <span className="mono">{pr.source_branch}</span>
              {pr.source_repository !==
                pr.reference.replace(/^[^/]+\//, "").replace(/#\d+$/, "") && (
                <>
                  {" "}
                  from <span className="mono">{pr.source_repository}</span>
                </>
              )}{" "}
              into <span className="mono">{pr.target_branch}</span>
            </span>
            <span className="text-muted">·</span>
            <span className="mono" title={pr.head_sha}>
              {pr.head_sha.slice(0, 10)}
            </span>
            <span className="text-muted">·</span>
            <span title={new Date(pr.created_at).toLocaleString()}>
              opened {relativeTime(pr.created_at)}
            </span>
            {pr.closed_at && (
              <span title={new Date(pr.closed_at).toLocaleString()}>
                , {pr.state} {relativeTime(pr.closed_at)}
              </span>
            )}
          </div>
          {since && (
            <div className="flex items-center gap-2 text-[12.5px]">
              <span className="font-medium text-accent">{since}</span>
              <button
                type="button"
                className="btn btn-sm"
                onClick={onShowSinceReview}
              >
                Show changes since your review
              </button>
            </div>
          )}
        </div>
        {pr.description.trim() ? (
          <div className="rounded-lg border bg-panel px-4 py-3">
            <Markdown html={pr.description_html} />
          </div>
        ) : (
          <span className="text-[12.5px] text-muted">No description.</span>
        )}
        <ConversationSection conversation={conversation} pr={pr} />
      </section>
      <aside
        aria-label="Merge readiness and reviewers"
        className="flex w-[300px] shrink-0 flex-col overflow-y-auto border-l bg-panel"
      >
        <div className="flex flex-col gap-2 border-b px-[18px] py-3.5">
          <span className="section-label text-fg-2">Before merging</span>
          <ul className="m-0 flex list-none flex-col gap-1.5 p-0 text-[12.5px]">
            <li className="flex items-center gap-2">
              <StateIcon
                state={
                  pr.checks.state === null || pr.checks.state === "success"
                    ? "success"
                    : pr.checks.state
                }
                label={
                  pr.checks.state ? CHECK_LABEL[pr.checks.state] : "No checks"
                }
              />
              {checksLabel(pr.checks)}
            </li>
            <li className="flex items-center gap-2">
              <StateIcon
                state={
                  pr.reviewers.some((r) => r.state === "changes_requested")
                    ? "changes_requested"
                    : pr.reviewers.some((r) => r.state === "approved")
                      ? "approved"
                      : "requested"
                }
                label="Reviews"
              />
              {pr.reviewers.filter((r) => r.state === "approved").length === 0
                ? "No approvals yet"
                : plural(
                    pr.reviewers.filter((r) => r.state === "approved").length,
                    "approval",
                  )}
            </li>
            <li className="flex items-center gap-2">
              <StateIcon
                state={
                  unresolved === 0
                    ? "success"
                    : unresolved === null
                      ? "neutral"
                      : "pending"
                }
                label="Threads"
              />
              {unresolved === null
                ? "Threads not read yet"
                : unresolved === 0
                  ? "No unresolved threads"
                  : plural(unresolved, "unresolved thread")}
            </li>
            <li className="flex items-center gap-2">
              <StateIcon
                state={
                  pr.mergeability === "mergeable"
                    ? "success"
                    : pr.mergeability === "conflicting"
                      ? "failure"
                      : "neutral"
                }
                label="Conflicts"
              />
              {pr.mergeability === "mergeable"
                ? "No conflicts"
                : pr.mergeability === "conflicting"
                  ? "Has conflicts with the target"
                  : pr.mergeability === "computing"
                    ? "Checking for conflicts…"
                    : "Conflicts are found when merging"}
            </li>
          </ul>
          <button
            type="button"
            className="btn btn-primary self-start"
            disabled
            title={pr.actions.merge.reason ?? "Merging comes in a later step."}
          >
            Merge
          </button>
          <span className="text-[11.5px] text-muted">
            {pr.actions.merge.reason ??
              "Merging from Brainiac comes in a later step."}
          </span>
        </div>
        <div className="flex flex-col gap-1.5 border-b px-[18px] py-3.5">
          <span className="section-label text-fg-2">Reviewers</span>
          {pr.reviewers.length === 0 ? (
            <span className="text-[12.5px] text-muted">No reviewers.</span>
          ) : (
            pr.reviewers.map((r) => (
              <span
                key={r.user.id}
                className="flex items-center gap-2 text-[12.5px]"
              >
                <StateIcon state={r.state} label={REVIEW_LABEL[r.state]} />
                <span className={r.is_me ? "font-medium" : ""}>
                  {r.user.display_name ?? r.user.login}
                </span>
                <span className="text-[11.5px] text-muted">
                  {REVIEW_LABEL[r.state]}
                </span>
              </span>
            ))
          )}
        </div>
        <div className="flex flex-col gap-1.5 px-[18px] py-3.5">
          <span className="section-label text-fg-2">Size</span>
          <span className="text-[12.5px] text-fg-2">
            {sizeLabel(pr)}
            {pr.counts.commits !== null &&
              ` · ${plural(pr.counts.commits, "commit")}`}
          </span>
        </div>
      </aside>
    </div>
  );
}

// --- The conversation -------------------------------------------------------

function ConversationSection({
  conversation,
  pr,
}: {
  conversation: Conversation | null;
  pr: PullRequest;
}) {
  return (
    <section aria-label="Conversation" className="flex flex-col gap-2.5">
      <h3 className="section-label m-0 text-fg-2">
        Conversation
        {conversation && (
          <span className="ml-1.5 font-normal text-muted">
            {plural(conversation.threads.length, "thread")}
          </span>
        )}
      </h3>
      {!conversation ? (
        <span className="text-[12.5px] text-muted">
          Reading the conversation…
        </span>
      ) : conversation.threads.length === 0 ? (
        <span className="text-[12.5px] text-muted">No comments yet.</span>
      ) : (
        conversation.threads.map((t) => <ThreadCard key={t.id} thread={t} />)
      )}
      <span className="text-[12px] text-muted">
        Commenting, replying, and reviewing come in the next step; until then,
        on {PROVIDER_LABEL[pr.kind]}.
      </span>
    </section>
  );
}

/** One thread; resolved ones are folded (SPEC.md, Pull request: Overview). */
function ThreadCard({ thread }: { thread: Thread }) {
  const where = thread.anchor ? anchorLabel(thread.anchor) : null;
  const comments = thread.comments.map((c) => (
    <CommentView key={c.id} comment={c} />
  ));
  if (thread.resolved)
    return (
      <details className="thread rounded-lg border bg-panel px-4 py-2">
        <summary className="cursor-default select-none text-[12.5px] text-muted">
          <StateIcon state="success" label="Resolved" />
          <span className="ml-1.5">Resolved</span>
          {where && <span className="mono ml-2 text-[11.5px]">{where}</span>}
          <span className="ml-2">
            · {plural(thread.comments.length, "comment")}
          </span>
        </summary>
        <div className="flex flex-col gap-3 pt-2 pb-1">{comments}</div>
      </details>
    );
  return (
    <div
      className="thread flex flex-col gap-3 rounded-lg border bg-panel px-4 py-3"
      data-outdated={thread.outdated || undefined}
    >
      {where && (
        <div className="flex items-center gap-2 text-[11.5px] text-fg-2">
          <CommentIcon size={12} className="text-muted" />
          <span className="mono">{where}</span>
          {thread.outdated && (
            <span
              className="pr-state"
              title="The lines this thread was on have changed since"
            >
              Outdated
            </span>
          )}
        </div>
      )}
      {comments}
    </div>
  );
}

function CommentView({ comment }: { comment: Comment }) {
  const name = comment.author.display_name ?? comment.author.login;
  return (
    <article className="flex gap-2.5">
      <Avatar name={name} size={22} />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[12px] text-fg-2">
          <span className={`text-fg ${comment.mine ? "font-medium" : ""}`}>
            {name}
          </span>
          {comment.review && (
            <span className="flex items-center gap-1">
              <StateIcon
                state={comment.review}
                label={REVIEW_LABEL[comment.review]}
              />
              {comment.review === "approved"
                ? "approved"
                : comment.review === "changes_requested"
                  ? "requested changes"
                  : "reviewed"}
            </span>
          )}
          <span className="text-muted" title={absoluteTime(comment.created_at)}>
            {relativeTime(comment.created_at)}
          </span>
          {comment.updated_at && (
            <span
              className="text-muted"
              title={`Edited ${absoluteTime(comment.updated_at)}`}
            >
              edited
            </span>
          )}
        </div>
        {comment.body.trim() ? (
          <Markdown html={comment.html} className="text-[13px]" />
        ) : (
          <span className="text-[12.5px] text-muted">No comment.</span>
        )}
      </div>
    </article>
  );
}

// --- Files changed ------------------------------------------------------------

const LETTER: Record<ChangedFile["status"], string> = {
  added: "A",
  modified: "M",
  removed: "D",
  renamed: "R",
  other: "?",
};

const TONE: Record<ChangedFile["status"], string> = {
  added: "add",
  modified: "mod",
  removed: "del",
  renamed: "ren",
  other: "other",
};

/** Files grouped by folder, root files first, as the commit view lists them. */
function groupByDir(
  files: ChangedFile[],
): Array<{ dir: string; files: ChangedFile[] }> {
  const byDir = new Map<string, ChangedFile[]>();
  for (const f of files) {
    const { dir } = splitPath(f.path);
    const list = byDir.get(dir);
    if (list) list.push(f);
    else byDir.set(dir, [f]);
  }
  return [...byDir.entries()]
    .sort(([a], [b]) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)))
    .map(([dir, list]) => ({
      dir,
      files: list.sort((a, b) => a.path.localeCompare(b.path)),
    }));
}

function FilesTab({
  pr,
  files,
  sinceFiles,
  sinceError,
  scope,
  onScope,
  conversation,
  onError,
}: {
  pr: PullRequest;
  files: PullRequestFiles | null;
  sinceFiles: PullRequestFiles | null;
  sinceError: string | null;
  scope: Scope;
  onScope: (scope: Scope) => void;
  conversation: Conversation | null;
  onError: (message: string | null) => void;
}) {
  const [showFiles, setShowFiles] = usePref("brainiac.pr.files", true);
  const [ignoreWhitespace, setIgnoreWhitespace] = usePref(
    "brainiac.diff.ignoreWhitespace",
    false,
  );
  const [filter, setFilter] = useState("");
  const [marks, setMarks] = useState<ViewedMarks>(() =>
    loadViewed(pr.reference),
  );
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [diff, setDiff] = useState<PullRequestDiff | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const diffLatest = useRef(createLatest()).current;

  const canSince = !!pr.reviewed_sha && pr.reviewed_sha !== pr.head_sha;
  const list = scope === "since" ? sinceFiles : files;
  const counts = useMemo(
    () => threadCounts(conversation?.threads ?? []),
    [conversation],
  );

  // Sections: files to look at, then viewed and generated ones folded. The
  // selected file stays in sight wherever it belongs.
  const sections = useMemo(() => {
    const all = list?.files ?? [];
    const needle = filter.trim().toLowerCase();
    const shown = needle
      ? all.filter((f) => f.path.toLowerCase().includes(needle))
      : all;
    const main: ChangedFile[] = [];
    const viewed: ChangedFile[] = [];
    const generated: ChangedFile[] = [];
    for (const f of shown) {
      if (f.path === selectedPath) main.push(f);
      else if (isViewed(marks, f)) viewed.push(f);
      else if (isGenerated(f.path)) generated.push(f);
      else main.push(f);
    }
    return { all, shown, main, viewed, generated };
  }, [list, filter, marks, selectedPath]);
  const ordered = useMemo(
    () => [
      ...groupByDir(sections.main).flatMap((g) => g.files),
      ...groupByDir(sections.viewed).flatMap((g) => g.files),
      ...groupByDir(sections.generated).flatMap((g) => g.files),
    ],
    [sections],
  );

  // The first file to look at, once the list is here or the scope changed:
  // one not viewed or generated when there is one. The marks are read
  // through a ref so a click's own mark does not move the selection.
  const marksRef = useRef(marks);
  marksRef.current = marks;
  useEffect(() => {
    if (!list) return;
    setSelectedPath((current) => {
      if (current && list.files.some((f) => f.path === current)) return current;
      const first = (files: ChangedFile[]) =>
        groupByDir(files).flatMap((g) => g.files)[0]?.path ?? null;
      return (
        first(
          list.files.filter(
            (f) => !isGenerated(f.path) && !isViewed(marksRef.current, f),
          ),
        ) ?? first(list.files)
      );
    });
  }, [list]);

  const selected = list?.files.find((f) => f.path === selectedPath) ?? null;
  const since = scope === "since";
  // biome-ignore lint/correctness/useExhaustiveDependencies: `list` stands for the head and scope the patch belongs to.
  useEffect(() => {
    if (!selected) {
      diffLatest.cancel();
      setDiff(null);
      setDiffLoading(false);
      return;
    }
    setDiffLoading(true);
    void diffLatest.run(
      () =>
        ipc.getPullRequestDiff({
          reference: pr.reference,
          path: selected.path,
          old_path: selected.old_path,
          since_review: since,
          options: { ignore_whitespace: ignoreWhitespace },
        }),
      (result) => {
        setDiffLoading(false);
        setDiff(result);
      },
      (e) => {
        setDiffLoading(false);
        onError(errorMessage(e));
      },
    );
  }, [
    pr.reference,
    selected?.path,
    selected?.old_path,
    since,
    list,
    ignoreWhitespace,
    diffLatest,
  ]);

  const mark = (f: ChangedFile, viewed: boolean) => {
    const next = { ...marks };
    if (viewed) next[f.path] = fileKey(f);
    else delete next[f.path];
    setMarks(next);
    saveViewed(pr.reference, next);
  };
  /** Clicking a file's row shows it and marks it viewed (SPEC.md, Files Changed). */
  const select = (f: ChangedFile) => {
    setSelectedPath(f.path);
    if (!isViewed(marks, f)) mark(f, true);
  };
  const index = ordered.findIndex((f) => f.path === selectedPath);
  const step = (delta: number) => {
    const next = ordered[index + delta];
    if (next) select(next);
  };
  const openInEditor = (path: string, line?: number) => {
    if (!diff) return;
    void ipc
      .openInEditor(diff.diff.repository_id, path, line)
      .catch((e) => onError(errorMessage(e)));
  };

  const empty = !list
    ? since && sinceError
      ? sinceError
      : "Reading the files…"
    : list.files.length === 0
      ? since
        ? "Nothing changed since your review."
        : "This pull request changes no files."
      : "Select a file to see its diff.";
  const meta = diff
    ? diff.source === "local"
      ? `from local Git, ${diff.base_sha.slice(0, 7)}…${diff.head_sha.slice(0, 7)}`
      : `from ${PROVIDER_LABEL[pr.kind]}`
    : undefined;

  const scopeControl = (
    <div role="tablist" aria-label="Which changes" className="seg seg-sm">
      <button
        type="button"
        role="tab"
        aria-selected={scope === "all"}
        onClick={() => onScope("all")}
      >
        All changes
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={scope === "since"}
        disabled={!canSince}
        title={
          canSince
            ? "Only what changed since the commit you last reviewed, from local Git"
            : pr.reviewed_sha
              ? "Nothing new since your review"
              : "You have not reviewed this pull request yet"
        }
        onClick={() => onScope("since")}
      >
        Since your review
      </button>
    </div>
  );

  return (
    <div className="flex min-h-0 flex-1">
      {showFiles && (
        <div className="flex w-72 shrink-0 flex-col border-r bg-panel-2">
          <div className="flex h-9 shrink-0 items-center gap-2 border-b pr-1.5 pl-3 text-[12px]">
            {list ? (
              <>
                <span className="font-semibold">
                  {plural(list.files.length, "file")}
                </span>
                <span className="text-add">
                  +{list.files.reduce((n, f) => n + f.additions, 0)}
                </span>
                <span className="text-del">
                  −{list.files.reduce((n, f) => n + f.deletions, 0)}
                </span>
              </>
            ) : (
              <span className="text-muted">
                {since && sinceError ? "Not available" : "Loading files…"}
              </span>
            )}
            <span className="flex-1" />
            <button
              type="button"
              className="btn btn-sm btn-ghost w-6 px-0"
              aria-label="Hide file list"
              title="Hide file list, giving the diff the full width"
              onClick={() => setShowFiles(false)}
            >
              <SidebarIcon size={13} />
            </button>
          </div>
          <div className="flex flex-col gap-2 border-b px-3 py-2">
            {scopeControl}
            <input
              type="search"
              className="text-input h-7 text-[12px]"
              placeholder="Filter files"
              aria-label="Filter files"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto py-1.5">
            {list && sections.shown.length === 0 && (
              <div className="px-3 py-2 text-[12px] text-muted">
                {filter ? "No file matches." : empty}
              </div>
            )}
            {!list && since && sinceError && (
              <div className="px-3 py-2 text-[12px] text-muted">
                {sinceError}
              </div>
            )}
            <FileGroups
              files={sections.main}
              selectedPath={selectedPath}
              marks={marks}
              counts={counts}
              onSelect={select}
              onMark={mark}
            />
            {sections.viewed.length > 0 && (
              <details className="pr-fold">
                <summary>
                  {plural(sections.viewed.length, "viewed file")}
                </summary>
                <FileGroups
                  files={sections.viewed}
                  selectedPath={selectedPath}
                  marks={marks}
                  counts={counts}
                  onSelect={select}
                  onMark={mark}
                />
              </details>
            )}
            {sections.generated.length > 0 && (
              <details className="pr-fold">
                <summary>
                  {plural(sections.generated.length, "generated file")}
                </summary>
                <FileGroups
                  files={sections.generated}
                  selectedPath={selectedPath}
                  marks={marks}
                  counts={counts}
                  onSelect={select}
                  onMark={mark}
                />
              </details>
            )}
          </div>
        </div>
      )}
      <div className="flex min-w-0 flex-1 flex-col">
        <DiffView
          diff={diff?.diff ?? null}
          loading={diffLoading}
          historical
          ignoreWhitespace={ignoreWhitespace}
          onIgnoreWhitespace={setIgnoreWhitespace}
          ignoreWhitespaceNote={
            diff?.source === "provider"
              ? `Whitespace can only be ignored in a diff from local Git; this one comes from ${PROVIDER_LABEL[pr.kind]}.`
              : undefined
          }
          stepper={
            ordered.length > 0 && index >= 0
              ? {
                  index,
                  total: ordered.length,
                  onPrev: () => step(-1),
                  onNext: () => step(1),
                }
              : undefined
          }
          extra={
            !showFiles && (
              <>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => setShowFiles(true)}
                >
                  <SidebarIcon size={13} />
                  Show files
                </button>
                {scopeControl}
              </>
            )
          }
          meta={meta}
          empty={empty}
          onOpenInEditor={openInEditor}
        />
      </div>
    </div>
  );
}

function FileGroups({
  files,
  selectedPath,
  marks,
  counts,
  onSelect,
  onMark,
}: {
  files: ChangedFile[];
  selectedPath: string | null;
  marks: ViewedMarks;
  counts: Map<string, { total: number; open: number }>;
  onSelect: (f: ChangedFile) => void;
  onMark: (f: ChangedFile, viewed: boolean) => void;
}) {
  return groupByDir(files).map((g) => (
    <div key={g.dir}>
      {g.dir && (
        <div
          className="flex h-[26px] items-center gap-[7px] px-3 text-[12.5px] text-fg-2"
          title={g.dir}
        >
          <FolderIcon size={13} className="shrink-0 text-muted" />
          <span className="truncate">{g.dir}</span>
        </div>
      )}
      {g.files.map((f) => (
        <FileRow
          key={f.path}
          file={f}
          nested={!!g.dir}
          selected={f.path === selectedPath}
          viewed={isViewed(marks, f)}
          threads={counts.get(f.path) ?? null}
          onSelect={() => onSelect(f)}
          onMark={(viewed) => onMark(f, viewed)}
        />
      ))}
    </div>
  ));
}

function FileRow({
  file,
  nested,
  selected,
  viewed,
  threads,
  onSelect,
  onMark,
}: {
  file: ChangedFile;
  nested: boolean;
  selected: boolean;
  viewed: boolean;
  threads: { total: number; open: number } | null;
  onSelect: () => void;
  onMark: (viewed: boolean) => void;
}) {
  const stats = file.binary
    ? "binary"
    : `+${file.additions} −${file.deletions}`;
  const label = file.old_path ? `${file.old_path} → ${file.path}` : file.path;
  return (
    <div
      className={`flex items-center pr-2 ${selected ? "bg-control" : ""}`}
      data-viewed={viewed || undefined}
    >
      <button
        type="button"
        className={`list-row h-[26px] min-w-0 flex-1 items-center gap-[7px] text-[12.5px] ${nested ? "pl-[25px]" : "pl-[9px]"}`}
        aria-current={selected}
        onClick={onSelect}
        title={`${label} (${stats})`}
      >
        <span
          className="mono w-[13px] shrink-0 text-center text-[11px] font-semibold"
          style={{ color: `var(--k-${TONE[file.status]}-fg)` }}
        >
          {LETTER[file.status]}
        </span>
        <span
          className={`min-w-0 flex-1 truncate ${viewed ? "text-muted" : ""}`}
        >
          {splitPath(file.path).name}
        </span>
        {threads && (
          <span
            className={`flex shrink-0 items-center gap-0.5 text-[11px] ${threads.open ? "text-fg-2" : "text-muted"}`}
            title={`${plural(threads.total, "thread")}, ${threads.open} open`}
          >
            <CommentIcon size={11} />
            {threads.open || threads.total}
          </span>
        )}
        {!file.binary && (
          <span className="shrink-0 text-[11px] tabular text-muted">
            <span className="text-add">+{file.additions}</span>{" "}
            <span className="text-del">−{file.deletions}</span>
          </span>
        )}
      </button>
      <input
        type="checkbox"
        className="ml-1 shrink-0"
        aria-label={`Viewed ${file.path}`}
        title={
          viewed
            ? "Viewed; the mark clears when the file changes"
            : "Mark as viewed"
        }
        checked={viewed}
        onChange={(e) => onMark(e.target.checked)}
      />
    </div>
  );
}

// --- Checks ------------------------------------------------------------------

function ChecksTab({
  pr,
  checks,
}: {
  pr: PullRequest;
  checks: PullRequestChecks | null;
}) {
  if (!checks) return <div className="p-6 text-muted">Reading the checks…</div>;
  return (
    <section
      aria-label="Checks"
      className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-6 py-5"
    >
      <span className="text-[12px] text-muted">
        {checks.checks.length === 0
          ? "No checks ran on this commit."
          : `${checksLabel(pr.checks)} at ${checks.head_sha.slice(0, 10)}, read ${relativeTime(checks.fetched_at)}.`}
      </span>
      <div className="flex flex-col gap-2">
        {checks.checks.map((c) => (
          <div
            key={`${c.name}:${c.url ?? ""}`}
            className="flex items-center gap-2.5 rounded-lg border bg-panel px-3.5 py-2.5 text-[12.5px]"
          >
            <StateIcon state={c.state} label={CHECK_LABEL[c.state]} />
            <span className="font-medium">{c.name}</span>
            <span className="text-[11.5px] text-muted">
              {CHECK_LABEL[c.state]}
            </span>
            {c.description && (
              <span className="min-w-0 flex-1 truncate text-fg-2">
                {c.description}
              </span>
            )}
            {!c.description && <span className="flex-1" />}
            {c.url && (
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => c.url && void openUrl(c.url).catch(() => {})}
              >
                <ExternalIcon size={12} />
                Log
              </button>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}
