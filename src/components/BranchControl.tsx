import { useState } from "react";
import { relativeTime } from "../lib/format";
import type { RepositorySummary } from "../lib/ipc";
import { branchStatus, middleTruncate, plural } from "../lib/repo";
import { BranchIcon, ChevronDown, FetchIcon } from "./icons";
import Popover from "./Popover";

const NOTE_TONE = {
  muted: "text-muted",
  warn: "text-dirty",
  error: "text-conflict",
} as const;

/**
 * The header's branch: its name, ahead and behind, and how fresh the last
 * fetch is, always visible; the upstream and Fetch now in its popover
 * (SPEC.md, Main window v0.1).
 */
export default function BranchControl({
  repository: r,
  fetching,
  disabled,
  onFetch,
  onShowUpstream,
}: {
  repository: RepositorySummary;
  fetching: boolean;
  /** The repository cannot be read, so it cannot be fetched either. */
  disabled: boolean;
  onFetch: () => void;
  /** Show the upstream's commits in History. */
  onShowUpstream: (ref: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const s = branchStatus(r, fetching);
  const u = r.upstream;
  return (
    <div className="relative min-w-0">
      <button
        type="button"
        className="mono flex h-[30px] max-w-full min-w-0 items-center gap-1.5 rounded-[7px] px-2 text-[12px] hover:bg-control"
        aria-haspopup="dialog"
        aria-expanded={open}
        title={s.name}
        onClick={() => setOpen(!open)}
      >
        <BranchIcon size={13} className="shrink-0 text-fg-2" />
        <span
          className={`min-w-0 truncate @max-[960px]:hidden ${s.detached ? "text-dirty" : ""}`}
        >
          {s.name}
        </span>
        <span
          className={`hidden shrink-0 @max-[960px]:inline ${s.detached ? "text-dirty" : ""}`}
        >
          {middleTruncate(s.name, 18)}
        </span>
        {s.upstream && (
          <span className="max-w-[220px] shrink truncate text-muted @max-[1100px]:hidden">
            → {s.upstream}
          </span>
        )}
        {s.ahead > 0 && (
          <span className="shrink-0 text-link" title={`${s.ahead} to push`}>
            ↑{s.ahead}
          </span>
        )}
        {s.behind > 0 && (
          <span className="shrink-0 text-dirty" title={`${s.behind} to pull`}>
            ↓{s.behind}
          </span>
        )}
        {s.note && (
          <span
            className={`shrink-0 whitespace-nowrap ${NOTE_TONE[s.note.tone]}`}
          >
            {s.note.tone === "error" ? "! " : "· "}
            {s.note.text}
          </span>
        )}
        <ChevronDown size={10} className="shrink-0 text-muted" />
      </button>
      {open && (
        <Popover
          role="dialog"
          label="Branch"
          onClose={() => setOpen(false)}
          className="w-[340px] p-3"
        >
          <div className="flex flex-col gap-3 text-[12.5px] leading-snug text-fg-2">
            <div className="flex flex-col gap-0.5">
              <span className="mono selectable text-[13px] break-all text-fg">
                {s.name}
              </span>
              <span className="text-[12px] text-muted">
                {s.detached ? (
                  "HEAD is at a commit, not on a branch."
                ) : u ? (
                  <>
                    Tracks <span className="mono">{u.ref}</span>
                  </>
                ) : (
                  "No upstream: the branch was never pushed, or its remote branch was deleted."
                )}
              </span>
            </div>
            {u && (
              <div className="flex flex-col gap-1.5">
                {u.ahead === 0 && u.behind === 0 && (
                  <Line mark="✓" tone="text-clean">
                    In step with {u.ref} as of the last fetch.
                  </Line>
                )}
                {u.ahead > 0 && (
                  <Line mark={`↑${u.ahead}`} tone="text-link">
                    {plural(u.ahead, "commit")} here that {u.ref} doesn't have
                    yet.
                  </Line>
                )}
                {u.behind > 0 && (
                  <Line mark={`↓${u.behind}`} tone="text-dirty">
                    {plural(u.behind, "commit")} on {u.ref} you don't have yet.{" "}
                    <button
                      type="button"
                      className="text-link hover:underline"
                      onClick={() => {
                        setOpen(false);
                        onShowUpstream(u.ref);
                      }}
                    >
                      Show in History
                    </button>
                  </Line>
                )}
              </div>
            )}
            <div className="flex flex-col gap-1">
              <span
                className={s.note?.tone === "warn" ? "text-dirty" : "text-fg-2"}
              >
                {r.last_fetch_at
                  ? `Last fetched ${relativeTime(r.last_fetch_at)}.`
                  : "Not fetched yet."}
              </span>
              {r.fetch_error && (
                <span className="selectable text-conflict">
                  The last fetch failed: {r.fetch_error.message}
                </span>
              )}
            </div>
            <div className="flex items-center justify-between">
              <button
                type="button"
                className="btn"
                disabled={fetching || disabled}
                onClick={() => {
                  setOpen(false);
                  onFetch();
                }}
              >
                <FetchIcon
                  size={13}
                  className={fetching ? "animate-pulse" : ""}
                />
                {fetching ? "Fetching…" : "Fetch now"}
              </button>
              <span className="kbd">⇧⌘F</span>
            </div>
            <p className="m-0 border-t pt-2 text-[11.5px] text-muted">
              Fetch updates remote-tracking branches and tags only. It never
              changes your files or your branches.
            </p>
          </div>
        </Popover>
      )}
    </div>
  );
}

function Line({
  mark,
  tone,
  children,
}: {
  mark: string;
  tone: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex gap-2">
      <span className={`mono w-9 shrink-0 ${tone}`}>{mark}</span>
      <span>{children}</span>
    </div>
  );
}
