import { describe, expect, it } from "vitest";
import { createSaveQueue } from "./saveQueue";

describe("createSaveQueue", () => {
  it("saves in order, reports errors, and goes idle once", async () => {
    const saved: number[] = [];
    const errors: unknown[] = [];
    let idle = 0;
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => {
      release = r;
    });
    const queue = createSaveQueue<number>(
      async (n) => {
        if (n === 1) await gate;
        if (n === 2) throw new Error("rejected");
        saved.push(n);
      },
      { onError: (e) => errors.push(e), onIdle: () => idle++ },
    );
    queue.enqueue(1);
    queue.enqueue(2);
    const last = queue.enqueue(3);
    expect(queue.pending()).toBe(3);
    release();
    await last;
    expect(saved).toEqual([1, 3]);
    expect(errors).toHaveLength(1);
    expect(idle).toBe(1);
    expect(queue.pending()).toBe(0);
  });
});
