import { type ReactNode, useEffect, useRef } from "react";

/**
 * Anchored menu that closes on Escape or a click outside. Render it inside a
 * `relative` wrapper next to the button that opens it.
 */
export default function Popover({
  onClose,
  align = "left",
  className = "",
  children,
}: {
  onClose: () => void;
  align?: "left" | "right";
  className?: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.parentElement?.contains(e.target as Node))
        onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);
  return (
    <div
      ref={ref}
      role="menu"
      className={`menu top-full mt-1 ${align === "right" ? "right-0" : "left-0"} ${className}`}
    >
      {children}
    </div>
  );
}
