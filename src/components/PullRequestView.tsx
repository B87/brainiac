import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { depthLabel, sameSubject, subjectLabel } from "../lib/explain";
import { absoluteTime, relativeTime } from "../lib/format";
import {
  type ChangedFile,
  type Comment,
  type Conversation,
  type DiffLine,
  errorMessage,
  ipc,
  isAppError,
  type MergeMethod,
  type MergeOptions,
  type MergeOutcome,
  onPullRequestChanged,
  type PullRequest,
  type PullRequestChecks,
  type PullRequestDiff,
  type PullRequestFiles,
  type RepositoryNotes,
  type RepositorySummary,
  type ReviewDraft,
  type ReviewDrafts,
  type ReviewVerdict,
  subscribe,
  type Thread,
  type WriteOutcome,
} from "../lib/ipc";
import { useKeys } from "../lib/keys";
import { usePref } from "../lib/prefs";
import {
  anchorLabel,
  CHECK_LABEL,
  type ChecklistItem,
  checksLabel,
  defaultMergeMessage,
  draftsBehind,
  fileKey,
  isGenerated,
  isViewed,
  lineKey,
  lineNotes,
  loadViewed,
  MERGE_METHOD_LABEL,
  mergeChecklist,
  PROVIDER_LABEL,
  REVIEW_LABEL,
  reviewIncomplete,
  reviewStartCommit,
  STATE_LABEL,
  sameCommit,
  saveViewed,
  sinceReviewLabel,
  sizeLabel,
  threadCounts,
  unresolvedThreads,
  unsentDrafts,
  VERDICT_LABEL,
  type ViewedMarks,
} from "../lib/pullRequests";
import { plural, splitPath } from "../lib/repo";
import { useSidePanel } from "../lib/sidePanel";
import { createLatest } from "../lib/stale";
import { useExplanation } from "../lib/useExplanation";
import Dialog from "./Dialog";
import DiffView, { type LineTarget } from "./DiffView";
import { Avatar } from "./HistoryTab";
import {
  BulbIcon,
  ChevronLeft,
  CommentIcon,
  ExternalIcon,
  FolderIcon,
  NoteIcon,
  RefreshIcon,
  SidebarIcon,
} from "./icons";
import { Markdown } from "./Markdown";
import { StateIcon } from "./PullRequestsTab";
import SidePanelButton from "./SidePanelButton";
import {
  composeAnnotate,
  type FileNoteCount,
  FileNotes,
  NotCoveredHeading,
  OrderSwitch,
  startsNotCovered,
  useExplainBlocked,
  useExplainedPatch,
} from "./useExplainedPatch";

type Tab = "overview" | "files" | "checks";
/** All changes, those since the account's last review, or since its drafts. */
type Scope = "all" | "review" | "drafts";

type Props = {
  reference: string;
  onBack: () => void;
  onOpenNote: (noteId: string) => void;
  onError: (message: string | null) => void;
};

/** The pull request on screen is read again after this long (SPEC.md, Staying up to date). */
const DETAIL_MAX_AGE = 60;
const REFRESH_MS = 60_000;

const VERDICTS: ReviewVerdict[] = ["comment", "approve", "request_changes"];

/** ⌘↩ in a comment box sends it. */
function submitOnEnter(submit: () => void) {
  return (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && e.metaKey) {
      e.preventDefault();
      submit();
    }
  };
}

/** One pull request: its overview, the files it changes, and its checks (SPEC.md, Pull request). */
export default function PullRequestView({
  reference,
  onBack,
  onOpenNote,
  onError,
}: Props) {
  const [tab, setTab] = useState<Tab>("overview");
  const { open: panelOpen, toggle: togglePanel } = useSidePanel();
  const [scope, setScope] = useState<Scope>("all");
  const [pr, setPr] = useState<PullRequest | null>(null);
  const [conversation, setConversation] = useState<Conversation | null>(null);
  const [drafts, setDrafts] = useState<ReviewDrafts | null>(null);
  const [files, setFiles] = useState<PullRequestFiles | null>(null);
  const [sinceFiles, setSinceFiles] = useState<PullRequestFiles | null>(null);
  const [sinceError, setSinceError] = useState<string | null>(null);
  const [checks, setChecks] = useState<PullRequestChecks | null>(null);
  const [repository, setRepository] = useState<RepositorySummary | null>(null);
  const [notes, setNotes] = useState<RepositoryNotes | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [reviewOpen, setReviewOpen] = useState(false);
  const [mergeOpen, setMergeOpen] = useState(false);
  /** The Overview's Explain… opens the dialog in Files Changed. */
  const [explainOnOpen, setExplainOnOpen] = useState(false);
  // Files Changed draws Explain and the panel toggle here, in the header,
  // since they act on the whole pull request (SPEC.md, section 14).
  const [explainSlot, setExplainSlot] = useState<HTMLSpanElement | null>(null);
  const latest = useRef(createLatest()).current;
  const conversationLatest = useRef(createLatest()).current;
  const draftsLatest = useRef(createLatest()).current;
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
  const loadDrafts = useCallback(() => {
    void draftsLatest.run(
      () => ipc.listReviewDrafts(reference),
      setDrafts,
      (e) => onError(errorMessage(e)),
    );
  }, [reference, draftsLatest, onError]);

  useEffect(() => {
    load(DETAIL_MAX_AGE);
    loadDrafts();
    const timer = setInterval(() => load(DETAIL_MAX_AGE), REFRESH_MS);
    return () => clearInterval(timer);
  }, [load, loadDrafts]);

  useEffect(
    () =>
      subscribe(
        onPullRequestChanged((e) => {
          if (e.reference === reference) load(DETAIL_MAX_AGE);
        }),
      ),
    [reference, load],
  );

  // The local checkout and the notes linked to it, for the side panel; a
  // Mac without a vault has no notes to show.
  useEffect(() => {
    let cancelled = false;
    void ipc
      .getPullRequestRepository(reference)
      .then((repo) => {
        if (cancelled) return;
        setRepository(repo);
        return ipc
          .getRepositoryNotes(repo.id)
          .then((n) => !cancelled && setNotes(n))
          .catch(() => setNotes(null));
      })
      .catch(() => setRepository(null));
    return () => {
      cancelled = true;
    };
  }, [reference]);

  // Files and checks follow the head commit: read again when it moved.
  const head = pr?.head_sha ?? null;
  useEffect(() => {
    if (tab !== "files" || !head) return;
    void filesLatest.run(
      () => ipc.listPullRequestFiles(reference, null),
      setFiles,
      (e) => onError(errorMessage(e)),
    );
  }, [tab, head, reference, filesLatest, onError]);
  const startCommit = drafts ? reviewStartCommit(drafts.drafts) : null;
  const sinceBase =
    scope === "review"
      ? (pr?.reviewed_sha ?? null)
      : scope === "drafts"
        ? startCommit
        : null;
  useEffect(() => {
    if (tab !== "files" || !head || !sinceBase) return;
    setSinceError(null);
    void sinceLatest.run(
      () => ipc.listPullRequestFiles(reference, sinceBase),
      (result) => {
        setSinceFiles(result);
        setSinceError(null);
      },
      (e) => {
        setSinceFiles(null);
        setSinceError(errorMessage(e));
      },
    );
  }, [tab, head, sinceBase, reference, sinceLatest]);
  useEffect(() => {
    if (tab !== "checks" || !head) return;
    void checksLatest.run(
      () => ipc.getPullRequestChecks(reference, DETAIL_MAX_AGE),
      setChecks,
      (e) => onError(errorMessage(e)),
    );
  }, [tab, head, reference, checksLatest, onError]);

  /** What a write to the provider leaves behind replaces what is on screen. */
  const applyOutcome = useCallback((outcome: WriteOutcome) => {
    setPr(outcome.pull_request);
    setConversation(outcome.conversation);
  }, []);

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
  const unsent = drafts ? unsentDrafts(drafts.drafts).length : 0;
  const showNewSinceDrafts = () => {
    setReviewOpen(false);
    setScope("drafts");
    setTab("files");
  };

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center gap-3 border-b bg-header px-3 pl-lead"
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
        {tab === "files" && <span ref={setExplainSlot} className="contents" />}
        {pr && (
          <button
            type="button"
            className="btn btn-primary"
            disabled={!pr.actions.review.allowed || !drafts}
            title={
              pr.actions.review.reason ??
              "Finish the review: a summary, a verdict, and your drafts"
            }
            onClick={() => setReviewOpen(true)}
          >
            Review Changes
            {unsent > 0 && (
              <span className="rounded-full bg-white/25 px-1.5 text-[11px] tabular">
                {unsent}
              </span>
            )}
          </button>
        )}
        {tab === "overview" && (
          <SidePanelButton open={panelOpen} onToggle={togglePanel} />
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
          sinceBase={sinceBase}
          startCommit={startCommit}
          onScope={setScope}
          conversation={conversation}
          drafts={drafts}
          onDrafts={setDrafts}
          onOutcome={applyOutcome}
          onError={onError}
          repositoryId={repository?.id ?? null}
          explainOnOpen={explainOnOpen}
          onExplainOpened={() => setExplainOnOpen(false)}
          explainSlot={explainSlot}
        />
      ) : tab === "checks" ? (
        <ChecksTab pr={pr} checks={checks} />
      ) : (
        <Overview
          pr={pr}
          conversation={conversation}
          drafts={drafts}
          repository={repository}
          notes={notes}
          onOpenNote={onOpenNote}
          onOutcome={applyOutcome}
          onReview={() => setReviewOpen(true)}
          onMerge={() => setMergeOpen(true)}
          onError={onError}
          onShowSinceReview={() => {
            setScope("review");
            setTab("files");
          }}
          onOpenExplanation={() => setTab("files")}
          onExplain={() => {
            setExplainOnOpen(true);
            setTab("files");
          }}
        />
      )}
      {reviewOpen && pr && drafts && (
        <ReviewDialog
          pr={pr}
          drafts={drafts}
          onClose={() => setReviewOpen(false)}
          onDrafts={setDrafts}
          onSubmitted={(outcome) => {
            applyOutcome(outcome);
            loadDrafts();
            setReviewOpen(false);
          }}
          onConflict={() => load(0)}
          onShowNewChanges={showNewSinceDrafts}
        />
      )}
      {mergeOpen && pr && (
        <MergeDialog
          pr={pr}
          unresolved={
            conversation
              ? unresolvedThreads(conversation.threads)
              : pr.counts.unresolved_threads
          }
          onClose={() => setMergeOpen(false)}
          onMerged={(outcome) => {
            applyOutcome(outcome);
            setMergeOpen(false);
            if (outcome.warning) onError(outcome.warning);
          }}
          onConflict={() => load(0)}
          onShowChanges={() => {
            setMergeOpen(false);
            setScope(pr.reviewed_sha ? "review" : "all");
            setTab("files");
          }}
        />
      )}
    </div>
  );
}

// --- Overview ----------------------------------------------------------------

function Overview({
  pr,
  conversation,
  drafts,
  repository,
  notes,
  onOpenNote,
  onOutcome,
  onReview,
  onMerge,
  onError,
  onShowSinceReview,
  onOpenExplanation,
  onExplain,
}: {
  pr: PullRequest;
  conversation: Conversation | null;
  drafts: ReviewDrafts | null;
  repository: RepositorySummary | null;
  notes: RepositoryNotes | null;
  onOpenNote: (noteId: string) => void;
  onOutcome: (outcome: WriteOutcome) => void;
  onReview: () => void;
  onMerge: () => void;
  onError: (message: string | null) => void;
  onShowSinceReview: () => void;
  onOpenExplanation: () => void;
  onExplain: () => void;
}) {
  const since = sinceReviewLabel(pr);
  const unresolved = conversation
    ? unresolvedThreads(conversation.threads)
    : pr.counts.unresolved_threads;
  const unsent = drafts ? unsentDrafts(drafts.drafts) : [];
  const startCommit = drafts ? reviewStartCommit(drafts.drafts) : null;
  const checkedOut =
    !!repository?.head &&
    repository.head.kind === "branch" &&
    repository.head.branch === pr.source_branch;
  const { open: panelOpen } = useSidePanel();
  // Merge waits for the checklist and for the provider (SPEC.md, Merging).
  const checklist = mergeChecklist(pr, unresolved);
  const canMerge = pr.actions.merge.allowed && checklist.complete;
  const mergeReason =
    pr.actions.merge.reason ??
    (checklist.complete
      ? null
      : "Merge waits until the list above is complete.");
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
          {repository && (
            <ExplanationLine
              repositoryId={repository.id}
              pr={pr}
              onOpen={onOpenExplanation}
              onExplain={onExplain}
            />
          )}
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
          {unsent.length > 0 && (
            <div className="flex items-center gap-2 text-[12.5px]">
              <span className="text-fg-2">
                {plural(unsent.length, "draft comment")} waiting
                {startCommit && !sameCommit(startCommit, pr.head_sha)
                  ? `, written on ${startCommit.slice(0, 10)}`
                  : ""}
                .
              </span>
              <button type="button" className="btn btn-sm" onClick={onReview}>
                Finish Review
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
        <ConversationSection
          conversation={conversation}
          pr={pr}
          onOutcome={onOutcome}
          onError={onError}
        />
      </section>
      {panelOpen && (
        <aside
          aria-label="Merge readiness and reviewers"
          className="flex w-[300px] shrink-0 flex-col overflow-y-auto border-l bg-panel"
        >
          <div className="flex flex-col gap-2 border-b px-[18px] py-3.5">
            <span className="section-label text-fg-2">Before merging</span>
            <Checklist items={checklist.items} />
            <button
              type="button"
              className="btn btn-primary self-start"
              disabled={!canMerge}
              title={
                mergeReason ??
                `Merge ${pr.source_branch} into ${pr.target_branch}`
              }
              onClick={onMerge}
            >
              Merge
            </button>
            {mergeReason && (
              <span className="text-[11.5px] text-muted">{mergeReason}</span>
            )}
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
          <div className="flex flex-col gap-1.5 border-b px-[18px] py-3.5">
            <span className="section-label text-fg-2">Size</span>
            <span className="text-[12.5px] text-fg-2">
              {sizeLabel(pr)}
              {pr.counts.commits !== null &&
                ` · ${plural(pr.counts.commits, "commit")}`}
            </span>
          </div>
          <div className="flex flex-col gap-1.5 border-b px-[18px] py-3.5">
            <span className="section-label text-fg-2">Local checkout</span>
            {repository ? (
              <div className="flex flex-col gap-0.5 text-[12.5px]">
                <span className="flex items-center gap-1.5">
                  <FolderIcon size={13} className="text-muted" />
                  <span className="font-medium">{repository.name}</span>
                  {checkedOut && (
                    <span className="pr-state" data-state="open">
                      Checked out
                    </span>
                  )}
                </span>
                <span
                  className="mono truncate text-[11.5px] text-muted"
                  title={repository.display_path}
                >
                  {repository.display_path}
                </span>
                {repository.head && (
                  <span className="text-fg-2">
                    On{" "}
                    <span className="mono">
                      {repository.head.kind === "branch"
                        ? repository.head.branch
                        : (repository.head.commit_id?.slice(0, 10) ??
                          "detached")}
                    </span>
                    {!checkedOut && (
                      <span className="text-muted">
                        ; the pull request's branch is{" "}
                        <span className="mono">{pr.source_branch}</span>
                      </span>
                    )}
                  </span>
                )}
              </div>
            ) : (
              <span className="text-[12.5px] text-muted">
                No local checkout tracks this repository.
              </span>
            )}
          </div>
          <div className="flex flex-col gap-1.5 px-[18px] py-3.5">
            <span className="section-label text-fg-2">Notes</span>
            {!notes || notes.notes.length === 0 ? (
              <span className="text-[12.5px] text-muted">
                No notes linked to the repository.
              </span>
            ) : (
              notes.notes.map((n) => (
                <button
                  key={n.id}
                  type="button"
                  className="flex items-center gap-1.5 self-start text-[12.5px] text-fg hover:underline"
                  onClick={() => onOpenNote(n.id)}
                >
                  <NoteIcon size={13} className="text-muted" />
                  {n.title}
                </button>
              ))
            )}
          </div>
        </aside>
      )}
    </div>
  );
}

// --- The conversation -------------------------------------------------------

/** The thread actions, shared by the Overview and the lines of a diff. */
type ThreadActions = {
  pr: PullRequest;
  onOutcome: (outcome: WriteOutcome) => void;
  onError: (message: string | null) => void;
};

function ConversationSection({
  conversation,
  pr,
  onOutcome,
  onError,
}: { conversation: Conversation | null } & ThreadActions) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const comment = () => {
    if (!text.trim() || busy) return;
    setBusy(true);
    void ipc
      .commentOnPullRequest({ reference: pr.reference, body: text })
      .then((outcome) => {
        setText("");
        onOutcome(outcome);
      })
      .catch((e) => onError(errorMessage(e)))
      .finally(() => setBusy(false));
  };
  const can = pr.actions.comment;
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
        conversation.threads.map((t) => (
          <ThreadCard
            key={t.id}
            thread={t}
            pr={pr}
            onOutcome={onOutcome}
            onError={onError}
          />
        ))
      )}
      <div className="flex flex-col gap-2 rounded-lg border bg-panel px-4 py-3">
        <textarea
          className="text-area"
          aria-label="Comment on the pull request"
          placeholder={
            can.allowed
              ? "Comment on the pull request… (Markdown; ⌘↩ to send)"
              : (can.reason ?? "")
          }
          disabled={!can.allowed || busy}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={submitOnEnter(comment)}
        />
        <div className="flex items-center gap-2">
          <span className="flex-1 text-[11.5px] text-muted">
            {can.allowed
              ? `Posted on ${PROVIDER_LABEL[pr.kind]} at once. Comments on lines wait for Finish Review.`
              : can.reason}
          </span>
          <button
            type="button"
            className="btn btn-sm btn-primary"
            disabled={!can.allowed || busy || !text.trim()}
            onClick={comment}
          >
            {busy ? "Posting…" : "Comment"}
          </button>
        </div>
      </div>
    </section>
  );
}

/**
 * One thread with Reply and, on a line, Resolve or Reopen; resolved ones are
 * folded (SPEC.md, Pull request: Overview). `compact` is the form shown
 * under a line of the diff, which already says where it is.
 */
function ThreadCard({
  thread,
  compact = false,
  pr,
  onOutcome,
  onError,
}: { thread: Thread; compact?: boolean } & ThreadActions) {
  const [replying, setReplying] = useState(false);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const where = thread.anchor ? anchorLabel(thread.anchor) : null;
  const run = (p: Promise<WriteOutcome>, then?: () => void) => {
    setBusy(true);
    void p
      .then((outcome) => {
        then?.();
        onOutcome(outcome);
      })
      .catch((e) => onError(errorMessage(e)))
      .finally(() => setBusy(false));
  };
  const reply = () => {
    if (!text.trim() || busy) return;
    run(
      ipc.replyToThread({
        reference: pr.reference,
        thread_id: thread.id,
        body: text,
      }),
      () => {
        setText("");
        setReplying(false);
      },
    );
  };
  const resolve = (resolved: boolean) =>
    run(
      ipc.resolveThread({
        reference: pr.reference,
        thread_id: thread.id,
        resolved,
      }),
    );
  const comments = thread.comments.map((c) => (
    <CommentView key={c.id} comment={c} />
  ));
  const canComment = pr.actions.comment;
  const canResolve = pr.actions.resolve;
  const actions = (
    <div className="flex items-center gap-2">
      {!replying && (
        <button
          type="button"
          className="btn btn-sm"
          disabled={!canComment.allowed || busy}
          title={canComment.reason ?? "Reply in this thread"}
          onClick={() => setReplying(true)}
        >
          Reply
        </button>
      )}
      {thread.anchor && (
        <button
          type="button"
          className="btn btn-sm"
          disabled={!canResolve.allowed || busy}
          title={
            canResolve.reason ??
            (thread.resolved
              ? "Reopen the thread"
              : "Mark the thread as resolved")
          }
          onClick={() => resolve(!thread.resolved)}
        >
          {thread.resolved ? "Reopen" : "Resolve"}
        </button>
      )}
      {busy && <span className="text-[11.5px] text-muted">Sending…</span>}
    </div>
  );
  const replyBox = replying && (
    <div className="flex flex-col gap-1.5">
      <textarea
        className="text-area"
        aria-label="Reply"
        placeholder="Reply… (Markdown; ⌘↩ to send)"
        // biome-ignore lint/a11y/noAutofocus: the user just asked to reply.
        autoFocus
        disabled={busy}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={submitOnEnter(reply)}
      />
      <div className="flex items-center gap-2">
        <button
          type="button"
          className="btn btn-sm btn-primary"
          disabled={busy || !text.trim()}
          onClick={reply}
        >
          Reply
        </button>
        <button
          type="button"
          className="btn btn-sm"
          disabled={busy}
          onClick={() => setReplying(false)}
        >
          Cancel
        </button>
      </div>
    </div>
  );
  const body = (
    <>
      {comments}
      {replyBox}
      {actions}
    </>
  );
  if (thread.resolved)
    return (
      <details
        className={`thread rounded-lg border bg-panel px-4 py-2 ${compact ? "text-[12.5px]" : ""}`}
        open={compact || undefined}
      >
        <summary className="cursor-default select-none text-[12.5px] text-muted">
          <StateIcon state="success" label="Resolved" />
          <span className="ml-1.5">Resolved</span>
          {where && !compact && (
            <span className="mono ml-2 text-[11.5px]">{where}</span>
          )}
          <span className="ml-2">
            · {plural(thread.comments.length, "comment")}
          </span>
        </summary>
        <div className="flex flex-col gap-3 pt-2 pb-1">{body}</div>
      </details>
    );
  return (
    <div
      className="thread flex flex-col gap-3 rounded-lg border bg-panel px-4 py-3"
      data-outdated={thread.outdated || undefined}
    >
      {where && !compact && (
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
      {body}
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

// --- Drafts on lines ----------------------------------------------------------

/** A draft under its line: the text, Edit, and Remove. */
function DraftCard({
  draft,
  onEdit,
  onDelete,
}: {
  draft: ReviewDraft;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const sent = draft.remote_id !== null;
  return (
    <div
      className="draft flex flex-col gap-1.5 rounded-lg border border-accent/40 bg-panel px-4 py-2.5"
      data-draft
    >
      <div className="flex items-center gap-2 text-[11.5px] text-fg-2">
        <span className="pr-state" data-state="open">
          {sent ? "Sent" : "Draft"}
        </span>
        <span className="text-muted">
          {sent
            ? "Posted; the rest of the review is still to send."
            : "Sent with Finish Review."}
        </span>
        <span className="flex-1" />
        {!sent && (
          <>
            <button type="button" className="btn btn-sm" onClick={onEdit}>
              Edit
            </button>
            <button
              type="button"
              className="btn btn-sm"
              aria-label={`Remove draft on ${anchorLabel(draft.anchor)}`}
              onClick={onDelete}
            >
              Remove
            </button>
          </>
        )}
      </div>
      <Markdown html={draft.html} />
    </div>
  );
}

/** The box a line comment is written in; saved as a draft on the Mac. */
function Composer({
  target,
  initial,
  busy,
  onSave,
  onCancel,
}: {
  target: LineTarget;
  initial: string;
  busy: boolean;
  onSave: (text: string) => void;
  onCancel: () => void;
}) {
  const [text, setText] = useState(initial);
  const save = () => text.trim() && !busy && onSave(text);
  const where =
    target.start_line !== null && target.start_line < target.line
      ? `lines ${target.start_line}–${target.line}`
      : `line ${target.line}`;
  return (
    <div className="flex flex-col gap-1.5 rounded-lg border border-accent/40 bg-panel px-4 py-2.5">
      <span className="text-[11.5px] text-fg-2">
        Comment on {where} ({target.side === "old" ? "old" : "new"} side). Kept
        on this Mac until Finish Review.
      </span>
      <textarea
        className="text-area"
        aria-label={`Comment on ${where}`}
        placeholder="Markdown; ⌘↩ to save"
        // biome-ignore lint/a11y/noAutofocus: the user just asked to comment here.
        autoFocus
        disabled={busy}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            onCancel();
          } else submitOnEnter(save)(e);
        }}
      />
      <div className="flex items-center gap-2">
        <button
          type="button"
          className="btn btn-sm btn-primary"
          disabled={busy || !text.trim()}
          onClick={save}
        >
          Save Draft
        </button>
        <button
          type="button"
          className="btn btn-sm"
          disabled={busy}
          onClick={onCancel}
        >
          Cancel
        </button>
      </div>
    </div>
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

type ComposerState = {
  target: LineTarget;
  draftId: string | null;
  initial: string;
};

function FilesTab({
  pr,
  files,
  sinceFiles,
  sinceError,
  scope,
  sinceBase,
  startCommit,
  onScope,
  conversation,
  drafts,
  onDrafts,
  onOutcome,
  onError,
  repositoryId,
  explainOnOpen,
  onExplainOpened,
  explainSlot,
}: {
  pr: PullRequest;
  files: PullRequestFiles | null;
  sinceFiles: PullRequestFiles | null;
  sinceError: string | null;
  scope: Scope;
  /** The commit a "since" scope compares from, or null for all changes. */
  sinceBase: string | null;
  /** The commit the drafts were written on, when there are drafts. */
  startCommit: string | null;
  onScope: (scope: Scope) => void;
  conversation: Conversation | null;
  drafts: ReviewDrafts | null;
  onDrafts: (drafts: ReviewDrafts) => void;
  onOutcome: (outcome: WriteOutcome) => void;
  onError: (message: string | null) => void;
  /** The local checkout, which an explanation needs; null without one. */
  repositoryId: string | null;
  /** The Overview's Explain… asked for the dialog. */
  explainOnOpen: boolean;
  onExplainOpened: () => void;
  /** Where in the view's header Explain and the panel toggle go. */
  explainSlot: HTMLElement | null;
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
  const [composer, setComposer] = useState<ComposerState | null>(null);
  const [draftBusy, setDraftBusy] = useState(false);
  const diffLatest = useRef(createLatest()).current;

  const since = sinceBase !== null;
  const canSinceReview = !!pr.reviewed_sha && pr.reviewed_sha !== pr.head_sha;
  const canSinceDrafts =
    startCommit !== null && !sameCommit(startCommit, pr.head_sha);
  const list = since ? sinceFiles : files;
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
  const byPath = useMemo(
    () => [
      ...groupByDir(sections.main).flatMap((g) => g.files),
      ...groupByDir(sections.viewed).flatMap((g) => g.files),
      ...groupByDir(sections.generated).flatMap((g) => g.files),
    ],
    [sections],
  );

  // The explanation (SPEC.md, section 14, Pull requests): the same panel,
  // notes, and reading order as a branch's, beside the review.
  const subject = useMemo(
    () =>
      repositoryId
        ? { kind: "pull_request" as const, reference: pr.reference }
        : null,
    [repositoryId, pr.reference],
  );
  const selectRef = useRef<(path: string) => void>(() => {});
  const explained = useExplainedPatch({
    repositoryId: repositoryId ?? "",
    view: "pull_request",
    subject,
    files: sections.shown,
    selectedPath,
    onSelectFile: (path) => selectRef.current(path),
    onNotice: (m) => onError(m),
    version: pr.head_sha,
    review: true,
  });
  // In reading order, viewed and generated files keep their place, so the
  // numbers hold; Path brings back the tree and its folds.
  const reading = explained.hasExplanation && explained.order === "reading";
  const ordered = reading ? explained.ordered : byPath;
  useEffect(() => {
    if (!explainOnOpen || !subject) return;
    explained.openDialog();
    onExplainOpened();
  }, [explainOnOpen, subject, explained, onExplainOpened]);

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
  // biome-ignore lint/correctness/useExhaustiveDependencies: `list` stands for the head and scope the patch belongs to.
  useEffect(() => {
    setComposer(null);
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
          since: sinceBase,
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
    sinceBase,
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
  selectRef.current = (path) => {
    const f = list?.files.find((x) => x.path === path);
    if (f) select(f);
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

  // Threads and drafts under the lines of the selected file, and the box a
  // new comment is written in (SPEC.md, Reviewing).
  const notes = useMemo(
    () =>
      selected
        ? lineNotes(
            selected.path,
            conversation?.threads ?? [],
            drafts?.drafts ?? [],
          )
        : new Map(),
    [selected, conversation, drafts],
  );
  const runDrafts = (p: Promise<ReviewDrafts>, then?: () => void) => {
    setDraftBusy(true);
    void p
      .then((result) => {
        onDrafts(result);
        then?.();
      })
      .catch((e) => onError(errorMessage(e)))
      .finally(() => setDraftBusy(false));
  };
  const saveDraft = (text: string) => {
    if (!composer || !selected || !diff) return;
    runDrafts(
      ipc.saveReviewDraft({
        reference: pr.reference,
        id: composer.draftId,
        anchor: {
          path: selected.path,
          side: composer.target.side,
          line: composer.target.line,
          start_line: composer.target.start_line,
          commit: diff.head_sha,
        },
        body: text,
      }),
      () => setComposer(null),
    );
  };
  const canReview = pr.actions.review;
  const startComment = (target: LineTarget) => {
    if (since && target.side === "old") {
      onError(
        "In changes since a commit, the old side is that commit, not the target branch: comment on the new side.",
      );
      return;
    }
    setComposer({ target, draftId: null, initial: "" });
  };
  const reviewAnnotate =
    diff?.diff.content.kind === "text"
      ? {
          render: (line: DiffLine) => {
            const keys: Array<[string, "old" | "new", number]> = [];
            if (line.new_no !== null)
              keys.push([lineKey("new", line.new_no), "new", line.new_no]);
            if (line.old_no !== null)
              keys.push([lineKey("old", line.old_no), "old", line.old_no]);
            const threads: Thread[] = [];
            const lineDrafts: ReviewDraft[] = [];
            for (const [key] of keys) {
              const n = notes.get(key);
              if (n) {
                threads.push(...n.threads);
                lineDrafts.push(...n.drafts);
              }
            }
            const here =
              composer &&
              keys.some(
                ([, side, no]) =>
                  side === composer.target.side && no === composer.target.line,
              );
            if (threads.length === 0 && lineDrafts.length === 0 && !here)
              return null;
            return (
              <>
                {threads.map((t) => (
                  <ThreadCard
                    key={t.id}
                    thread={t}
                    compact
                    pr={pr}
                    onOutcome={onOutcome}
                    onError={onError}
                  />
                ))}
                {lineDrafts
                  .filter((d) => d.id !== composer?.draftId)
                  .map((d) => (
                    <DraftCard
                      key={d.id}
                      draft={d}
                      onEdit={() =>
                        setComposer({
                          target: {
                            side: d.anchor.side,
                            line: d.anchor.line ?? 0,
                            start_line: d.anchor.start_line,
                          },
                          draftId: d.id,
                          initial: d.body,
                        })
                      }
                      onDelete={() =>
                        runDrafts(ipc.deleteReviewDraft(pr.reference, d.id))
                      }
                    />
                  ))}
                {here && composer && (
                  <Composer
                    key={composer.draftId ?? "new"}
                    target={composer.target}
                    initial={composer.initial}
                    busy={draftBusy}
                    onSave={saveDraft}
                    onCancel={() => setComposer(null)}
                  />
                )}
              </>
            );
          },
          onComment: canReview.allowed ? startComment : undefined,
          note: canReview.reason ?? undefined,
        }
      : undefined;
  const annotate = composeAnnotate(reviewAnnotate, explained.annotate);

  const empty = !list
    ? since && sinceError
      ? sinceError
      : "Reading the files…"
    : list.files.length === 0
      ? since
        ? "Nothing changed since then."
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
        aria-selected={scope === "review"}
        disabled={!canSinceReview}
        title={
          canSinceReview
            ? "Only what changed since the commit you last reviewed, from local Git"
            : pr.reviewed_sha
              ? "Nothing new since your review"
              : "You have not reviewed this pull request yet"
        }
        onClick={() => onScope("review")}
      >
        Since your review
      </button>
      {canSinceDrafts && (
        <button
          type="button"
          role="tab"
          aria-selected={scope === "drafts"}
          title="Only what changed since the commit your drafts were written on, from local Git"
          onClick={() => onScope("drafts")}
        >
          Since your drafts
        </button>
      )}
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
            {explained.hasExplanation && (
              <div className="flex items-center gap-2">
                <OrderSwitch
                  order={explained.order}
                  setOrder={explained.setOrder}
                />
                <span className="ml-auto text-[11.5px] tabular text-muted">
                  {sections.all.filter((f) => isViewed(marks, f)).length} of{" "}
                  {sections.all.length} viewed
                </span>
              </div>
            )}
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
            {reading && (
              <ReadingList
                files={ordered}
                steps={explained.steps}
                fileNotes={explained.fileNotes}
                selectedPath={selectedPath}
                marks={marks}
                counts={counts}
                onSelect={select}
                onMark={mark}
              />
            )}
            {!reading && (
              <FileGroups
                files={sections.main}
                selectedPath={selectedPath}
                marks={marks}
                counts={counts}
                fileNotes={explained.fileNotes}
                onSelect={select}
                onMark={mark}
              />
            )}
            {!reading && sections.viewed.length > 0 && (
              <details className="pr-fold">
                <summary>
                  {plural(sections.viewed.length, "viewed file")}
                </summary>
                <FileGroups
                  files={sections.viewed}
                  selectedPath={selectedPath}
                  marks={marks}
                  counts={counts}
                  fileNotes={explained.fileNotes}
                  onSelect={select}
                  onMark={mark}
                />
              </details>
            )}
            {!reading && sections.generated.length > 0 && (
              <details className="pr-fold">
                <summary>
                  {plural(sections.generated.length, "generated file")}
                </summary>
                <FileGroups
                  files={sections.generated}
                  selectedPath={selectedPath}
                  marks={marks}
                  counts={counts}
                  fileNotes={explained.fileNotes}
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
          annotate={annotate}
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
            <>
              {!showFiles && (
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
              )}
              {explained.toolbar}
            </>
          }
          meta={meta}
          empty={empty}
          onOpenInEditor={openInEditor}
        />
      </div>
      {explained.panel}
      {explained.dialog}
      {explainSlot && explained.header
        ? createPortal(explained.header, explainSlot)
        : null}
    </div>
  );
}

function FileGroups({
  files,
  selectedPath,
  marks,
  counts,
  fileNotes,
  onSelect,
  onMark,
}: {
  files: ChangedFile[];
  selectedPath: string | null;
  marks: ViewedMarks;
  counts: Map<string, { total: number; open: number }>;
  fileNotes: Map<string, FileNoteCount>;
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
          notes={fileNotes.get(f.path)}
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

/**
 * The Overview's one line about the explanation (SPEC.md, section 14, Pull
 * requests): whether there is one, how old, how much moved, and a way to it.
 */
function ExplanationLine({
  repositoryId,
  pr,
  onOpen,
  onExplain,
}: {
  repositoryId: string;
  pr: PullRequest;
  onOpen: () => void;
  onExplain: () => void;
}) {
  const subject = useMemo(
    () => ({ kind: "pull_request" as const, reference: pr.reference }),
    [pr.reference],
  );
  const { current, placement } = useExplanation(
    repositoryId,
    subject,
    pr.head_sha,
  );
  const blocked = useExplainBlocked(repositoryId, subject, pr.head_sha);
  const outOfDate = placement?.notes.filter((n) => n.out_of_date).length ?? 0;
  let text: string;
  if (!current) text = "Not explained";
  else if (current.state === "working") text = "Explaining…";
  else if (current.state === "ready")
    text = `Explained · ${depthLabel(current.depth)} · ${relativeTime(current.created_at)}${
      sameSubject(current.subject, subject)
        ? ""
        : ` · from ${subjectLabel(current.subject)}`
    }`;
  else
    text =
      current.state === "cancelled"
        ? "Explanation cancelled"
        : "The explanation failed";
  return (
    <div className="flex items-center gap-2 text-[12.5px]">
      <BulbIcon size={13} className="shrink-0 text-muted" />
      <span className={current ? "text-fg-2" : "text-muted"}>{text}</span>
      {placement?.moved && outOfDate > 0 && (
        <span className="pr-state">
          {plural(outOfDate, "note")} out of date
        </span>
      )}
      {current ? (
        <button type="button" className="btn btn-sm" onClick={onOpen}>
          Open in Files Changed
        </button>
      ) : (
        <button
          type="button"
          className="btn btn-sm"
          title={blocked ?? "Explain this pull request (E in Files Changed)"}
          disabled={!!blocked}
          onClick={onExplain}
        >
          Explain…
        </button>
      )}
    </div>
  );
}

/**
 * The files in the explanation's reading order, viewed ones in place. Each
 * toured file has its tour step, so a filter or Since your review leaves
 * the numbers alone; a file the tour does not list has none.
 */
function ReadingList({
  files,
  steps,
  fileNotes,
  selectedPath,
  marks,
  counts,
  onSelect,
  onMark,
}: {
  files: ChangedFile[];
  steps: Map<string, number>;
  fileNotes: Map<string, FileNoteCount>;
  selectedPath: string | null;
  marks: ViewedMarks;
  counts: Map<string, { total: number; open: number }>;
  onSelect: (f: ChangedFile) => void;
  onMark: (f: ChangedFile, viewed: boolean) => void;
}) {
  return files.map((f, i) => (
    <div key={f.path}>
      {startsNotCovered(files, i, steps) && <NotCoveredHeading />}
      <FileRow
        file={f}
        nested={false}
        step={steps.get(f.path) ?? null}
        notes={fileNotes.get(f.path)}
        selected={f.path === selectedPath}
        viewed={isViewed(marks, f)}
        threads={counts.get(f.path) ?? null}
        onSelect={() => onSelect(f)}
        onMark={(viewed) => onMark(f, viewed)}
      />
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
  step,
  notes,
}: {
  file: ChangedFile;
  nested: boolean;
  /** Its step in the explanation's tour; null for a file the tour does not
   * list, which keeps the column so the rows line up; absent in Path. */
  step?: number | null;
  notes?: FileNoteCount;
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
        {step !== undefined && (
          <span className="w-[16px] shrink-0 text-right text-[11px] tabular text-muted">
            {step ?? ""}
          </span>
        )}
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
        <FileNotes count={notes} />
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

// --- Finish Review -------------------------------------------------------------

/**
 * **Finish Review** (SPEC.md, Reviewing): the summary, the verdict, the
 * drafts to send, and the commit being reviewed. New commits since the
 * review started turn approving off and offer the new changes; a
 * submission cut off midway offers to send what is left.
 */
function ReviewDialog({
  pr,
  drafts,
  onClose,
  onDrafts,
  onSubmitted,
  onConflict,
  onShowNewChanges,
}: {
  pr: PullRequest;
  drafts: ReviewDrafts;
  onClose: () => void;
  onDrafts: (drafts: ReviewDrafts) => void;
  onSubmitted: (outcome: WriteOutcome) => void;
  /** The head moved while the dialog was open: read the pull request again. */
  onConflict: () => void;
  onShowNewChanges: () => void;
}) {
  const pending = drafts.pending;
  const [body, setBody] = useState(pending?.body ?? "");
  const [verdict, setVerdict] = useState<ReviewVerdict>(
    pending?.verdict ?? "comment",
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const unsent = unsentDrafts(drafts.drafts);
  const sent = drafts.drafts.length - unsent.length;
  const behind = draftsBehind(drafts.drafts, pr.head_sha);
  const start = reviewStartCommit(drafts.drafts);
  const approve = pr.actions.approve;
  const approveReason = behind
    ? "New commits arrived since you started this review: look at them first."
    : approve.reason;
  const incomplete = reviewIncomplete(body, drafts.drafts, verdict);
  const blocked = behind
    ? "Look at the new commits, or continue on the current one, before sending."
    : incomplete;

  const submit = () => {
    if (busy || blocked) return;
    setBusy(true);
    setError(null);
    void ipc
      .submitReview({
        reference: pr.reference,
        body,
        verdict,
        expected_head_sha: pr.head_sha,
      })
      .then(onSubmitted)
      .catch((e) => {
        setError(errorMessage(e));
        if (isAppError(e) && e.code === "CONFLICT") onConflict();
        // What was sent before a timeout is marked; show it.
        void ipc
          .listReviewDrafts(pr.reference)
          .then(onDrafts)
          .catch(() => {});
      })
      .finally(() => setBusy(false));
  };
  const moveOn = () => {
    setBusy(true);
    void ipc
      .moveReviewDrafts(pr.reference, pr.head_sha)
      .then(onDrafts)
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setBusy(false));
  };
  const removeDraft = (id: string) => {
    setBusy(true);
    void ipc
      .deleteReviewDraft(pr.reference, id)
      .then(onDrafts)
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setBusy(false));
  };
  const submitLabel =
    pending && sent > 0
      ? "Send the Rest"
      : verdict === "approve"
        ? "Approve"
        : verdict === "request_changes"
          ? "Request Changes"
          : "Submit Review";

  return (
    <Dialog
      title="Finish Review"
      width={600}
      onClose={onClose}
      footer={
        <>
          {error && (
            <span className="flex-1 text-[12px] text-del" role="alert">
              {error}
            </span>
          )}
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || !!blocked}
            title={blocked ?? undefined}
            onClick={submit}
          >
            {busy ? "Sending…" : submitLabel}
          </button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5 text-[13px]">
        <span className="text-[12.5px] text-fg-2">
          Reviewing{" "}
          <span className="mono" title={pr.head_sha}>
            {pr.head_sha.slice(0, 10)}
          </span>{" "}
          of <span className="mono">{pr.source_branch}</span>, sent to{" "}
          {PROVIDER_LABEL[pr.kind]}
          {pr.kind === "bitbucket_cloud" && unsent.length > 0
            ? " as one request per comment"
            : ""}
          .
        </span>
        {pending && sent > 0 && (
          <div className="rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
            This review was cut off midway: {sent} of{" "}
            {plural(drafts.drafts.length, "comment")} reached{" "}
            {PROVIDER_LABEL[pr.kind]}
            {pending.summary_sent ? ", and the summary" : ""}. Sending again
            posts only what is left.
          </div>
        )}
        {behind && (
          <div className="flex flex-col gap-2 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
            <span>
              New commits arrived since you started this review
              {start ? ` on ${start.slice(0, 10)}` : ""}. Nothing is sent,
              approving is off, and your drafts are kept until you have looked
              at the new changes.
            </span>
            <div className="flex gap-2">
              <button
                type="button"
                className="btn btn-sm"
                disabled={busy}
                onClick={onShowNewChanges}
              >
                Show New Changes
              </button>
              <button
                type="button"
                className="btn btn-sm"
                disabled={busy}
                title="Your drafts then go on the current commit; a draft on a line that changed may be refused."
                onClick={moveOn}
              >
                Continue on {pr.head_sha.slice(0, 10)}
              </button>
            </div>
          </div>
        )}
        <label className="flex flex-col gap-1">
          <span className="text-[12px] text-fg-2">Summary</span>
          <textarea
            className="text-area min-h-[90px]"
            placeholder="What you think of the change… (Markdown; ⌘↩ to send)"
            value={body}
            readOnly={pending?.summary_sent}
            disabled={busy}
            onChange={(e) => setBody(e.target.value)}
            onKeyDown={submitOnEnter(submit)}
          />
          {pending?.summary_sent && (
            <span className="text-[11.5px] text-muted">
              Already posted; only the verdict is left.
            </span>
          )}
        </label>
        <fieldset className="m-0 flex flex-col gap-1.5 border-0 p-0">
          <legend className="mb-1 p-0 text-[12px] text-fg-2">Verdict</legend>
          {VERDICTS.map((v) => {
            const off = v === "approve" && (behind || !approve.allowed);
            return (
              <label
                key={v}
                className={`flex items-center gap-2 ${off ? "text-muted" : ""}`}
                title={
                  v === "approve" ? (approveReason ?? undefined) : undefined
                }
              >
                <input
                  type="radio"
                  name="verdict"
                  value={v}
                  checked={verdict === v}
                  disabled={off || busy}
                  onChange={() => setVerdict(v)}
                />
                {VERDICT_LABEL[v]}
                {v === "approve" && off && approveReason && (
                  <span className="text-[11.5px]">— {approveReason}</span>
                )}
              </label>
            );
          })}
        </fieldset>
        <div className="flex flex-col gap-1.5">
          <span className="text-[12px] text-fg-2">
            {drafts.drafts.length === 0
              ? "No comments on lines. Add them from a line's + in Files Changed."
              : `${plural(drafts.drafts.length, "comment on a line", "comments on lines")} to send`}
          </span>
          {drafts.drafts.map((d) => (
            <div
              key={d.id}
              className="flex flex-col gap-1 rounded-md border bg-panel px-3 py-2"
            >
              <div className="flex items-center gap-2 text-[11.5px] text-fg-2">
                <span className="mono">{anchorLabel(d.anchor)}</span>
                {d.remote_id !== null && (
                  <span className="pr-state" data-state="open">
                    Sent
                  </span>
                )}
                <span className="flex-1" />
                {d.remote_id === null && (
                  <button
                    type="button"
                    className="btn btn-sm"
                    disabled={busy}
                    aria-label={`Remove draft on ${anchorLabel(d.anchor)}`}
                    onClick={() => removeDraft(d.id)}
                  >
                    Remove
                  </button>
                )}
              </div>
              <Markdown html={d.html} className="text-[12.5px]" />
            </div>
          ))}
        </div>
      </div>
    </Dialog>
  );
}

// --- Merging (SPEC.md, Merging) ----------------------------------------------

/** What the pull request needs before merging, with an icon per line. */
function Checklist({ items }: { items: ChecklistItem[] }) {
  return (
    <ul className="m-0 flex list-none flex-col gap-1.5 p-0 text-[12.5px]">
      {items.map((item) => (
        <li key={item.key} className="flex items-center gap-2">
          <StateIcon
            state={item.state}
            label={
              item.state === "success"
                ? "Done"
                : item.blocking
                  ? "Blocks merging"
                  : "Not required"
            }
          />
          {item.label}
        </li>
      ))}
    </ul>
  );
}

function MergeDialog({
  pr,
  unresolved,
  onClose,
  onMerged,
  onConflict,
  onShowChanges,
}: {
  pr: PullRequest;
  unresolved: number | null;
  onClose: () => void;
  onMerged: (outcome: MergeOutcome) => void;
  /** The head moved while the dialog was open: read the pull request again. */
  onConflict: () => void;
  onShowChanges: () => void;
}) {
  const [options, setOptions] = useState<MergeOptions | null>(null);
  const [method, setMethod] = useState<MergeMethod | null>(null);
  const [title, setTitle] = useState("");
  const [message, setMessage] = useState("");
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The pull request as it was when the dialog opened: the message is
  // prefilled from it once, and a head that differs means new commits.
  const opened = useRef(pr).current;
  const moved = !sameCommit(opened.head_sha, pr.head_sha);
  const checklist = mergeChecklist(pr, unresolved);

  useEffect(() => {
    let cancelled = false;
    void ipc
      .getMergeOptions(opened.reference)
      .then((o) => {
        if (cancelled) return;
        setOptions(o);
        setMethod(o.default_method);
        setDeleteBranch(o.delete_branch);
        const text = defaultMergeMessage(opened, o.default_method);
        setTitle(text.title);
        setMessage(text.message);
      })
      .catch((e) => !cancelled && setError(errorMessage(e)));
    return () => {
      cancelled = true;
    };
  }, [opened]);

  const choose = (m: MergeMethod) => {
    setMethod(m);
    const text = defaultMergeMessage(pr, m);
    setTitle(text.title);
    setMessage(text.message);
  };
  const takesMessage = method === "merge_commit" || method === "squash";
  const blocked = moved
    ? "New commits arrived; look at them before merging."
    : !pr.actions.merge.allowed
      ? pr.actions.merge.reason
      : !checklist.complete
        ? "The checklist is not complete."
        : !options || !method
          ? "Reading what the repository allows…"
          : null;

  const submit = () => {
    if (busy || blocked || !method) return;
    setBusy(true);
    setError(null);
    void ipc
      .mergePullRequest({
        reference: pr.reference,
        method,
        commit_title: takesMessage ? title : "",
        commit_message: takesMessage ? message : "",
        delete_branch: deleteBranch && !!options?.can_delete_branch,
        expected_head_sha: pr.head_sha,
      })
      .then(onMerged)
      .catch((e) => {
        setError(errorMessage(e));
        if (isAppError(e) && e.code === "CONFLICT") onConflict();
      })
      .finally(() => setBusy(false));
  };

  return (
    <Dialog
      title="Merge Pull Request"
      width={600}
      onClose={onClose}
      footer={
        <>
          {error && (
            <span className="flex-1 text-[12px] text-del" role="alert">
              {error}
            </span>
          )}
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || !!blocked}
            title={blocked ?? "Merging cannot be undone from Brainiac"}
            onClick={submit}
          >
            {busy ? "Merging…" : `Merge ${pr.head_sha.slice(0, 10)}`}
          </button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5 text-[13px]">
        <span className="text-[12.5px] text-fg-2">
          Merging{" "}
          <span className="mono" title={pr.head_sha}>
            {pr.head_sha.slice(0, 10)}
          </span>{" "}
          of <span className="mono">{pr.source_branch}</span> into{" "}
          <span className="mono">{pr.target_branch}</span> on{" "}
          {PROVIDER_LABEL[pr.kind]}. {PROVIDER_LABEL[pr.kind]} does the merge;
          the local checkout changes when it is next fetched.
        </span>
        {moved && (
          <div className="flex flex-col gap-2 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
            <span>
              New commits arrived since you opened this:{" "}
              <span className="mono">{opened.head_sha.slice(0, 10)}</span> is
              now <span className="mono">{pr.head_sha.slice(0, 10)}</span>.
              Nothing was merged.
            </span>
            <button
              type="button"
              className="btn btn-sm self-start"
              disabled={busy}
              onClick={onShowChanges}
            >
              Show New Changes
            </button>
          </div>
        )}
        <Checklist items={checklist.items} />
        <fieldset className="m-0 flex flex-col gap-1.5 border-0 p-0">
          <legend className="mb-1 p-0 text-[12px] text-fg-2">Method</legend>
          {!options ? (
            <span className="text-[12.5px] text-muted">
              Reading what the repository allows…
            </span>
          ) : (
            options.methods.map((m) => (
              <label key={m} className="flex items-center gap-2">
                <input
                  type="radio"
                  name="merge-method"
                  value={m}
                  checked={method === m}
                  disabled={busy}
                  onChange={() => choose(m)}
                />
                {MERGE_METHOD_LABEL[m]}
              </label>
            ))
          )}
        </fieldset>
        {takesMessage && (
          <div className="flex flex-col gap-2">
            {pr.kind === "github" && (
              <label className="flex flex-col gap-1">
                <span className="text-[12px] text-fg-2">Commit title</span>
                <input
                  className="text-input"
                  value={title}
                  disabled={busy}
                  onChange={(e) => setTitle(e.target.value)}
                />
              </label>
            )}
            <label className="flex flex-col gap-1">
              <span className="text-[12px] text-fg-2">Commit message</span>
              <textarea
                className="text-area min-h-[70px]"
                value={message}
                disabled={busy}
                onChange={(e) => setMessage(e.target.value)}
              />
            </label>
          </div>
        )}
        {options?.can_delete_branch && (
          <label
            className="flex items-center gap-2"
            title={
              options.deletes_branch_itself
                ? `${PROVIDER_LABEL[pr.kind]} deletes the branch itself when merging; the repository says so.`
                : undefined
            }
          >
            <input
              type="checkbox"
              checked={deleteBranch}
              disabled={busy || options.deletes_branch_itself}
              onChange={(e) => setDeleteBranch(e.target.checked)}
            />
            Delete the branch <span className="mono">{pr.source_branch}</span>{" "}
            on {PROVIDER_LABEL[pr.kind]}
            {options.deletes_branch_itself && (
              <span className="text-[11.5px] text-muted">
                (the repository does this itself)
              </span>
            )}
          </label>
        )}
      </div>
    </Dialog>
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
