/**
 * A repository shown the same way wherever it appears (SPEC.md, Main window
 * v0.2): name, branch, a status dot with its words, ahead and behind when
 * not zero, and how fresh the data is.
 */
import { relativeTime } from "../lib/format";
import type { RepositorySummary } from "../lib/ipc";
import { headLabel, repoTone } from "../lib/repo";

/** Fetched data older than this is shown as stale (SPEC.md, Fetching). */
const STALE_FETCH_MS = 2 * 24 * 3600 * 1000;

export function statusWords(repo: RepositorySummary): string {
  const tone = repoTone(repo);
  if (tone === "missing") return "missing";
  if (tone === "conflict") return "conflicted";
  if (tone === "error") return "error";
  const n = repo.counts?.unique_paths ?? 0;
  if (n > 0) return `${n} changed`;
  if (tone === "unknown") return "not checked yet";
  return "clean";
}

export function freshness(repo: RepositorySummary, now = Date.now()) {
  if (repo.last_fetch_at) {
    const stale = now - Date.parse(repo.last_fetch_at) > STALE_FETCH_MS;
    return { text: `fetched ${relativeTime(repo.last_fetch_at, now)}`, stale };
  }
  return {
    text: `checked ${relativeTime(repo.last_checked_at, now)}`,
    stale: false,
  };
}

/** One line: dot, name, branch, status words, ahead/behind, freshness. */
export function RepoLine({
  repo,
  name,
  compact = false,
}: {
  repo: RepositorySummary;
  name?: string;
  compact?: boolean;
}) {
  const tone = repoTone(repo);
  const u = repo.upstream;
  const fresh = freshness(repo);
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <span className="dot" data-state={tone} />
      <span className="truncate font-medium">{name ?? repo.name}</span>
      {tone !== "missing" && (
        <span className="mono truncate text-[11.5px] text-muted">
          {headLabel(repo)}
        </span>
      )}
      <span
        className={`shrink-0 text-[11.5px] ${tone === "conflict" ? "font-semibold text-conflict" : tone === "dirty" ? "text-dirty" : "text-muted"}`}
      >
        {statusWords(repo)}
      </span>
      {u && (u.ahead > 0 || u.behind > 0) && (
        <span className="tabular shrink-0 text-[11.5px] text-muted">
          {u.ahead ? `↑${u.ahead}` : ""}
          {u.ahead && u.behind ? " " : ""}
          {u.behind ? `↓${u.behind}` : ""}
        </span>
      )}
      {!compact && tone !== "missing" && (
        <span
          className={`shrink-0 text-[11.5px] ${fresh.stale ? "text-dirty" : "text-muted"}`}
        >
          {fresh.text}
        </span>
      )}
    </span>
  );
}

/** A compact chip for rows: dot and name, the rest in its tooltip. */
export function RepoChip({
  repo,
  fallbackName,
  onClick,
}: {
  repo: RepositorySummary | undefined;
  /** Shown when the repository was removed. */
  fallbackName?: string;
  onClick?: () => void;
}) {
  const label = repo?.name ?? fallbackName ?? "Removed repository";
  const title = repo
    ? `${repo.name} · ${headLabel(repo)} · ${statusWords(repo)}`
    : "This repository was removed from Brainiac";
  const body = (
    <>
      <span className="dot" data-state={repo ? repoTone(repo) : "missing"} />
      <span>{label}</span>
      {repo && (repo.counts?.unique_paths ?? 0) > 0 && (
        <span className="text-dirty">{repo.counts?.unique_paths}</span>
      )}
    </>
  );
  return onClick ? (
    <button type="button" className="ref-chip" title={title} onClick={onClick}>
      {body}
    </button>
  ) : (
    <span className="ref-chip" title={title}>
      {body}
    </span>
  );
}
