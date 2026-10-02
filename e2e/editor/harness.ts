/**
 * The note editor alone in a page, for the WebKit tests in `editor.spec.ts`.
 * Tests drive it with real keyboard and mouse input and read its state through
 * `window.ed`, never through CodeMirror directly.
 */
import { EditorView } from "@codemirror/view";
import { noteText } from "../../src/lib/editor/lineEndings";
import {
  createNoteState,
  isLivePreview,
  type NoteLink,
  setLivePreview,
} from "../../src/lib/editor/livePreview";

const parent = document.getElementById("editor") as HTMLElement;
let view: EditorView | null = null;
const opened: NoteLink[] = [];

function current(): EditorView {
  if (!view) throw new Error("No note loaded");
  return view;
}

const frame = () =>
  new Promise<void>((resolve) =>
    requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
  );

const harness = {
  opened,
  frame,

  /** Load a note; images under `attachments/` come from this folder, web
   * images are left as text. */
  async load(text: string, live = true) {
    view?.destroy();
    view = new EditorView({
      parent,
      state: createNoteState(text, {
        livePreview: live,
        resolveImage: (src) =>
          /^[a-z][a-z0-9+.-]*:/i.test(src)
            ? null
            : `attachments/${src.split("/").pop()}`,
        onOpenLink: (link) => opened.push(link),
      }),
    });
    view.focus();
    await frame();
  },

  text: () => noteText(current().state),
  lines: () => current().state.doc.lines,
  isLive: () => isLivePreview(current().state),

  async setLive(on: boolean) {
    setLivePreview(current(), on);
    await frame();
  },

  selection() {
    const { anchor, head } = current().state.selection.main;
    return { anchor, head };
  },

  /** The line number of the cursor. */
  line() {
    const v = current();
    return v.state.doc.lineAt(v.state.selection.main.head).number;
  },

  async select(anchor: number) {
    const v = current();
    v.dispatch({ selection: { anchor }, scrollIntoView: true });
    v.focus();
    await frame();
  },

  /** Bring a position on screen without moving the cursor. */
  async reveal(pos: number) {
    current().dispatch({
      effects: EditorView.scrollIntoView(pos, { y: "center" }),
    });
    await frame();
  },

  /** Viewport coordinates of the character after `pos`. */
  coords(pos: number) {
    const c = current().coordsAtPos(pos);
    return c && { x: (c.left + c.right) / 2 + 1, y: (c.top + c.bottom) / 2 };
  },

  /** What a copy puts on the clipboard. */
  copy() {
    const data = new DataTransfer();
    current().contentDOM.dispatchEvent(
      new ClipboardEvent("copy", {
        clipboardData: data,
        bubbles: true,
        cancelable: true,
      }),
    );
    return data.getData("text/plain");
  },

  caretOnScreen() {
    const v = current();
    const caret = v.coordsAtPos(v.state.selection.main.head);
    const box = v.scrollDOM.getBoundingClientRect();
    return (
      !!caret && caret.top >= box.top - 1 && caret.bottom <= box.bottom + 1
    );
  },

  /** The number of the first line at the top of the view. */
  topLine() {
    const v = current();
    const block = v.lineBlockAtHeight(
      v.scrollDOM.scrollTop -
        v.documentTop +
        v.scrollDOM.getBoundingClientRect().top,
    );
    return v.state.doc.lineAt(block.from).number;
  },

  /** Scroll by `px`; true when the view could not move any further. */
  async scrollBy(px: number) {
    const scroller = current().scrollDOM;
    const before = scroller.scrollTop;
    scroller.scrollTop += px;
    await frame();
    return scroller.scrollTop === before;
  },

  /** Median milliseconds per typed character: the editor's own work, and
   * until the next frame. */
  async typingCost(samples = 40) {
    const v = current();
    const work: number[] = [];
    const shown: number[] = [];
    for (let i = 0; i < samples; i++) {
      const start = performance.now();
      v.dispatch(v.state.replaceSelection("x"));
      v.contentDOM.getBoundingClientRect();
      work.push(performance.now() - start);
      await new Promise((resolve) => requestAnimationFrame(resolve));
      shown.push(performance.now() - start);
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
    const median = (xs: number[]) =>
      [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
    return { work: median(work), shown: median(shown) };
  },
};

export type Harness = typeof harness;

declare global {
  interface Window {
    ed: Harness;
  }
}

window.ed = harness;
