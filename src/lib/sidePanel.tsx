/**
 * Whether the right-hand side panel is open (SPEC.md, Keyboard defaults).
 * One choice for every view that has one: the repository preview, a branch
 * or tag's history, today's repositories, activity and pull-request
 * settings, a pull request's merge checklist, and a note's context panel.
 */
import { createContext, type ReactNode, useContext, useMemo } from "react";

const KEY = "brainiac.panel.right";
const LEGACY = "brainiac.notes.context";

// Keep a choice made before the panel was shared across views.
try {
  if (localStorage.getItem(KEY) === null) {
    const legacy = localStorage.getItem(LEGACY);
    if (legacy !== null) localStorage.setItem(KEY, legacy);
  }
} catch {
  // Preference only.
}

export const SIDE_PANEL_KEY = KEY;

export type SidePanel = {
  open: boolean;
  toggle: () => void;
};

const SidePanelContext = createContext<SidePanel | null>(null);

export function SidePanelState({
  open,
  toggle,
  children,
}: SidePanel & { children: ReactNode }) {
  const value = useMemo(() => ({ open, toggle }), [open, toggle]);
  return (
    <SidePanelContext.Provider value={value}>
      {children}
    </SidePanelContext.Provider>
  );
}

export function useSidePanel(): SidePanel {
  const value = useContext(SidePanelContext);
  if (!value) throw new Error("useSidePanel outside the app");
  return value;
}
