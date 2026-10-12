/**
 * Pane widths for behavior CSS cannot change on its own (SPEC.md, Main
 * window): a split diff that needs room, a list that folds away.
 */
import { useEffect, useState } from "react";

/**
 * Whether an element is narrower than `px`, kept current as it resizes.
 * Returns a callback ref for the element, so one that appears later (once
 * its data loads) is measured too.
 */
export function useNarrowerThan(
  px: number,
): [(el: HTMLElement | null) => void, boolean] {
  const [el, setEl] = useState<HTMLElement | null>(null);
  const [narrow, setNarrow] = useState(false);
  useEffect(() => {
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => {
      const w = entry.contentRect.width;
      // A hidden element measures 0: keep the last answer.
      if (w > 0) setNarrow(w < px);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [el, px]);
  return [setEl, narrow];
}
