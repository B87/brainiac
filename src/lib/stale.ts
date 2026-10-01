/**
 * Guards against stale async responses. Each `run` call gets a sequence
 * number; results whose sequence is no longer the latest are dropped, so a
 * slow diff for a previously selected file never overwrites the current one
 * (SPEC.md, Changes and diffs: "discard obsolete requests after selection switches").
 */
export function createLatest() {
  let seq = 0;
  return {
    /** Start a request; returns a function that tells whether it is still current. */
    begin(): () => boolean {
      const mine = ++seq;
      return () => mine === seq;
    },
    /** Run `task` and call `onResult` only if no newer request started meanwhile. */
    async run<T>(
      task: () => Promise<T>,
      onResult: (value: T) => void,
      onError?: (e: unknown) => void,
    ) {
      const isCurrent = this.begin();
      try {
        const value = await task();
        if (isCurrent()) onResult(value);
      } catch (e) {
        if (isCurrent()) onError?.(e);
      }
    },
    /** Invalidate everything in flight without starting a new request. */
    cancel() {
      seq++;
    },
  };
}

export type Latest = ReturnType<typeof createLatest>;
