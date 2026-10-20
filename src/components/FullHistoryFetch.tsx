import { useState } from "react";
import { errorMessage, ipc, isAppError } from "../lib/ipc";

/** Whether a run's or an explanation's refusal is a shallow clone's. */
export function isShallowClone(e: unknown): boolean {
  return isAppError(e) && e.code === "SHALLOW_CLONE";
}

/**
 * **Fetch full history** (SPEC.md, Fetching): the remedy New run and Explain
 * offer for a shallow clone. It downloads the history the clone left out,
 * once, and never touches local branches or the working tree.
 */
export default function FullHistoryFetch({
  repositoryId,
  onFetched,
}: {
  repositoryId: string;
  onFetched: () => void;
}) {
  const [fetching, setFetching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fetchHistory = () => {
    setFetching(true);
    setError(null);
    ipc
      .fetchFullHistory(repositoryId)
      .then(() => onFetched())
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setFetching(false));
  };
  return (
    <div className="flex flex-col items-start gap-1.5">
      <button
        type="button"
        className="btn btn-sm"
        disabled={fetching}
        onClick={fetchHistory}
      >
        {fetching ? "Fetching full history…" : "Fetch full history"}
      </button>
      <span className="text-[12px] text-muted">
        Downloads the rest of the repository's history once, which can take a
        while for a large repository. Your branches and files are not changed.
      </span>
      {error && (
        <span role="alert" className="text-[12.5px] text-conflict">
          {error}
        </span>
      )}
    </div>
  );
}
