import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useState } from "react";
import { shortPath } from "../lib/format";
import {
  type AppSnapshot,
  errorMessage,
  ipc,
  type Workspace,
  type WorkspacePreview,
  type WorkspacePreviewEntry,
} from "../lib/ipc";
import { discoveryLabel } from "../lib/workspace";
import {
  CloseIcon,
  FolderIcon,
  FolderPlusIcon,
  GridIcon,
  RescanIcon,
} from "./icons";

/** Which flow the dialog starts in. */
export type AddMode =
  | { kind: "choose" }
  | { kind: "discover" }
  | { kind: "manual" }
  | { kind: "members"; workspace: Workspace };

type Props = {
  snapshot: AppSnapshot;
  mode: AddMode;
  onClose: () => void;
  onOpenRepository: () => void;
  /** Called with the created or updated workspace after the snapshot reloads. */
  onDone: (workspace: Workspace) => void;
};

/** Path a preview entry is tracked by. */
function entryPath(e: WorkspacePreviewEntry): string {
  return e.resolved_path ?? e.configured_path;
}

export default function AddDialog({
  snapshot,
  mode: initial,
  onClose,
  onOpenRepository,
  onDone,
}: Props) {
  const [mode, setMode] = useState<AddMode>(initial);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const submit = async (task: () => Promise<Workspace>) => {
    setBusy(true);
    setError(null);
    try {
      onDone(await task());
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const title =
    mode.kind === "choose"
      ? "Add to Brainiac"
      : mode.kind === "discover"
        ? "New workspace from a folder"
        : mode.kind === "manual"
          ? "New workspace"
          : `Add repositories to ${mode.workspace.name}`;

  return (
    <div className="absolute inset-0 z-30 flex items-start justify-center bg-black/30 pt-20">
      <button
        type="button"
        aria-label="Close"
        className="absolute inset-0"
        onClick={onClose}
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="relative flex max-h-[calc(100%-120px)] w-[560px] flex-col rounded-xl border border-control-line bg-header shadow-2xl"
      >
        <div className="flex items-center gap-2 border-b px-5 py-3.5">
          <h2 className="m-0 flex-1 text-[15px] font-semibold">{title}</h2>
          <button
            type="button"
            className="btn btn-sm btn-ghost px-1.5"
            aria-label="Close"
            onClick={onClose}
          >
            <CloseIcon />
          </button>
        </div>
        {error && (
          <div className="selectable border-b border-red-300 bg-red-50 px-5 py-2 text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100">
            {error}
          </div>
        )}
        {mode.kind === "choose" && (
          <Choose
            onOpenRepository={() => {
              onClose();
              onOpenRepository();
            }}
            onDiscover={() => setMode({ kind: "discover" })}
            onManual={() => setMode({ kind: "manual" })}
          />
        )}
        {mode.kind === "discover" && (
          <Discover
            busy={busy}
            onError={setError}
            onCreate={(request) => submit(() => ipc.createWorkspace(request))}
          />
        )}
        {mode.kind === "manual" && (
          <Manual
            snapshot={snapshot}
            busy={busy}
            onCreate={(name, paths) =>
              submit(() =>
                ipc.createWorkspace({
                  name,
                  discovery_mode: "manual",
                  discovery_root: null,
                  discovery_path: null,
                  paths,
                }),
              )
            }
          />
        )}
        {mode.kind === "members" && (
          <Members
            snapshot={snapshot}
            workspace={mode.workspace}
            busy={busy}
            onError={setError}
            onAdd={(paths) =>
              submit(() =>
                ipc.updateWorkspaceMembership({
                  workspace_id: mode.workspace.id,
                  add: paths,
                  remove: [],
                }),
              )
            }
          />
        )}
      </div>
    </div>
  );
}

function Choose({
  onOpenRepository,
  onDiscover,
  onManual,
}: {
  onOpenRepository: () => void;
  onDiscover: () => void;
  onManual: () => void;
}) {
  const option = (
    icon: React.ReactNode,
    label: string,
    detail: string,
    onClick: () => void,
    hint?: string,
  ) => (
    <button
      type="button"
      className="flex w-full items-start gap-3 rounded-lg border border-control-line px-4 py-3 text-left hover:bg-control"
      onClick={onClick}
    >
      <span className="mt-0.5 text-fg-2">{icon}</span>
      <span className="flex flex-1 flex-col gap-0.5">
        <span className="font-medium">{label}</span>
        <span className="text-[12px] text-muted">{detail}</span>
      </span>
      {hint && <span className="kbd">{hint}</span>}
    </button>
  );
  return (
    <div className="flex flex-col gap-2.5 p-5">
      {option(
        <FolderIcon size={16} />,
        "Open a repository",
        "Inspect one repository on its own. It does not need a workspace.",
        onOpenRepository,
        "⌘O",
      )}
      {option(
        <RescanIcon size={16} />,
        "New workspace from a folder",
        "Pick a folder and choose which repositories inside it to track. If the folder is itself a repository, it becomes the workspace root.",
        onDiscover,
      )}
      {option(
        <GridIcon size={16} />,
        "New workspace by picking repositories",
        "Group repositories that live anywhere on disk.",
        onManual,
      )}
    </div>
  );
}

function Discover({
  busy,
  onError,
  onCreate,
}: {
  busy: boolean;
  onError: (message: string | null) => void;
  onCreate: (request: {
    name: string;
    discovery_mode: "discovered";
    discovery_root: string;
    discovery_path: string | null;
    paths: string[];
  }) => void;
}) {
  const [folder, setFolder] = useState<string | null>(null);
  const [sub, setSub] = useState("");
  const [preview, setPreview] = useState<WorkspacePreview | null>(null);
  const [scanning, setScanning] = useState(false);
  const [name, setName] = useState("");
  const [chosen, setChosen] = useState<Set<string>>(new Set());

  const scan = async (dir: string, subfolder: string) => {
    setScanning(true);
    onError(null);
    try {
      const p = await ipc.discoverRepositories(
        dir,
        subfolder.trim() || undefined,
      );
      setPreview(p);
      setName((n) => n || p.name);
      setChosen(
        new Set(p.entries.filter((e) => e.status === "ok").map(entryPath)),
      );
    } catch (e) {
      setPreview(null);
      onError(errorMessage(e));
    } finally {
      setScanning(false);
    }
  };

  const pick = async () => {
    const dir = await open({
      directory: true,
      multiple: false,
      title: "Choose a folder",
    });
    if (typeof dir !== "string") return;
    setFolder(dir);
    setName("");
    void scan(dir, sub);
  };

  return (
    <>
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-5">
        <Field label="Folder">
          <div className="flex items-center gap-2">
            <span className="mono min-w-0 flex-1 truncate text-[12px] text-fg-2">
              {folder ? shortPath(folder) : "No folder chosen"}
            </span>
            <button type="button" className="btn btn-sm" onClick={pick}>
              {folder ? "Change…" : "Choose…"}
            </button>
          </div>
        </Field>
        <Field
          label="Look for repositories in"
          hint="A subfolder of the chosen folder, such as services. Leave empty to scan the folder itself."
        >
          <form
            className="flex gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (folder) void scan(folder, sub);
            }}
          >
            <input
              className="text-input mono flex-1 text-[12px]"
              placeholder="(the folder itself)"
              value={sub}
              onChange={(e) => setSub(e.target.value)}
            />
            <button
              type="submit"
              className="btn btn-sm"
              disabled={!folder || scanning}
            >
              {scanning ? "Scanning…" : "Scan"}
            </button>
          </form>
        </Field>
        {preview && (
          <>
            <Field label="Name">
              <input
                className="text-input w-full"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </Field>
            <Field label="Repositories to track">
              <EntryList
                entries={preview.entries}
                rootPath={folder}
                chosen={chosen}
                onChange={setChosen}
              />
            </Field>
          </>
        )}
      </div>
      <Footer
        busy={busy}
        disabled={!folder || !preview || !name.trim()}
        label="Create workspace"
        note={preview ? `${chosen.size} selected` : undefined}
        onSubmit={() =>
          folder &&
          onCreate({
            name: name.trim(),
            discovery_mode: "discovered",
            discovery_root: folder,
            discovery_path: sub.trim() || null,
            paths: [...chosen],
          })
        }
      />
    </>
  );
}

/** Checklist of discovery results; entries that cannot be tracked explain why. */
function EntryList({
  entries,
  rootPath,
  chosen,
  onChange,
  tracked,
}: {
  entries: WorkspacePreviewEntry[];
  rootPath: string | null;
  chosen: Set<string>;
  onChange: (next: Set<string>) => void;
  /** Paths already in the workspace, shown checked and disabled. */
  tracked?: Set<string>;
}) {
  if (entries.length === 0)
    return <div className="text-[12px] text-muted">No folders found.</div>;
  return (
    <div className="flex flex-col rounded-lg border">
      {entries.map((e) => {
        const path = entryPath(e);
        const already = tracked?.has(path) ?? false;
        const ok = e.status === "ok" && !already;
        const isRoot = rootPath !== null && e.configured_path === rootPath;
        return (
          <label
            key={e.configured_path}
            className={`flex items-center gap-2.5 border-t border-t-line-soft px-3 py-2 first:border-t-0 ${ok ? "" : "text-muted"}`}
          >
            <input
              type="checkbox"
              disabled={!ok}
              checked={already || chosen.has(path)}
              onChange={(ev) => {
                const next = new Set(chosen);
                if (ev.target.checked) next.add(path);
                else next.delete(path);
                onChange(next);
              }}
            />
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="flex items-center gap-1.5">
                <span className={ok ? "font-medium" : ""}>
                  {e.display_name}
                </span>
                {isRoot && e.status === "ok" && (
                  <span className="tag-box">ROOT</span>
                )}
              </span>
              <span className="truncate text-[11.5px] text-muted">
                {already
                  ? "Already in this workspace"
                  : (e.message ??
                    (e.existing_repository_id
                      ? "Already registered in Brainiac"
                      : shortPath(path)))}
              </span>
            </span>
          </label>
        );
      })}
    </div>
  );
}

function Manual({
  snapshot,
  busy,
  onCreate,
}: {
  snapshot: AppSnapshot;
  busy: boolean;
  onCreate: (name: string, paths: string[]) => void;
}) {
  const [name, setName] = useState("");
  const [paths, setPaths] = useState<Set<string>>(new Set());
  return (
    <>
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-5">
        <Field label="Name">
          <input
            // biome-ignore lint/a11y/noAutofocus: the name is the first thing to fill in.
            autoFocus
            className="text-input w-full"
            placeholder="Personal"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <Field label="Repositories">
          <RepositoryPicker
            snapshot={snapshot}
            chosen={paths}
            onChange={setPaths}
          />
        </Field>
      </div>
      <Footer
        busy={busy}
        disabled={!name.trim()}
        label="Create workspace"
        note={`${paths.size} selected`}
        onSubmit={() => onCreate(name.trim(), [...paths])}
      />
    </>
  );
}

/** Registered repositories as a checklist, plus folders added through the picker. */
function RepositoryPicker({
  snapshot,
  chosen,
  onChange,
  exclude,
}: {
  snapshot: AppSnapshot;
  chosen: Set<string>;
  onChange: (next: Set<string>) => void;
  exclude?: Set<string>;
}) {
  const [extra, setExtra] = useState<string[]>([]);
  const registered = snapshot.repositories
    .filter(
      (r) => !exclude?.has(r.display_path) && !exclude?.has(r.canonical_root),
    )
    .sort((a, b) => a.name.localeCompare(b.name));
  const known = new Set(registered.map((r) => r.display_path));
  const rows = [
    ...registered.map((r) => ({ path: r.display_path, name: r.name })),
    ...extra
      .filter((p) => !known.has(p))
      .map((p) => ({
        path: p,
        name: p.split("/").filter(Boolean).at(-1) ?? p,
      })),
  ];
  const addFolders = async () => {
    const picked = await open({
      directory: true,
      multiple: true,
      title: "Add repositories",
    });
    const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
    if (!list.length) return;
    setExtra((prev) => [...prev, ...list.filter((p) => !prev.includes(p))]);
    onChange(new Set([...chosen, ...list]));
  };
  return (
    <div className="flex flex-col gap-2">
      {rows.length > 0 && (
        <div className="flex max-h-64 flex-col overflow-y-auto rounded-lg border">
          {rows.map((r) => (
            <label
              key={r.path}
              className="flex items-center gap-2.5 border-t border-t-line-soft px-3 py-2 first:border-t-0"
            >
              <input
                type="checkbox"
                checked={chosen.has(r.path)}
                onChange={(ev) => {
                  const next = new Set(chosen);
                  if (ev.target.checked) next.add(r.path);
                  else next.delete(r.path);
                  onChange(next);
                }}
              />
              <span className="flex min-w-0 flex-col">
                <span className="font-medium">{r.name}</span>
                <span className="mono truncate text-[11px] text-muted">
                  {shortPath(r.path)}
                </span>
              </span>
            </label>
          ))}
        </div>
      )}
      <button
        type="button"
        className="btn btn-sm self-start"
        onClick={addFolders}
      >
        <FolderPlusIcon size={13} />
        Add folders…
      </button>
    </div>
  );
}

function Members({
  snapshot,
  workspace,
  busy,
  onError,
  onAdd,
}: {
  snapshot: AppSnapshot;
  workspace: Workspace;
  busy: boolean;
  onError: (message: string | null) => void;
  onAdd: (paths: string[]) => void;
}) {
  const [preview, setPreview] = useState<WorkspacePreview | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const members = useMemo(
    () => new Set(workspace.members.map((m) => m.canonical_path)),
    [workspace],
  );
  const root = workspace.discovery_root;
  const path = workspace.discovery_path;

  useEffect(() => {
    if (workspace.discovery_mode !== "discovered" || !root) return;
    ipc
      .discoverRepositories(root, path ?? undefined)
      .then(setPreview)
      .catch((e) => onError(errorMessage(e)));
  }, [workspace.discovery_mode, root, path, onError]);

  const memberRepoPaths = new Set(
    snapshot.repositories
      .filter((r) => workspace.members.some((m) => m.repository_id === r.id))
      .flatMap((r) => [r.display_path, r.canonical_root]),
  );

  return (
    <>
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-5">
        {preview && (
          <Field
            label={`Found in ${discoveryLabel(workspace) ?? "the folder"}`}
          >
            <EntryList
              entries={preview.entries}
              rootPath={root}
              chosen={chosen}
              tracked={members}
              onChange={setChosen}
            />
          </Field>
        )}
        <Field label={preview ? "Other repositories" : "Repositories"}>
          <RepositoryPicker
            snapshot={snapshot}
            chosen={chosen}
            exclude={memberRepoPaths}
            onChange={setChosen}
          />
        </Field>
      </div>
      <Footer
        busy={busy}
        disabled={chosen.size === 0}
        label="Add to workspace"
        note={`${chosen.size} selected`}
        onSubmit={() => onAdd([...chosen])}
      />
    </>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="section-label">{label}</span>
      {children}
      {hint && <span className="text-[11.5px] text-muted">{hint}</span>}
    </div>
  );
}

function Footer({
  busy,
  disabled,
  label,
  note,
  onSubmit,
}: {
  busy: boolean;
  disabled: boolean;
  label: string;
  note?: string;
  onSubmit: () => void;
}) {
  return (
    <div className="flex items-center gap-3 border-t px-5 py-3">
      <span className="flex-1 text-[12px] text-muted">{note}</span>
      <button
        type="button"
        className="btn btn-primary"
        disabled={disabled || busy}
        onClick={onSubmit}
      >
        {busy ? "Saving…" : label}
      </button>
    </div>
  );
}
