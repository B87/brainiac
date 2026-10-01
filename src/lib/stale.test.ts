import { describe, expect, it } from "vitest";
import { createLatest } from "./stale";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe("createLatest", () => {
  it("drops a slow earlier response that resolves after a newer request", async () => {
    const latest = createLatest();
    const seen: string[] = [];
    const slow = latest.run(
      async () => {
        await sleep(30);
        return "slow";
      },
      (v) => seen.push(v),
    );
    const fast = latest.run(
      async () => "fast",
      (v) => seen.push(v),
    );
    await Promise.all([slow, fast]);
    expect(seen).toEqual(["fast"]);
  });

  it("drops errors from stale requests too", async () => {
    const latest = createLatest();
    const errors: unknown[] = [];
    const p = latest.run(
      async () => {
        await sleep(10);
        throw new Error("old");
      },
      () => {},
      (e) => errors.push(e),
    );
    latest.cancel();
    await p;
    expect(errors).toEqual([]);
  });

  it("delivers the newest result", async () => {
    const latest = createLatest();
    let value = "";
    await latest.run(
      async () => "a",
      (v) => (value = v),
    );
    await latest.run(
      async () => "b",
      (v) => (value = v),
    );
    expect(value).toBe("b");
  });
});
