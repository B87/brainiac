/**
 * Each tab's xterm.js instance, kept outside React so a tab can move
 * between the Terminal view and the dock without restarting: its element is
 * moved into the pane that shows it, and its program only sees a resize
 * (SPEC.md, section 15). xterm.js is loaded the first time a tab opens.
 */
import { errorMessage, ipc } from "../../lib/ipc";
import { IS_MAC, tabKey } from "../../lib/terminal";
import { terminalStore } from "./store";

type Xterm = typeof import("./xterm");

type Session = {
  term: import("@xterm/xterm").Terminal;
  fit: import("@xterm/addon-fit").FitAddon;
  element: HTMLDivElement;
  /** The backend's ID once the program started. */
  id: string | null;
  starting: boolean;
  observer: ResizeObserver;
  frame: number;
};

const sessions = new Map<string, Session>();
let loading: Promise<Xterm> | null = null;

/** What a tab asks of the window: leave it for the rest of the window. */
let onLeave: () => void = () => {};
export function setLeaveHandler(handler: () => void) {
  onLeave = handler;
}

function load(): Promise<Xterm> {
  loading ??= import("./xterm");
  return loading;
}

/**
 * Show tab `key` in `container`, creating its terminal and starting its
 * program the first time. The program starts at the size the pane has.
 */
export async function attach(key: string, container: HTMLElement) {
  const tab = terminalStore.tab(key);
  if (!tab) return;
  const xterm = await load();
  // Another attach may have created it while xterm.js loaded.
  const session = sessions.get(key) ?? create(key, xterm);
  if (!container.isConnected) return;
  if (session.element.parentElement !== container)
    container.appendChild(session.element);
  // Opened once it is in the page, where xterm.js can measure its font.
  if (!session.term.element) xterm.open(session.term, session.element);
  session.observer.disconnect();
  session.observer.observe(container);
  refit(session);
  if (!session.id && !session.starting) void start(key, session);
}

/** The pane that showed `key` went away; its terminal stays alive. */
export function detach(key: string, container: HTMLElement) {
  const session = sessions.get(key);
  if (!session) return;
  if (session.element.parentElement === container) {
    session.observer.disconnect();
    container.removeChild(session.element);
  }
}

export function focus(key: string) {
  sessions.get(key)?.term.focus();
}

/** Whether focus is in a tab's terminal. */
export function hasFocus(): boolean {
  const active = document.activeElement;
  return !!active?.closest?.("[data-terminal-pane]");
}

/** End tab `key`'s program and forget its terminal. */
export function dispose(key: string) {
  const session = sessions.get(key);
  if (!session) return;
  sessions.delete(key);
  cancelAnimationFrame(session.frame);
  session.observer.disconnect();
  if (session.id) void ipc.closeTerminal(session.id).catch(() => {});
  session.term.dispose();
  session.element.remove();
}

/** The program ended: tell the tab's terminal, which stays readable. */
export function ended(key: string, code: number | null) {
  const session = sessions.get(key);
  if (!session) return;
  session.id = null;
  const how = code === null ? "was ended" : `exited with code ${code}`;
  session.term.write(`\r\n\x1b[2m[The program ${how}.]\x1b[0m\r\n`);
}

function create(key: string, xterm: Xterm): Session {
  const element = document.createElement("div");
  element.className = "terminal-host";
  const { term, fit } = xterm.createTerminal({
    optionAsMeta: terminalStore.optionAsMeta(),
    onLink: (uri) => xterm.openLink(uri),
  });
  const session: Session = {
    term,
    fit,
    element,
    id: null,
    starting: false,
    observer: new ResizeObserver(() => refit(session)),
    frame: 0,
  };
  sessions.set(key, session);

  term.attachCustomKeyEventHandler((e) => {
    const meant = tabKey(e, IS_MAC);
    if (meant === null) return true;
    if (meant === "leave" && e.type === "keydown") {
      // Handled here, so the window's own shortcut does not run again.
      e.preventDefault();
      onLeave();
    }
    // Brainiac's shortcuts go on to the window, not to the program.
    return false;
  });
  term.onData((data) => {
    if (session.id) void ipc.writeTerminal(session.id, data).catch(() => {});
  });
  term.onBinary((data) => {
    if (!session.id) return;
    const bytes = Array.from(data, (c) => c.charCodeAt(0) & 0xff);
    void ipc.writeTerminalBytes(session.id, bytes).catch(() => {});
  });
  term.onResize(({ cols, rows }) => {
    if (session.id)
      void ipc.resizeTerminal(session.id, cols, rows).catch(() => {});
  });
  return session;
}

async function start(key: string, session: Session) {
  const tab = terminalStore.tab(key);
  if (!tab) return;
  session.starting = true;
  try {
    const started = await ipc.openTerminal(
      { cwd: tab.cwd, cols: session.term.cols, rows: session.term.rows },
      (bytes) => session.term.write(bytes),
    );
    if (!sessions.has(key)) {
      // The tab closed while its program started.
      void ipc.closeTerminal(started.id).catch(() => {});
      return;
    }
    session.id = started.id;
    terminalStore.update(key, {
      id: started.id,
      program: started.program,
      cwd: started.cwd,
    });
    // The pane may have changed size while the program started.
    void ipc
      .resizeTerminal(started.id, session.term.cols, session.term.rows)
      .catch(() => {});
  } catch (e) {
    terminalStore.update(key, { error: errorMessage(e) });
  } finally {
    session.starting = false;
  }
}

/** Fit the terminal to its pane, at most once a frame. */
function refit(session: Session) {
  cancelAnimationFrame(session.frame);
  session.frame = requestAnimationFrame(() => {
    const host = session.element.parentElement;
    // A hidden pane has no size; fitting it would shrink the program to nothing.
    if (!host || host.clientWidth === 0 || host.clientHeight === 0) return;
    try {
      session.fit.fit();
    } catch {
      // Not yet open; the next resize fits it.
    }
  });
}
