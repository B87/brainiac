import { describe, expect, it } from "vitest";
import { VIRTUALIZE_ABOVE, visibleRange } from "./virtual";

describe("visibleRange", () => {
  it("renders short lists in full", () => {
    expect(visibleRange(VIRTUALIZE_ABOVE, 18, 5000, 600)).toEqual({
      start: 0,
      end: VIRTUALIZE_ABOVE,
    });
  });

  it("renders only the rows around the viewport of a long list", () => {
    const { start, end } = visibleRange(100_000, 20, 20 * 50_000, 400);
    expect(start).toBeLessThanOrEqual(50_000);
    expect(end).toBeGreaterThanOrEqual(50_020);
    expect(end - start).toBeLessThan(200);
  });

  it("clamps at both ends", () => {
    expect(visibleRange(5000, 20, -100, 400).start).toBe(0);
    const past = visibleRange(5000, 20, 20 * 9000, 400);
    expect(past.end).toBe(5000);
    expect(past.start).toBeLessThanOrEqual(past.end);
  });
});
