/**
 * Fixed-row-height windowing for long lists such as diffs (SPEC §7:
 * "virtualize long output"). Short lists render in full so text selection
 * and copy keep working across the whole patch.
 */

/** Lists with at most this many rows are not windowed. */
export const VIRTUALIZE_ABOVE = 1000;

/** Rows rendered beyond each edge of the viewport, to hide blank flashes while scrolling. */
const OVERSCAN = 40;

/** Half-open range `[start, end)` of rows to render. */
export function visibleRange(
  count: number,
  rowHeight: number,
  scrollTop: number,
  viewportHeight: number,
): { start: number; end: number } {
  if (count <= VIRTUALIZE_ABOVE) return { start: 0, end: count };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const visible = Math.ceil(Math.max(0, viewportHeight) / rowHeight);
  const start = Math.min(count, Math.max(0, first - OVERSCAN));
  const end = Math.min(count, first + visible + OVERSCAN);
  return { start, end: Math.max(start, end) };
}
