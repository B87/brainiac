/**
 * Loading one patch: the selected comparison, the ignore-whitespace
 * preference, and stale-response handling in one place. While a new patch
 * loads, the previous one stays available (SPEC §4, refresh behavior).
 */
import { useEffect, useRef, useState } from "react";
import { type DiffResult, type DiffSelector, errorMessage, ipc } from "./ipc";
import { usePref } from "./prefs";
import { createLatest } from "./stale";

export type DiffState = {
  diff: DiffResult | null;
  loading: boolean;
  ignoreWhitespace: boolean;
  setIgnoreWhitespace: (on: boolean) => void;
};

/**
 * Load the patch for `selector` (null shows nothing). A change of
 * `revalidate` reloads the same selection, such as after a new status
 * observation.
 */
export function useDiff(
  repositoryId: string,
  selector: DiffSelector | null,
  revalidate: unknown,
  onError: (message: string) => void,
): DiffState {
  const [ignoreWhitespace, setIgnoreWhitespace] = usePref(
    "brainiac.diff.ignoreWhitespace",
    false,
  );
  const [diff, setDiff] = useState<DiffResult | null>(null);
  const [loading, setLoading] = useState(false);
  const latest = useRef(createLatest()).current;
  const key = selector ? JSON.stringify(selector) : null;
  const errorRef = useRef(onError);
  errorRef.current = onError;

  // biome-ignore lint/correctness/useExhaustiveDependencies: `key` stands for `selector`; `revalidate` only triggers a reload.
  useEffect(() => {
    if (!selector) {
      latest.cancel();
      setDiff(null);
      setLoading(false);
      return;
    }
    setLoading(true);
    void latest.run(
      () =>
        ipc.getDiff(repositoryId, selector, {
          ignore_whitespace: ignoreWhitespace,
        }),
      (result) => {
        setLoading(false);
        if (result.repository_id === repositoryId) setDiff(result);
      },
      (e) => {
        setLoading(false);
        errorRef.current(errorMessage(e));
      },
    );
  }, [repositoryId, key, revalidate, ignoreWhitespace, latest]);

  return { diff, loading, ignoreWhitespace, setIgnoreWhitespace };
}
