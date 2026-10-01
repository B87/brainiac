import { relativeTime, shortPath } from "../lib/format";
import { errorMessage, ipc, type RepositorySummary } from "../lib/ipc";

type Props = {
  repository: RepositorySummary;
  onRefresh: () => void;
  onRemove: () => void;
  onError: (message: string | null) => void;
};

export default function Inspector({
  repository: r,
  onRefresh,
  onRemove,
  onError,
}: Props) {
  const head = r.head;
  const run = (p: Promise<unknown>) =>
    void p.catch((e) => onError(errorMessage(e)));
  return (
    <aside className="pane flex w-64 shrink-0 flex-col gap-3 overflow-y-auto border-l p-3">
      <div>
        <div className="font-semibold">{r.name}</div>
        <div className="muted mono selectable break-all text-[11px]">
          {shortPath(r.display_path)}
        </div>
        {r.display_path !== r.canonical_root && (
          <div className="muted text-[11px]">linked worktree</div>
        )}
      </div>

      <Field label="HEAD">
        {!head ? (
          "—"
        ) : head.kind === "branch" ? (
          <span className="mono">{head.branch}</span>
        ) : head.kind === "detached" ? (
          `detached at ${head.commit_id?.slice(0, 10)}`
        ) : (
          "no commits yet"
        )}
      </Field>
      <Field label="Upstream">
        {r.upstream ? (
          <span className="mono">
            {r.upstream.ref} · ↑{r.upstream.ahead} ↓{r.upstream.behind}
          </span>
        ) : (
          <span className="muted">none</span>
        )}
        {r.upstream && (
          <div className="muted text-[11px]">
            From local refs; no fetch performed.
          </div>
        )}
      </Field>
      <Field label="Last refresh">
        {relativeTime(r.last_checked_at)} · {r.state}
      </Field>
      {r.error && (
        <Field label="Problem">
          <div className="text-red-600 dark:text-red-400">
            {r.error.message}
          </div>
          {r.error.details && (
            <details className="muted selectable mt-1 whitespace-pre-wrap text-[11px]">
              <summary>Details</summary>
              {r.error.details}
            </details>
          )}
        </Field>
      )}

      <div className="mt-2 flex flex-col gap-1">
        <button
          type="button"
          className="rounded-md border px-2 py-1"
          onClick={() => run(ipc.openInEditor(r.id))}
        >
          Open in editor
        </button>
        <button
          type="button"
          className="rounded-md border px-2 py-1"
          onClick={() => run(ipc.revealInFinder(r.id))}
        >
          Reveal in Finder
        </button>
        <button
          type="button"
          className="rounded-md border px-2 py-1"
          onClick={onRefresh}
        >
          Refresh <kbd>⌘R</kbd>
        </button>
        <button
          type="button"
          className="muted mt-2 rounded-md border px-2 py-1"
          onClick={onRemove}
          title="Removes the registration only; your files are untouched."
        >
          Remove from Brainiac
        </button>
      </div>
    </aside>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="muted text-[11px] font-semibold uppercase tracking-wide">
        {label}
      </div>
      <div>{children}</div>
    </div>
  );
}
