import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

/** Progress callback: bytes received so far and the total when the server reports it. */
export type ProgressFn = (received: number, total: number | null) => void;

/**
 * Asks the configured endpoint (the latest GitHub release's `latest.json`)
 * whether a newer signed build exists. Resolves to `null` when up to date.
 * The signature check happens in Rust; a tampered artifact is rejected there.
 */
export function checkForUpdate(): Promise<Update | null> {
  return check({ timeout: 15_000 });
}

/** Downloads, verifies and installs `update`, then restarts the app. */
export async function installUpdate(
  update: Update,
  onProgress: ProgressFn,
): Promise<void> {
  let received = 0;
  let total: number | null = null;
  await update.downloadAndInstall((event) => {
    if (event.event === "Started") {
      total = event.data.contentLength ?? null;
      onProgress(0, total);
    } else if (event.event === "Progress") {
      received += event.data.chunkLength;
      onProgress(received, total);
    }
  });
  await relaunch();
}

export function formatBytes(n: number): string {
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
