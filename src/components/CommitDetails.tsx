import type { CommitSummary } from "../lib/ipc";

export default function CommitDetails({
  commit,
}: {
  commit: CommitSummary | null;
}) {
  if (!commit)
    return <div className="muted p-4">Select a commit to see its details.</div>;
  const copy = (text: string) => void navigator.clipboard?.writeText(text);
  return (
    <div className="selectable p-4">
      <h2 className="mb-2 text-base font-semibold">{commit.subject}</h2>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
        <dt className="muted">Commit</dt>
        <dd className="mono flex items-center gap-2">
          {commit.id}
          <button
            type="button"
            className="rounded border px-1 text-[11px]"
            onClick={() => copy(commit.id)}
          >
            Copy
          </button>
        </dd>
        <dt className="muted">Author</dt>
        <dd>
          {commit.author_name} &lt;{commit.author_email}&gt;
        </dd>
        <dt className="muted">Authored</dt>
        <dd>{new Date(commit.authored_at).toLocaleString()}</dd>
        <dt className="muted">Committed</dt>
        <dd>{new Date(commit.committed_at).toLocaleString()}</dd>
        <dt className="muted">Parents</dt>
        <dd className="mono">
          {commit.parent_ids.length
            ? commit.parent_ids.map((p) => p.slice(0, 10)).join(", ")
            : "none (root commit)"}
        </dd>
        {commit.decorations.length > 0 && (
          <>
            <dt className="muted">Refs</dt>
            <dd className="mono">{commit.decorations.join(", ")}</dd>
          </>
        )}
      </dl>
      <p className="muted mt-4 text-[11px]">
        Changed files and per-file patches for commits arrive in v0.1.
      </p>
    </div>
  );
}
