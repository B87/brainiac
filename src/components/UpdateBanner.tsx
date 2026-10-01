import type { Update } from "@tauri-apps/plugin-updater";
import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "../lib/ipc";
import { checkForUpdate, formatBytes, installUpdate } from "../lib/updater";

/** Delay before the silent startup check so it never competes with the first repository refresh. */
const STARTUP_CHECK_DELAY_MS = 8_000;

type Phase =
  | { kind: "hidden" }
  | { kind: "checking" }
  | { kind: "up_to_date" }
  | { kind: "available"; update: Update }
  | {
      kind: "downloading";
      update: Update;
      received: number;
      total: number | null;
    }
  | { kind: "error"; message: string };

type Props = {
  /** Incremented by the parent whenever the user picks "Check for Updates…". */
  manualTick: number;
};

export default function UpdateBanner({ manualTick }: Props) {
  const [phase, setPhase] = useState<Phase>({ kind: "hidden" });
  const busy = useRef(false);

  const runCheck = useCallback(async (manual: boolean) => {
    if (busy.current) return;
    busy.current = true;
    if (manual) setPhase({ kind: "checking" });
    try {
      const update = await checkForUpdate();
      if (update) setPhase({ kind: "available", update });
      else if (manual) setPhase({ kind: "up_to_date" });
    } catch (e) {
      // A failed background check is not worth a banner; a manual one is.
      if (manual) setPhase({ kind: "error", message: errorMessage(e) });
    } finally {
      busy.current = false;
    }
  }, []);

  // Silent check shortly after launch. Skipped in `pnpm tauri dev`, where the
  // running version never matches a published release.
  useEffect(() => {
    if (import.meta.env.DEV) return;
    const timer = setTimeout(
      () => void runCheck(false),
      STARTUP_CHECK_DELAY_MS,
    );
    return () => clearTimeout(timer);
  }, [runCheck]);

  useEffect(() => {
    if (manualTick > 0) void runCheck(true);
  }, [manualTick, runCheck]);

  const install = useCallback(async (update: Update) => {
    setPhase({ kind: "downloading", update, received: 0, total: null });
    try {
      await installUpdate(update, (received, total) =>
        setPhase({ kind: "downloading", update, received, total }),
      );
    } catch (e) {
      setPhase({ kind: "error", message: errorMessage(e) });
    }
  }, []);

  if (phase.kind === "hidden") return null;

  const dismiss = (
    <button
      type="button"
      className="rounded border px-2 py-0.5"
      onClick={() => setPhase({ kind: "hidden" })}
    >
      Dismiss
    </button>
  );

  return (
    <div className="flex items-center gap-3 border-b border-sky-300 bg-sky-50 px-3 py-2 text-sky-900 dark:border-sky-800 dark:bg-sky-950 dark:text-sky-100">
      {phase.kind === "checking" && (
        <span className="flex-1">Checking for updates…</span>
      )}
      {phase.kind === "up_to_date" && (
        <>
          <span className="flex-1">Brainiac is up to date.</span>
          {dismiss}
        </>
      )}
      {phase.kind === "available" && (
        <>
          <span className="flex-1">
            Brainiac {phase.update.version} is available
            {phase.update.body ? ` — ${firstLine(phase.update.body)}` : ""}.
          </span>
          <button
            type="button"
            className="rounded border px-2 py-0.5 font-medium"
            onClick={() => void install(phase.update)}
          >
            Install and restart
          </button>
          {dismiss}
        </>
      )}
      {phase.kind === "downloading" && (
        <span className="flex-1">
          Downloading Brainiac {phase.update.version}…{" "}
          {phase.total
            ? `${Math.min(100, Math.round((phase.received / phase.total) * 100))}%`
            : formatBytes(phase.received)}
        </span>
      )}
      {phase.kind === "error" && (
        <>
          <span className="flex-1 selectable">
            Update failed: {phase.message}
          </span>
          {dismiss}
        </>
      )}
    </div>
  );
}

function firstLine(text: string): string {
  const line = text.split("\n").find((l) => l.trim().length > 0) ?? "";
  return line.replace(/^#+\s*/, "").trim();
}
