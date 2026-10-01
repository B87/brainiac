import { useEffect, useMemo, useRef, useState } from "react";
import { shortPath } from "../lib/format";
import type { AppSnapshot } from "../lib/ipc";

type Props = {
  snapshot: AppSnapshot;
  onClose: () => void;
  onSelect: (id: string) => void;
  onAdd: () => void;
};

type Item = { id: string; label: string; hint: string; action: () => void };

export default function CommandPalette({
  snapshot,
  onClose,
  onSelect,
  onAdd,
}: Props) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const items = useMemo<Item[]>(() => {
    const q = query.trim().toLowerCase();
    const repos: Item[] = snapshot.repositories
      .filter(
        (r) =>
          !q ||
          r.name.toLowerCase().includes(q) ||
          r.display_path.toLowerCase().includes(q),
      )
      .map((r) => ({
        id: r.id,
        label: r.name,
        hint: shortPath(r.display_path),
        action: () => onSelect(r.id),
      }));
    const add: Item = {
      id: "__add",
      label: "Open Repository…",
      hint: "⌘O",
      action: onAdd,
    };
    return !q || "open repository".includes(q) ? [...repos, add] : repos;
  }, [snapshot, query, onSelect, onAdd]);

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
    <div className="absolute inset-0 z-10 flex items-start justify-center bg-black/20 pt-24">
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
        aria-label="Switch repository"
        tabIndex={-1}
        className="pane relative w-[480px] rounded-lg border shadow-xl"
        onKeyDown={onKeyDown}
      >
        <input
          ref={inputRef}
          className="selectable w-full rounded-t-lg border-b bg-transparent px-3 py-2 outline-none"
          placeholder="Switch repository…"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setIndex(0);
          }}
        />
        <div className="max-h-80 overflow-y-auto p-1">
          {items.length === 0 && (
            <div className="muted px-2 py-1">No matches.</div>
          )}
          {items.map((it, i) => (
            <button
              type="button"
              key={it.id}
              className="row w-full text-left"
              aria-pressed={i === index}
              onMouseEnter={() => setIndex(i)}
              onClick={it.action}
            >
              <span className="truncate">{it.label}</span>
              <span className="muted ml-auto truncate text-[11px]">
                {it.hint}
              </span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
