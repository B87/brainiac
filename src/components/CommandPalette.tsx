import { useEffect, useMemo, useRef, useState } from "react";
import { shortPath } from "../lib/format";
import type { AppSnapshot, RepositorySummary } from "../lib/ipc";
import { repoTone } from "../lib/repo";
import { FetchIcon, FolderIcon, GridIcon, PlusIcon, SearchIcon } from "./icons";
import type { View } from "./Sidebar";

type Props = {
  snapshot: AppSnapshot;
  onClose: () => void;
  onView: (view: View) => void;
  onOpenRepository: () => void;
  onNewWorkspace: () => void;
  onFetch: () => void;
  /** The open repository, which Locate Folder… applies to. */
  current: RepositorySummary | null;
  onLocate: (repositoryId: string) => void;
};

type Item = {
  id: string;
  label: string;
  hint: string;
  icon: React.ReactNode;
  action: () => void;
};

export default function CommandPalette({
  snapshot,
  onClose,
  onView,
  onOpenRepository,
  onNewWorkspace,
  onFetch,
  current,
  onLocate,
}: Props) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const items = useMemo<Item[]>(() => {
    const q = query.trim().toLowerCase();
    const match = (...fields: string[]) =>
      !q || fields.some((f) => f.toLowerCase().includes(q));
    const workspaces: Item[] = snapshot.workspaces
      .filter((w) => match(w.name, w.discovery_root ?? ""))
      .map((w) => ({
        id: `w:${w.id}`,
        label: w.name,
        hint: `workspace · ${w.members.length}`,
        icon: <GridIcon size={13} />,
        action: () => onView({ kind: "workspace", id: w.id }),
      }));
    const activity: Item[] = snapshot.workspaces
      .filter((w) => match(`${w.name} activity`))
      .map((w) => ({
        id: `a:${w.id}`,
        label: `${w.name} · Activity`,
        hint: w.unseen_activity ? `${w.unseen_activity} new` : "",
        icon: <GridIcon size={13} />,
        action: () => onView({ kind: "workspace", id: w.id, tab: "activity" }),
      }));
    const repos: Item[] = snapshot.repositories
      .filter((r) => match(r.name, r.display_path))
      .map((r) => ({
        id: `r:${r.id}`,
        label: r.name,
        hint: shortPath(r.display_path),
        icon: <span className="dot" data-state={repoTone(r)} />,
        action: () => onView({ kind: "repository", id: r.id }),
      }));
    const actions: Item[] = [
      {
        id: "all",
        label: "All repositories",
        hint: "",
        icon: <GridIcon size={13} />,
        action: () => onView({ kind: "all" }),
      },
      {
        id: "open",
        label: "Open Repository…",
        hint: "⌘O",
        icon: <FolderIcon size={13} />,
        action: onOpenRepository,
      },
      {
        id: "new",
        label: "New Workspace…",
        hint: "",
        icon: <PlusIcon size={13} />,
        action: onNewWorkspace,
      },
      {
        id: "fetch",
        label: "Fetch Now",
        hint: "remote-tracking refs only",
        icon: <FetchIcon size={13} />,
        action: onFetch,
      },
      ...(current
        ? [
            {
              id: "locate",
              label: "Locate Folder…",
              hint: current.name,
              icon: <FolderIcon size={13} />,
              action: () => onLocate(current.id),
            },
          ]
        : []),
    ].filter((a) => match(a.label));
    return [
      ...repos,
      ...workspaces,
      ...(query.trim() ? activity : []),
      ...actions,
    ];
  }, [
    snapshot,
    query,
    onView,
    onOpenRepository,
    onNewWorkspace,
    onFetch,
    current,
    onLocate,
  ]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") onClose();
    else if (e.key === "ArrowDown")
      setIndex((i) => Math.min(i + 1, items.length - 1));
    else if (e.key === "ArrowUp") setIndex((i) => Math.max(i - 1, 0));
    else if (e.key === "Enter") items[index]?.action();
    else return;
    e.preventDefault();
  };

  return (
    <div className="absolute inset-0 z-30 flex items-start justify-center bg-black/25 pt-24">
      <button
        type="button"
        aria-label="Close command palette"
        className="absolute inset-0"
        onMouseDown={onClose}
        onClick={onClose}
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Switch repository or workspace"
        tabIndex={-1}
        className="relative w-[520px] rounded-xl border border-control-line bg-header shadow-2xl"
        onKeyDown={onKeyDown}
      >
        <div className="flex items-center gap-2 border-b px-3.5 text-muted">
          <SearchIcon size={14} />
          <input
            ref={inputRef}
            className="selectable h-11 flex-1 bg-transparent text-[14px] outline-none"
            placeholder="Switch repository or workspace…"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setIndex(0);
            }}
          />
        </div>
        <div className="max-h-96 overflow-y-auto p-1.5">
          {items.length === 0 && (
            <div className="px-2 py-1 text-muted">No matches.</div>
          )}
          {items.map((it, i) => (
            <button
              type="button"
              key={it.id}
              className="menu-item h-8"
              aria-current={i === index}
              onMouseEnter={() => setIndex(i)}
              onClick={it.action}
            >
              <span className="flex w-4 justify-center">{it.icon}</span>
              <span className="truncate">{it.label}</span>
              <span className="muted-in-menu ml-auto truncate text-[11px] text-muted">
                {it.hint}
              </span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
