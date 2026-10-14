/**
 * xterm.js and its add-ons, in a module of their own so they load the first
 * time a terminal opens, not with the app (SPEC.md, section 15).
 */
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { openUrl } from "@tauri-apps/plugin-opener";

/** The terminal's colors: dark in both of Brainiac's themes, as most terminals are. */
const THEME = {
  background: "#17171b",
  foreground: "#d6d6dc",
  cursor: "#d6d6dc",
  cursorAccent: "#17171b",
  selectionBackground: "#3a4a6b",
  black: "#1d1d22",
  red: "#ff8a80",
  green: "#7fd49b",
  yellow: "#ffb454",
  blue: "#7aa7ff",
  magenta: "#d7a6ff",
  cyan: "#6fd3e0",
  white: "#d6d6dc",
  brightBlack: "#8c8c96",
  brightRed: "#ffb0a8",
  brightGreen: "#a6e8bb",
  brightYellow: "#ffd08a",
  brightBlue: "#a8c6ff",
  brightMagenta: "#e6c6ff",
  brightCyan: "#9ee6ef",
  brightWhite: "#ffffff",
};

function monoFont(): string {
  const font = getComputedStyle(document.documentElement)
    .getPropertyValue("--mono")
    .trim();
  return font || 'ui-monospace, "SF Mono", Menlo, monospace';
}

export function createTerminal(options: {
  optionAsMeta: boolean;
  onLink: (uri: string) => void;
}): { term: Terminal; fit: FitAddon } {
  const term = new Terminal({
    // Needed by the unicode11 add-on.
    allowProposedApi: true,
    fontFamily: monoFont(),
    fontSize: 12.5,
    lineHeight: 1.15,
    scrollback: 10_000,
    theme: THEME,
    macOptionIsMeta: options.optionAsMeta,
    // ⌥-click moves the cursor, as in Terminal; ⌥-drag selects in programs that take the mouse.
    macOptionClickForcesSelection: true,
    cursorBlink: true,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  const unicode = new Unicode11Addon();
  term.loadAddon(unicode);
  term.unicode.activeVersion = "11";
  term.loadAddon(
    new WebLinksAddon((event, uri) => {
      // A plain click selects text; ⌘-click opens, as in Terminal.
      if (event.metaKey || event.ctrlKey) options.onLink(uri);
    }),
  );
  return { term, fit };
}

/**
 * Open `term` in `element`, drawn with WebGL when the WebView allows it, and
 * with xterm.js's own DOM renderer otherwise or when the GPU context is lost.
 */
export function open(term: Terminal, element: HTMLElement) {
  term.open(element);
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => webgl.dispose());
    term.loadAddon(webgl);
  } catch {
    // No WebGL here: the DOM renderer draws it.
  }
}

export function openLink(uri: string) {
  void openUrl(uri).catch(() => {});
}
