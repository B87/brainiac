/**
 * The Terminal view and the dock: two sizes of the same tabs (SPEC.md,
 * section 15). Each shows the active tab's pane; moving between them moves
 * the tab's terminal, so its program keeps running and only sees a resize.
 */
import { useEffect, useRef } from "react";
import { usePref } from "../../lib/prefs";
import { IS_MAC, type TerminalTabState } from "../../lib/terminal";
import { CloseIcon, PlusIcon, TerminalIcon } from "../icons";
import { closeTab } from "./actions";
import { attach, detach, focus } from "./sessions";
import { terminalStore, useTerminals } from "./store";

const MOD = IS_MAC ? "⌘" : "Ctrl+";
const LEAVE = `${MOD}Esc`;
const NEW_TAB = IS_MAC ? "⇧⌘T" : "Ctrl+Shift+T";

type BarProps = {
  place: "view" | "dock";
  onNew: () => void;
  /** The view: back to the view before it, with the dock open. The dock: the Terminal view. */
  onMove: () => void;
  /** The dock only: hide it. */
  onHide?: () => void;
};

function TerminalBar({ place, onNew, onMove, onHide }: BarProps) {
  const { tabs, active } = useTerminals();
  const dock = place === "dock";
  return (
    <div
      role="tablist"
      aria-label="Terminal tabs"
      className={`flex shrink-0 items-end gap-0.5 border-b bg-header px-2.5 ${
        dock ? "h-9" : "h-12 pl-lead"
      }`}
      data-tauri-drag-region={dock ? undefined : true}
    >
      <div
        className="db-tabs"
        onWheel={(e) => {
          if (e.deltaX === 0) e.currentTarget.scrollLeft += e.deltaY;
        }}
      >
        {tabs.map((t) => (
          <div key={t.key} className="flex">
            <button
              type="button"
              role="tab"
              aria-selected={active === t.key}
              className={`db-tab ${dock ? "!h-7" : ""}`}
              title={t.cwd || "Home folder"}
              onClick={() => {
                terminalStore.setActive(t.key);
                focus(t.key);
              }}
              onAuxClick={(e) => {
                if (e.button === 1) void closeTab(t.key);
              }}
            >
              <span className="truncate font-medium">{t.title}</span>
              {t.program && (
                <span className="text-[11px] text-muted">{t.program}</span>
              )}
              {/* biome-ignore lint/a11y/useSemanticElements: a close control inside the tab. */}
              <span
                role="button"
                tabIndex={-1}
                className="close"
                aria-label={`Close ${t.title}`}
                onClick={(e) => {
                  e.stopPropagation();
                  void closeTab(t.key);
                }}
                onKeyDown={() => {}}
              >
                <CloseIcon size={10} />
              </span>
            </button>
          </div>
        ))}
      </div>
      <button
        type="button"
        className="btn btn-ghost icon-btn mb-1"
        aria-label="New terminal"
        title={`New terminal (${NEW_TAB})`}
        onClick={onNew}
      >
        <PlusIcon size={13} />
      </button>
      <div
        className="h-full flex-1"
        data-tauri-drag-region={!dock || undefined}
      />
      <span className="mb-2 hidden whitespace-nowrap text-[11px] text-muted lg:inline">
        {dock && `${MOD}J hide · `}
        {NEW_TAB} new tab · {LEAVE} leave the terminal
      </span>
      <button
        type="button"
        className={`btn mb-1 ml-2 px-2 text-xs ${dock ? "h-6" : "mb-1.5 h-7"}`}
        onClick={onMove}
      >
        {dock ? "Maximize" : "Dock"}
      </button>
      {dock && onHide && (
        <button
          type="button"
          className="btn btn-ghost icon-btn mb-1 ml-1"
          aria-label="Hide the dock"
          title={`Hide the dock (${MOD}J)`}
          onClick={onHide}
        >
          <CloseIcon size={11} />
        </button>
      )}
    </div>
  );
}

/** The active tab's terminal, and what to say over it. */
function TerminalPane({ tab }: { tab: TerminalTabState }) {
  const host = useRef<HTMLDivElement>(null);
  const [hintSeen, setHintSeen] = usePref("terminal.hintSeen", false);
  useEffect(() => {
    const element = host.current;
    if (!element) return;
    void attach(tab.key, element).then(() => focus(tab.key));
    return () => detach(tab.key, element);
  }, [tab.key]);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-[#17171b]">
      {!hintSeen && (
        <div className="flex items-center gap-2.5 border-b border-[#c9d8f5] bg-[#eef3ff] px-3 py-2 text-[#2a3f6b] dark:border-[#2c3a5c] dark:bg-[#1c2438] dark:text-[#c9d8f5]">
          <span className="flex-1">
            {LEAVE} moves between the terminal and the window. Closing a tab
            ends its shell.
          </span>
          <button
            type="button"
            className="btn h-6 px-2 text-xs"
            onClick={() => setHintSeen(true)}
          >
            Got it
          </button>
        </div>
      )}
      {tab.error && (
        <div
          role="alert"
          className="selectable border-b border-red-800 bg-red-950 px-3 py-2 text-red-100"
        >
          {tab.error}
        </div>
      )}
      <div
        ref={host}
        data-terminal-pane
        className="min-h-0 min-w-0 flex-1 overflow-hidden py-1 pl-2"
      />
    </div>
  );
}

/** What the view or the dock shows with no tab open. */
function NoTabs({ onNew, where }: { onNew: () => void; where: string }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
      <TerminalIcon size={22} className="text-muted" />
      <div className="text-muted">No terminal is open.</div>
      <button type="button" className="btn btn-primary" onClick={onNew}>
        New terminal in {where}
      </button>
    </div>
  );
}

export function TerminalView({
  where,
  onNew,
  onDock,
}: {
  /** Where a new tab opens: the repository last selected, or the home folder. */
  where: string;
  onNew: () => void;
  onDock: () => void;
}) {
  const { tabs, active } = useTerminals();
  const tab = tabs.find((t) => t.key === active);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <TerminalBar place="view" onNew={onNew} onMove={onDock} />
      {tab ? (
        <TerminalPane key={tab.key} tab={tab} />
      ) : (
        <NoTabs onNew={onNew} where={where} />
      )}
    </div>
  );
}

export function TerminalDock({
  where,
  side,
  onNew,
  onMaximize,
  onHide,
}: {
  where: string;
  /** Chosen in Settings → Terminal. */
  side: "bottom" | "right";
  onNew: () => void;
  onMaximize: () => void;
  onHide: () => void;
}) {
  const { tabs, active } = useTerminals();
  const tab = tabs.find((t) => t.key === active);
  const [size, setSize] = usePref(
    side === "right" ? "terminal.dockWidth" : "terminal.dockHeight",
    side === "right" ? 520 : 330,
  );
  const dragging = useRef<{ from: number; size: number } | null>(null);
  const along = (e: { clientX: number; clientY: number }) =>
    side === "right" ? e.clientX : e.clientY;
  return (
    <section
      aria-label="Terminal dock"
      className={`relative flex shrink-0 flex-col ${
        side === "right" ? "border-l" : "border-t"
      }`}
      style={side === "right" ? { width: size } : { height: size }}
    >
      <div
        className={`absolute z-10 ${
          side === "right"
            ? "top-0 bottom-0 -left-1 w-2 cursor-col-resize"
            : "-top-1 right-0 left-0 h-2 cursor-row-resize"
        }`}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          dragging.current = { from: along(e), size };
        }}
        onPointerMove={(e) => {
          const drag = dragging.current;
          if (!drag) return;
          const next = drag.size - (along(e) - drag.from);
          setSize(Math.max(160, Math.min(next, side === "right" ? 1400 : 900)));
        }}
        onPointerUp={() => {
          dragging.current = null;
        }}
      />
      <TerminalBar
        place="dock"
        onNew={onNew}
        onMove={onMaximize}
        onHide={onHide}
      />
      {tab ? (
        <TerminalPane key={tab.key} tab={tab} />
      ) : (
        <NoTabs onNew={onNew} where={where} />
      )}
    </section>
  );
}
