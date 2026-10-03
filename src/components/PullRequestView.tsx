import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useRef, useState } from "react";
import { relativeTime } from "../lib/format";
import {
  errorMessage,
  ipc,
  onPullRequestChanged,
  type PullRequest,
  type PullRequestChecks,
  type PullRequestFiles,
  subscribe,
} from "../lib/ipc";
import { useKeys } from "../lib/keys";
import {
  CHECK_LABEL,
  checksLabel,
  PROVIDER_LABEL,
  REVIEW_LABEL,
  STATE_LABEL,
  sizeLabel,
} from "../lib/pullRequests";
import { plural } from "../lib/repo";
import { createLatest } from "../lib/stale";
import { Avatar } from "./HistoryTab";
import { ChevronLeft, ExternalIcon, RefreshIcon } from "./icons";
import { StateIcon } from "./PullRequestsTab";

type Tab = "overview" | "files" | "checks";

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
  const [pr, setPr] = useState<PullRequest | null>(null);
  const [files, setFiles] = useState<PullRequestFiles | null>(null);
  const [checks, setChecks] = useState<PullRequestChecks | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const latest = useRef(createLatest()).current;
  const filesLatest = useRef(createLatest()).current;
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
    },
    [reference, latest, onError],
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
      () => ipc.listPullRequestFiles(reference),
      setFiles,
      (e) => onError(errorMessage(e)),
    );
  }, [tab, head, reference, filesLatest, onError]);
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
        <FilesTab pr={pr} files={files} />
      ) : tab === "checks" ? (
        <ChecksTab pr={pr} checks={checks} />
      ) : (
        <Overview pr={pr} />
      )}
    </div>
  );
}

function Overview({ pr }: { pr: PullRequest }) {
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
        </div>
        {pr.description.trim() ? (
          <pre className="selectable m-0 whitespace-pre-wrap rounded-lg border bg-panel px-4 py-3 font-[inherit] text-[13px] leading-relaxed">
            {pr.description}
          </pre>
        ) : (
          <span className="text-[12.5px] text-muted">No description.</span>
        )}
        <div className="text-[12px] text-muted">
          The conversation and review come in a later step; comments are on{" "}
          {PROVIDER_LABEL[pr.kind]} for now.
        </div>
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
                  pr.counts.unresolved_threads === 0
                    ? "success"
                    : pr.counts.unresolved_threads === null
                      ? "neutral"
                      : "pending"
                }
                label="Threads"
              />
              {pr.counts.unresolved_threads === null
                ? "Threads not read yet"
                : pr.counts.unresolved_threads === 0
                  ? "No unresolved threads"
                  : plural(pr.counts.unresolved_threads, "unresolved thread")}
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

function FilesTab({
  pr,
  files,
}: {
  pr: PullRequest;
  files: PullRequestFiles | null;
}) {
  if (!files) return <div className="p-6 text-muted">Reading the files…</div>;
  const stale = files.head_sha !== pr.head_sha;
  return (
    <section
      aria-label="Files changed"
      className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-6 py-5"
    >
      <span className="text-[12px] text-muted">
        {plural(files.files.length, "file")} at{" "}
        <span className="mono">{files.head_sha.slice(0, 10)}</span>
        {stale && " (a newer commit arrived; reading again)"}. Diffs come in a
        later step.
      </span>
      <table className="pr-table">
        <thead>
          <tr>
            <th>File</th>
            <th>Change</th>
            <th>Lines</th>
          </tr>
        </thead>
        <tbody>
          {files.files.map((f) => (
            <tr key={f.path} tabIndex={-1} style={{ cursor: "default" }}>
              <td className="mono">
                {f.old_path && (
                  <>
                    <span className="text-muted">{f.old_path}</span>{" "}
                    <span className="text-muted">→</span>{" "}
                  </>
                )}
                {f.path}
              </td>
              <td className="text-fg-2">
                {f.status === "added"
                  ? "Added"
                  : f.status === "removed"
                    ? "Removed"
                    : f.status === "renamed"
                      ? "Renamed"
                      : f.status === "modified"
                        ? "Modified"
                        : "Changed"}
                {f.binary && " · binary"}
              </td>
              <td className="tabular whitespace-nowrap">
                <span className="text-add">+{f.additions}</span>{" "}
                <span className="text-del">−{f.deletions}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

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
