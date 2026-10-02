import { type ReactNode, useEffect, useRef } from "react";
import { CloseIcon } from "./icons";

/**
 * A modal dialog over the window: Escape and the backdrop close it, and the
 * keyboard belongs to it while it is open (lib/keys.ts). Focus moves into it
 * when it opens and returns where it was when it closes.
 */
export default function Dialog({
  title,
  onClose,
  width = 520,
  footer,
  children,
}: {
  title: string;
  onClose: () => void;
  width?: number;
  footer?: ReactNode;
  children: ReactNode;
}) {
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const before =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    // A field with autoFocus already has focus; otherwise the dialog takes
    // it, so Escape reaches it even when opened from a menu that closed.
    if (!box.current?.contains(document.activeElement)) box.current?.focus();
    return () => {
      if (before?.isConnected) before.focus();
    };
  }, []);
  return (
    <div className="absolute inset-0 z-30 flex items-start justify-center bg-black/30 pt-20">
      {/* biome-ignore lint/a11y/noStaticElementInteractions: the backdrop is for the mouse; the keyboard closes with Escape or the Close button. */}
      <div role="presentation" className="absolute inset-0" onClick={onClose} />
      <div
        ref={box}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="relative flex max-h-[calc(100%-120px)] flex-col rounded-xl border border-control-line bg-header shadow-2xl outline-none"
        style={{ width }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            onClose();
          }
        }}
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
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {children}
        </div>
        {footer && (
          <div className="flex items-center justify-end gap-2 border-t px-5 py-3">
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}
