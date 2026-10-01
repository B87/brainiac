import { relativeTime, shortPath } from "../lib/format";
import type { AppSnapshot, RepositorySummary } from "../lib/ipc";
import { totals } from "../lib/workspace";
import type { View } from "./Sidebar";

/** Shortcuts that work in each view (SPEC.md, Keyboard defaults). */
function hints(view: View): Array<[string, string]> {
  if (view.kind === "repository")
    return [
      ["J K", "rows"],
      ["[ ]", "files"],
      ["N P", "hunks"],
      ["⌘1–3", "tabs"],
      ["/", "filter"],
    ];
  if (view.kind === "workspace")
    return view.tab === "activity"
      ? [["⌘1 ⌘2", "tabs"]]
      : [
          ["J K", "rows"],
          ["↵", "open"],
          ["/", "filter"],
          ["⌘1 ⌘2", "tabs"],
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
}: {
  snapshot: AppSnapshot | null;
  /** Repositories the main area shows; one when a repository is open. */
  scope: RepositorySummary[];
  selected: RepositorySummary | null;
  view: View;
  /** A short-lived message, such as the outcome of a fetch. */
  notice: string | null;
  onRetry: () => void;
}) {
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
    <footer className="flex h-[26px] shrink-0 items-center gap-3.5 border-t bg-sidebar px-3.5 text-[11.5px] text-muted">
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
        <span className="mono selectable truncate">
          {shortPath(selected.display_path)}
        </span>
      )}
      <span className="flex-1" />
      <span className="hidden items-center gap-2.5 xl:flex">
        {hints(view).map(([keys, label]) => (
          <span key={label} className="flex items-center gap-1">
            <span className="kbd">{keys}</span>
            {label}
          </span>
        ))}
      </span>
      {snapshot && !selected && scope.length > 0 && (
        <span>
          {t.upToDate} of {scope.length} up to date
        </span>
      )}
      {snapshot && selected && (
        <span>Watching {snapshot.repositories.length} repositories</span>
      )}
      {snapshot?.git.version && (
        <span title={snapshot.git.version}>
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
