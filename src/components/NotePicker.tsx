import { useEffect, useRef, useState } from "react";
import { ipc, type SearchHit } from "../lib/ipc";
import { createLatest } from "../lib/stale";
import { CloseIcon, NoteIcon } from "./icons";

/** Choose a note by searching its title or text. */
export default function NotePicker({
  value,
  onChange,
}: {
  /** The chosen note, with the title to show. */
  value: { id: string; title: string } | null;
  onChange: (note: { id: string; title: string } | null) => void;
}) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [index, setIndex] = useState(0);
  const latest = useRef(createLatest()).current;

  useEffect(() => {
    const q = query.trim();
    if (!q) {
      latest.cancel();
      setHits([]);
      return;
    }
    const t = setTimeout(() => {
      void latest.run(
        () => ipc.search({ query: q, kinds: ["note"], limit: 8 }),
        (r) => {
          setHits(r.notes.hits);
          setIndex(0);
        },
      );
    }, 120);
    return () => clearTimeout(t);
  }, [query, latest]);

  if (value)
    return (
      <div className="flex items-center gap-2">
        <span className="ref-chip h-7 max-w-[360px] text-[12.5px]">
          <NoteIcon size={12} />
          <span>{value.title}</span>
        </span>
        <button
          type="button"
          className="btn btn-sm btn-ghost px-1.5"
          aria-label="Remove the linked note"
          onClick={() => onChange(null)}
        >
          <CloseIcon />
        </button>
      </div>
    );

  const pick = (hit: SearchHit) => {
    onChange({ id: hit.id, title: hit.title.map((p) => p.text).join("") });
    setQuery("");
    setHits([]);
  };

  return (
    <div className="relative">
      <input
        className="text-input w-full"
        placeholder="Search notes to link…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown")
            setIndex((i) => Math.min(i + 1, hits.length - 1));
          else if (e.key === "ArrowUp") setIndex((i) => Math.max(i - 1, 0));
          else if (e.key === "Enter" && hits[index]) pick(hits[index]);
          else return;
          e.preventDefault();
        }}
      />
      {hits.length > 0 && (
        <div className="menu top-full left-0 mt-1 w-full">
          {hits.map((h, i) => (
            <button
              key={h.id}
              type="button"
              className="menu-item"
              aria-current={i === index}
              onMouseEnter={() => setIndex(i)}
              onClick={() => pick(h)}
            >
              <NoteIcon size={12} />
              <span className="truncate">
                {h.title.map((p) => p.text).join("")}
              </span>
              <span className="muted-in-menu ml-auto truncate text-[11px] text-muted">
                {h.detail}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
