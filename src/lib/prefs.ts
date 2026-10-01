/**
 * Per-viewer display preferences kept in localStorage (wrap, split layout,
 * hidden file list). Storage can be unavailable; then the default applies and
 * changes last until the window closes.
 */
import { useCallback, useState } from "react";

function read<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

export function usePref<T>(key: string, fallback: T): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(() => read(key, fallback));
  const set = useCallback(
    (next: T) => {
      setValue(next);
      try {
        localStorage.setItem(key, JSON.stringify(next));
      } catch {
        // Preference only.
      }
    },
    [key],
  );
  return [value, set];
}
