import { useEffect, useState } from "react";
import { SidebarIcon } from "./icons";

/** Whether a folded list is showing over the patch; Escape folds it again. */
export function useOverlay(): [boolean, (open: boolean) => void] {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);
  return [open, setOpen];
}

/**
 * A folded list on the left of a patch: a narrow strip with its name written
 * down it. Clicking shows the list again (SPEC.md, Main window).
 */
export default function FoldStrip({
  label,
  title,
  expanded,
  onClick,
}: {
  label: string;
  title: string;
  /** The list is showing over the patch. */
  expanded?: boolean;
  onClick: () => void;
}) {
  return (
    <div className="flex w-11 shrink-0 flex-col items-center border-r bg-panel pt-2.5">
      <button
        type="button"
        className="btn btn-sm h-auto w-8 flex-col gap-1.5 px-0 py-2"
        aria-expanded={expanded}
        title={title}
        onClick={onClick}
      >
        <SidebarIcon size={13} />
        <span className="text-[11.5px] [writing-mode:vertical-rl]">
          {label}
        </span>
      </button>
    </div>
  );
}

/** Where the list sits: in the row, folded away, or over the patch. */
export function listClass(state: "shown" | "folded" | "floating"): string {
  return state === "folded"
    ? "hidden"
    : state === "floating"
      ? "absolute inset-y-0 left-11 z-20 flex flex-col border-r bg-panel shadow-xl"
      : "relative flex shrink-0 flex-col border-r bg-panel";
}
