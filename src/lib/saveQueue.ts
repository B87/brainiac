/**
 * Saves that run one after another, in the order they were asked for. Each
 * value is complete (the whole settings object, not a patch), so a later save
 * never depends on an earlier one succeeding. `onIdle` runs once the last
 * queued save has finished, which is when the caller reloads.
 */
export type SaveQueue<T> = {
  enqueue: (value: T) => Promise<void>;
  /** Saves queued or running. */
  pending: () => number;
};

export function createSaveQueue<T>(
  save: (value: T) => Promise<void>,
  hooks: { onError: (e: unknown) => void; onIdle: () => void },
): SaveQueue<T> {
  let chain: Promise<void> = Promise.resolve();
  let pending = 0;
  return {
    enqueue(value) {
      pending++;
      chain = chain.then(async () => {
        try {
          await save(value);
        } catch (e) {
          hooks.onError(e);
        }
        pending--;
        if (pending === 0) hooks.onIdle();
      });
      return chain;
    },
    pending: () => pending,
  };
}
