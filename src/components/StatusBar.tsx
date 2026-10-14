import { type FooterHint, useFooterHints } from "../lib/footer";
import { relativeTime, shortPath } from "../lib/format";
import {
  jobElapsed,
  jobVerb,
  stepOf,
  useHostJobsContext,
  useNow,
} from "../lib/hostJobs";
import type { AppSnapshot, RepositorySummary } from "../lib/ipc";
import { totals } from "../lib/workspace";
import { Spinner } from "./HostJobView";
import type { View } from "./Sidebar";

/** Shortcuts that work in each view (SPEC.md, Keyboard defaults). */
function hints(view: View): Array<[string, string]> {
  // A pull request publishes its own, which follow the tab and the focus.
  if (view.kind === "pullRequest") return [];
  if (view.kind === "today")
    return [
      ["J K", "rows"],
      ["Space", "done"],
      ["↵", "open"],
      ["⌥⌘B", "panel"],
      ["⇧⌘N", "new task"],
    ];
  if (view.kind === "tasks")
    return [
      ["J K", "rows"],
      ["Space", "done"],
      ["↵", "open"],
      ["⇧⌘N", "new task"],
    ];
  if (view.kind === "notes")
    return [
      ["⌘N", "new"],
      ["⌘S", "save"],
      ["⇧⌘E", "source"],
      ["⌥⌘B", "context"],
      ["/", "filter"],
    ];
  if (view.kind === "databases")
    return [
      ["⌘⏎", "run"],
      ["⇧⌘⏎", "run all"],
      ["⌘.", "cancel"],
      ["⌘S", "save query"],
      ["⌥⌘T", "new tab"],
    ];
  if (view.kind === "terminal")
    return [
      ["⇧⌘T", "new tab"],
      ["⌘J", "dock"],
      ["⌘Esc", "leave the terminal"],
    ];
  if (view.kind === "repository")
    return [
      ["J K", "rows"],
      ["[ ]", "files"],
      ["N P", "hunks"],
      ["⌘1–5", "tabs"],
      ["/", "filter"],
    ];
  if (view.kind === "workspace")
    return view.tab === "activity"
      ? [
          ["⌘1 ⌘2", "tabs"],
          ["⌥⌘B", "panel"],
        ]
      : [
          ["J K", "rows"],
          ["↵", "open"],
          ["/", "filter"],
          ["⌘1 ⌘2", "tabs"],
          ["⌥⌘B", "panel"],
        ];
  return [
    ["J K", "rows"],
    ["↵", "open"],
    ["/", "filter"],
  ];
}

/** Bottom bar: overall state of the repositories in view, Git version, and last check. */
export default function StatusBar({
  snapshot,
  scope,
  selected,
  view,
  notice,
  onRetry,
  onOpenHost,
}: {
  snapshot: AppSnapshot | null;
  /** Repositories the main area shows; one when a repository is open. */
  scope: RepositorySummary[];
  selected: RepositorySummary | null;
  view: View;
  /** A short-lived message, such as the outcome of a fetch. */
  notice: string | null;
  onRetry: () => void;
  /** Settings → Agents, at a run host's page. */
  onOpenHost: (hostId: string) => void;
}) {
  const published = useFooterHints();
  const contextual = view.kind === "pullRequest";
  const jobs = Object.values(useHostJobsContext()).filter(
    (j) => j.state === "running",
  );
  const now = useNow(jobs.length > 0);
  let state = "unknown";
  let label = "Loading…";
  let retry = false;
  if (snapshot) {
    const failing = scope.filter(
      (r) => r.state === "error" || r.state === "missing",
    );
    const stale = scope.filter((r) => r.state === "stale");
    if (scope.some((r) => r.state === "refreshing")) {
      state = "refreshing";
      label = "Refreshing…";
    } else if (failing.length) {
      state = "error";
      label =
        failing.length === 1
          ? `${failing[0].name}: ${failing[0].error?.message ?? "unavailable"}`
          : `${failing.length} repositories unavailable`;
      retry = true;
    } else if (stale.length) {
      state = "stale";
      label =
        stale.length === 1
          ? `${stale[0].name} is stale; showing its last snapshot`
          : `${stale.length} repositories are stale; showing their last snapshots`;
      retry = true;
    } else {
      state = "clean";
      label = scope.length ? "Up to date" : "No repositories";
    }
  }
  const checked =
    scope
      .map((r) => r.last_checked_at)
      .filter((t): t is string => !!t)
      .sort()
      .at(-1) ?? null;
  const t = totals(scope);

  return (
    <footer className="flex h-[26px] shrink-0 items-center gap-3.5 overflow-hidden border-t bg-sidebar px-3.5 text-[11.5px] whitespace-nowrap text-muted">
      <span
        className={`flex min-w-0 items-center gap-1.5 ${state === "clean" ? "text-fg-2" : state === "unknown" ? "" : "text-dirty"}`}
      >
        <span className="dot" data-state={state} />
        <span className="truncate">{label}</span>
      </span>
      {retry && (
        <button
          type="button"
          className="text-link hover:underline"
          onClick={onRetry}
        >
          Retry
        </button>
      )}
      {notice && (
        <span className="truncate text-fg-2" role="status">
          {notice}
        </span>
      )}
      {selected && !notice && (
        <span className="mono selectable truncate max-[820px]:hidden">
          {shortPath(selected.display_path)}
        </span>
      )}
      <span className="flex-1" />
      {jobs.map((job) => (
        <button
          key={job.id}
          type="button"
          className="flex shrink-0 items-center gap-1.5 text-fg-2 hover:text-fg"
          title={`${job.host_name}: ${stepOf(job)}. Show its progress.`}
          onClick={() => onOpenHost(job.host_id)}
        >
          <Spinner />
          <span className="tabular">
            {job.host_name} {jobVerb(job)} · {jobElapsed(job, now)}
          </span>
        </button>
      ))}
      {contextual ? (
        <ContextHints hints={published ?? []} />
      ) : (
        <span className="hidden items-center gap-2.5 xl:flex">
          {hints(view).map(([keys, label]) => (
            <span key={label} className="flex items-center gap-1">
              <span className="kbd">{keys}</span>
              {label}
            </span>
          ))}
        </span>
      )}
      {snapshot && !selected && scope.length > 0 && (
        <span className="shrink-0">
          {t.upToDate} of {scope.length} up to date
        </span>
      )}
      {snapshot && selected && (
        <span title="Repositories Brainiac keeps up to date">
          Watching {snapshot.repositories.length}
        </span>
      )}
      {snapshot?.git.version && (
        <span className="max-[960px]:hidden" title={snapshot.git.version}>
          Git{" "}
          {snapshot.git.version
            .replace(/^git version /, "")
            .replace(/\s*\(.*\)$/, "")}
        </span>
      )}
      <span>Checked {relativeTime(checked)}</span>
    </footer>
  );
}

/**
 * A view's own shortcuts, always on screen. A hint that cannot act now stays,
 * dimmed, with the reason for a screen reader and a tooltip. In a narrow
 * window only the essential ones remain, so the way to the full list is
 * never lost.
 */
function ContextHints({ hints }: { hints: FooterHint[] }) {
  return (
    <fieldset className="m-0 flex min-w-0 shrink items-center gap-2.5 overflow-hidden border-0 p-0">
      <legend className="sr-only">Keyboard shortcuts</legend>
      {hints.map((h) => {
        const body = (
          <>
            {h.keys && <span className="kbd">{h.keys}</span>}
            {h.label}
            {h.disabled && h.why && (
              <span className="sr-only"> (unavailable: {h.why})</span>
            )}
          </>
        );
        const visibility = h.essential ? "" : "max-xl:hidden";
        const dim = h.disabled ? "opacity-50" : "";
        const cls = `flex shrink-0 items-center gap-1 ${visibility} ${dim}`;
        return h.onActivate ? (
          <button
            key={`${h.keys}${h.label}`}
            type="button"
            className={`${cls} hover:text-fg`}
            aria-disabled={h.disabled || undefined}
            title={h.disabled ? h.why : undefined}
            onClick={h.onActivate}
          >
            {body}
          </button>
        ) : (
          <span
            key={`${h.keys}${h.label}`}
            className={cls}
            aria-disabled={h.disabled || undefined}
            title={h.disabled ? h.why : undefined}
          >
            {body}
          </span>
        );
      })}
    </fieldset>
  );
}
