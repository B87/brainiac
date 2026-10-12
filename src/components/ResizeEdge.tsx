import { useEffect, useRef, useState } from "react";
import { usePref } from "../lib/prefs";

/** A pane's width: where it starts, how narrow it may get, how wide. */
export type PaneBounds = {
  initial: number;
  min: number;
  /** Read when it is needed, so a narrower window leaves the rest room. */
  max: () => number;
};

/** At most this share of the window, and never past `cap` pixels. */
export function shareOfWindow(share: number, cap: number, min: number) {
  return () =>
    Math.max(min, Math.min(cap, Math.round(window.innerWidth * share)));
}

/** The list on the left of History and Changes. */
export const LIST_BOUNDS: PaneBounds = {
  initial: 360,
  min: 240,
  max: shareOfWindow(0.5, 720, 240),
};

export type PaneWidth = {
  width: number;
  bounds: PaneBounds;
  /** The width while dragging, not yet remembered; null when the drag ends. */
  onLive: (w: number | null) => void;
  onDone: (w: number) => void;
};

/**
 * A pane's width, dragged by a `ResizeEdge` and remembered under `key`. A
 * width saved in a wider window is clamped to what this window allows.
 */
export function usePaneWidth(key: string, bounds: PaneBounds): PaneWidth {
  const [saved, setSaved] = usePref(key, bounds.initial);
  const [live, setLive] = useState<number | null>(null);
  // The maximum follows the window: draw again when it is resized, so a
  // width saved in a wider window shrinks with it.
  const [, redraw] = useState(0);
  useEffect(() => {
    const onResize = () => redraw((n) => n + 1);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  const clamp = (w: number) =>
    Math.round(Math.min(bounds.max(), Math.max(bounds.min, w)));
  return {
    width: live ?? clamp(saved),
    bounds,
    onLive: setLive,
    onDone: (w) => setSaved(clamp(w)),
  };
}

/**
 * The draggable edge of a pane (its parent must be `relative`): drag to
 * resize, arrow keys when focused, double-click for the starting width.
 * `side` is the edge it sits on: "right" for a list on the left, "left" for
 * a panel on the right.
 */
export default function ResizeEdge({
  pane,
  side,
  label,
}: {
  pane: PaneWidth;
  side: "left" | "right";
  label: string;
}) {
  const { width, bounds, onLive, onDone } = pane;
  const drag = useRef<{ x: number; w: number } | null>(null);
  const clamp = (w: number) =>
    Math.round(Math.min(bounds.max(), Math.max(bounds.min, w)));
  // A panel on the right grows as the pointer moves left.
  const widthAt = (x: number) => {
    const d = drag.current;
    if (!d) return width;
    return clamp(side === "right" ? d.w + x - d.x : d.w + d.x - x);
  };
  return (
    // biome-ignore lint/a11y/useSemanticElements: an <hr> cannot be dragged or focused; this is a focusable splitter.
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      aria-valuemin={bounds.min}
      aria-valuemax={bounds.max()}
      tabIndex={0}
      title="Drag to resize; double-click for the default width"
      className={`absolute top-0 bottom-0 z-10 w-[6px] cursor-col-resize hover:bg-accent/40 focus-visible:bg-accent/60 focus-visible:outline-none ${side === "right" ? "-right-[3px]" : "-left-[3px]"}`}
      onPointerDown={(e) => {
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { x: e.clientX, w: width };
      }}
      onPointerMove={(e) => {
        if (drag.current) onLive(widthAt(e.clientX));
      }}
      onPointerUp={(e) => {
        if (!drag.current) return;
        const w = widthAt(e.clientX);
        drag.current = null;
        onLive(null);
        onDone(w);
      }}
      // The system took the pointer (a gesture, a window change): drop the
      // drag without saving its width.
      onPointerCancel={() => {
        if (!drag.current) return;
        drag.current = null;
        onLive(null);
      }}
      onDoubleClick={() => onDone(bounds.initial)}
      onKeyDown={(e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        e.preventDefault();
        e.stopPropagation();
        const wider = (e.key === "ArrowRight") === (side === "right");
        onDone(clamp(width + (wider ? 24 : -24)));
      }}
    />
  );
}
