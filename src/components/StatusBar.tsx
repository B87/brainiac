import { relativeTime } from "../lib/format";
import type { AppSnapshot, RepositorySummary } from "../lib/ipc";

export default function StatusBar({
  snapshot,
  selected,
}: {
  snapshot: AppSnapshot | null;
  selected: RepositorySummary | null;
}) {
  let label = "Loading…";
  let checked: string | null = null;
  if (snapshot) {
    const repos = selected ? [selected] : snapshot.repositories;
    if (repos.some((r) => r.state === "refreshing")) label = "Refreshing";
    else if (repos.some((r) => r.state === "error" || r.state === "missing"))
      label = "Error";
    else if (repos.some((r) => r.state === "stale")) label = "Stale";
    else label = repos.length ? "Up to date" : "No repositories";
    checked =
      repos
        .map((r) => r.last_checked_at)
        .filter((t): t is string => !!t)
        .sort()
        .at(-1) ?? null;
  }
  return (
    <footer className="muted flex items-center gap-3 border-t px-3 py-1 text-[11px]">
      <span>{label}</span>
      {snapshot?.git.version && (
        <span className="ml-auto">{snapshot.git.version}</span>
      )}
      <span>Last checked {relativeTime(checked)}</span>
    </footer>
  );
}
