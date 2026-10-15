import {
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

export type ContextMenuItem =
  | { separator: true }
  | {
      label: string;
      onSelect: () => void;
      danger?: boolean;
      disabled?: boolean;
    };

type Position = { x: number; y: number };

/**
 * A menu opened by a right-click, drawn at the pointer. `onContextMenu` goes on
 * the row; `menu` goes anywhere in the same component. The window's own menu is
 * blocked in `main.tsx`, and this one takes its place.
 */
export function useContextMenu(items: ContextMenuItem[]) {
  const [at, setAt] = useState<Position | null>(null);
  return {
    onContextMenu: (e: React.MouseEvent) => {
      e.preventDefault();
      setAt({ x: e.clientX, y: e.clientY });
    },
    menu: at ? (
      <ContextMenu at={at} items={items} onClose={() => setAt(null)} />
    ) : null,
  };
}

function ContextMenu({
  at,
  items,
  onClose,
}: {
  at: Position;
  items: ContextMenuItem[];
  onClose: () => void;
}): ReactNode {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(at);

  // Keep the menu inside the window: flip it up or left when it would overflow.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(at.x, window.innerWidth - width - 4)),
      y: Math.max(4, Math.min(at.y, window.innerHeight - height - 4)),
    });
    el.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [at]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const buttons = [
          ...(ref.current?.querySelectorAll<HTMLButtonElement>(
            "button:not(:disabled)",
          ) ?? []),
        ];
        const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const step = e.key === "ArrowDown" ? 1 : -1;
        buttons[(i + step + buttons.length) % buttons.length]?.focus();
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("blur", onClose);
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  // A portal puts the menu on `body`, so a row's overflow or transform cannot clip it.
  return createPortal(
    <div
      ref={ref}
      role="menu"
      data-popover
      className="menu"
      style={{ position: "fixed", left: pos.x, top: pos.y }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((item, i) =>
        "separator" in item ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: separators have no identity
          <div key={i} className="menu-sep" aria-hidden="true" />
        ) : (
          <button
            key={item.label}
            type="button"
            role="menuitem"
            className="menu-item"
            disabled={item.disabled}
            style={
              item.danger ? { color: "var(--conflict, #e5484d)" } : undefined
            }
            onClick={() => {
              onClose();
              item.onSelect();
            }}
          >
            {item.label}
          </button>
        ),
      )}
    </div>,
    document.body,
  );
}
