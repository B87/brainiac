import { open, save } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { useState } from "react";
import { absoluteTime, shortPath } from "../lib/format";
import {
  type ExportResult,
  errorMessage,
  ipc,
  type RestorePreview,
  type RestoreResult,
} from "../lib/ipc";
import { plural } from "../lib/repo";
import Dialog from "./Dialog";

/**
 * Export (SPEC.md, Backup and restore): the vault's notes, a consistent copy
 * of Brainiac's data, tasks as JSON, and a manifest, in a new folder.
 */
export async function runExport(
  onNotice: (message: string) => void,
  onError: (message: string | null) => void,
): Promise<ExportResult | null> {
  try {
    const folder = await open({
      directory: true,
      multiple: false,
      title: "Choose Where to Save the Export",
    });
    if (typeof folder !== "string") return null;
    const result = await ipc.exportBackup(folder);
    if (result.complete)
      onNotice(
        `Exported ${plural(Number(result.notes), "note")} and ${plural(Number(result.tasks), "task")} to ${shortPath(result.path)}`,
      );
    return result;
  } catch (e) {
    onError(errorMessage(e));
    return null;
  }
}

/** An export that could not copy everything says so: it is never called complete. */
export function ExportProblems({
  result,
  onClose,
}: {
  result: ExportResult;
  onClose: () => void;
}) {
  return (
    <Dialog
      title="Export Incomplete"
      onClose={onClose}
      footer={
        <button type="button" className="btn btn-primary" onClick={onClose}>
          OK
        </button>
      }
    >
      <p className="mt-0">
        The export in {shortPath(result.path)} is missing{" "}
        {plural(result.problems.length, "file")} that kept changing while it was
        copied. Export again when they are not being edited.
      </p>
      <ul className="mono m-0 max-h-48 overflow-y-auto pl-5 text-[12px]">
        {result.problems.map((p) => (
          <li key={p}>{p}</li>
        ))}
      </ul>
    </Dialog>
  );
}

/**
 * Restore from an export: check it before replacing anything, choose where
 * its notes go, then restart into the restored data.
 */
export function RestoreDialog({ onClose }: { onClose: () => void }) {
  const [preview, setPreview] = useState<RestorePreview | null>(null);
  const [target, setTarget] = useState<{
    path: string;
    existing: boolean;
  } | null>(null);
  const [result, setResult] = useState<RestoreResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const pickExport = async () => {
    setError(null);
    try {
      const path = await open({
        directory: true,
        multiple: false,
        title: "Choose a Brainiac Export",
      });
      if (typeof path !== "string") return;
      setPreview(await ipc.previewRestore(path));
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  const pickTarget = async (existing: boolean) => {
    setError(null);
    const path = existing
      ? await open({
          directory: true,
          multiple: false,
          title: "Choose the Folder That Holds the Notes",
        })
      : await save({
          title: "Choose a New Folder for the Notes",
          defaultPath: preview?.vault_name ?? "Notes",
        });
    if (typeof path === "string") setTarget({ path, existing });
  };
  const restore = async () => {
    if (!preview || !target) return;
    setBusy(true);
    setError(null);
    try {
      setResult(
        await ipc.restoreBackup({
          export_path: preview.path,
          vault_path: target.path,
          use_existing_vault: target.existing,
        }),
      );
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  if (result)
    return (
      <Dialog
        title="Ready to Restore"
        onClose={() => void relaunch()}
        footer={
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void relaunch()}
          >
            Restart Brainiac
          </button>
        }
      >
        <p className="mt-0">
          Brainiac restarts to open the restored data. Your current data is
          snapshotted first, and search is rebuilt from the vault.
        </p>
        {result.matched_repositories > 0 && (
          <p>
            {plural(
              Number(result.matched_repositories),
              "repository",
              "repositories",
            )}{" "}
            matched by remote URL to repositories on this Mac.
          </p>
        )}
        {result.unmatched_repositories.length > 0 && (
          <p>
            Not found on this Mac: {result.unmatched_repositories.join(", ")}.
            They show as missing; use Locate… to point them at their folders.
          </p>
        )}
      </Dialog>
    );

  return (
    <Dialog
      title="Restore from Export"
      onClose={onClose}
      width={560}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!preview || !target || busy}
            onClick={() => void restore()}
          >
            Restore and Restart…
          </button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <p className="m-0 text-fg-2">
          A restore replaces Brainiac's data on this Mac (tasks, links,
          workspaces, settings) with the export's. Repositories registered here
          stay registered.
        </p>
        <div className="flex flex-col gap-1.5">
          <span className="section-label">1. The export</span>
          {preview ? (
            <div className="rounded-md border px-3 py-2">
              <div className="mono truncate text-[12px]">
                {shortPath(preview.path)}
              </div>
              <div className="text-[12px] text-muted">
                {absoluteTime(preview.exported_at)} · Brainiac{" "}
                {preview.app_version} · {plural(Number(preview.notes), "note")}{" "}
                · {plural(Number(preview.tasks), "task")} ·{" "}
                {plural(
                  preview.repositories.length,
                  "repository",
                  "repositories",
                )}
              </div>
            </div>
          ) : null}
          <button
            type="button"
            className="btn self-start"
            onClick={() => void pickExport()}
          >
            {preview ? "Choose Another Export…" : "Choose Export Folder…"}
          </button>
        </div>
        {preview && (
          <div className="flex flex-col gap-1.5">
            <span className="section-label">2. Where the notes go</span>
            {target && (
              <div className="rounded-md border px-3 py-2 text-[12px]">
                {target.existing
                  ? "Use the notes already in "
                  : "Copy the notes into "}
                <span className="mono">{shortPath(target.path)}</span>
              </div>
            )}
            <div className="flex gap-2">
              <button
                type="button"
                className="btn"
                onClick={() => void pickTarget(false)}
              >
                Copy into a New Folder…
              </button>
              <button
                type="button"
                className="btn"
                onClick={() => void pickTarget(true)}
              >
                Use an Existing Vault Folder…
              </button>
            </div>
            <span className="text-[12px] text-muted">
              Notes are matched by their brainiac_id, then by path and content,
              so a vault synced to this Mac keeps its tasks and links.
            </span>
          </div>
        )}
        {error && (
          <div role="alert" className="text-conflict">
            {error}
          </div>
        )}
      </div>
    </Dialog>
  );
}
