import { PanelRightIcon } from "./icons";

/** Shows or hides the right-hand side panel (⌥⌘B). */
export default function SidePanelButton({
  open,
  onToggle,
  name = "side panel",
}: {
  open: boolean;
  onToggle: () => void;
  /** Accessible name, as in "Hide side panel". */
  name?: string;
}) {
  const titled = name.charAt(0).toUpperCase() + name.slice(1);
  return (
    <button
      type="button"
      className="btn btn-ghost icon-btn"
      aria-label={open ? `Hide ${name}` : `Show ${name}`}
      aria-pressed={open}
      title={`${titled} (⌥⌘B)`}
      onClick={onToggle}
    >
      <PanelRightIcon />
    </button>
  );
}
