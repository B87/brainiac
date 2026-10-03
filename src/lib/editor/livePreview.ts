/**
 * The note editor: CodeMirror 6 editing the note's Markdown text. Live Preview
 * draws formatting over that text with decorations (hidden markup, bullets,
 * checkboxes, rules, images); Source mode is the same editor with those turned
 * off. Decorations never change the document, so neither mode can rewrite a
 * note. What to draw is decided in `preview.ts`; this file only turns it into
 * CodeMirror decorations, widgets, and event handlers.
 */
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import {
  HighlightStyle,
  syntaxHighlighting,
  syntaxTree,
} from "@codemirror/language";
import {
  Compartment,
  EditorSelection,
  EditorState,
  type Extension,
  Facet,
  type Range,
  type SelectionRange,
} from "@codemirror/state";
import {
  type Command,
  Decoration,
  type DecorationSet,
  drawSelection,
  EditorView,
  keymap,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { keepLineEndings, noteText } from "./lineEndings";
import { markdownKeys, noteMarkdown } from "./markdown";
import {
  activeLines,
  type LinkKind,
  linkAt,
  previewItems,
  taskToggle,
} from "./preview";

/** A clicked link: a web address, or another note by Markdown link or wikilink. */
export type NoteLink = { kind: LinkKind; target: string; wiki?: boolean };

export type NoteEditorOptions = {
  livePreview: boolean;
  /** Called after every edit with the note's text, line endings included. */
  onChange?: (text: string) => void;
  /** Called when the cursor moves to another line, with its number (from 1). */
  onCursorLine?: (line: number) => void;
  /** `Cmd`+click on a link. */
  onOpenLink?: (link: NoteLink) => void;
  /** The URL to draw a vault image from, or null to leave its Markdown as text
   * (web images, missing files). */
  resolveImage?: (src: string) => string | null;
  /** Show the note without letting it be edited (previews). */
  readOnly?: boolean;
};

const livePreviewOn = Facet.define<boolean, boolean>({
  combine: (values) => values.some(Boolean),
});
const mode = new Compartment();

class BulletWidget extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const dot = document.createElement("span");
    dot.className = "cm-md-bullet";
    dot.textContent = "•";
    return dot;
  }
}

class CheckboxWidget extends WidgetType {
  constructor(readonly checked: boolean) {
    super();
  }
  eq(other: CheckboxWidget) {
    return other.checked === this.checked;
  }
  toDOM() {
    const box = document.createElement("input");
    box.type = "checkbox";
    box.className = "cm-md-checkbox";
    box.checked = this.checked;
    box.tabIndex = -1;
    return box;
  }
  // The editor's mousedown handler toggles the text; the box only draws it.
  ignoreEvent() {
    return false;
  }
}

class RuleWidget extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const rule = document.createElement("span");
    rule.className = "cm-md-rule";
    return rule;
  }
}

class ImageWidget extends WidgetType {
  constructor(
    readonly url: string,
    readonly alt: string,
  ) {
    super();
  }
  eq(other: ImageWidget) {
    return other.url === this.url && other.alt === this.alt;
  }
  toDOM(view: EditorView) {
    const img = document.createElement("img");
    img.className = "cm-md-image";
    img.src = this.url;
    img.alt = this.alt;
    // A loaded image changes the line's height; have CodeMirror measure it.
    img.addEventListener("load", () => view.requestMeasure());
    return img;
  }
}

const bullet = Decoration.replace({ widget: new BulletWidget() });
const rule = Decoration.replace({ widget: new RuleWidget() });
const hidden = Decoration.replace({});
const link = Decoration.mark({ class: "cm-md-link" });

function previewPlugin(resolveImage: (src: string) => string | null) {
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet = Decoration.none;
      atomic: DecorationSet = Decoration.none;
      lines = "";

      constructor(view: EditorView) {
        this.build(view);
      }

      update(u: ViewUpdate) {
        const lines = [...activeLines(u.state)].join();
        if (
          u.docChanged ||
          u.viewportChanged ||
          (u.selectionSet && lines !== this.lines) ||
          syntaxTree(u.startState) !== syntaxTree(u.state) ||
          u.startState.facet(livePreviewOn) !== u.state.facet(livePreviewOn)
        )
          this.build(u.view);
      }

      build(view: EditorView) {
        const { state } = view;
        this.lines = [...activeLines(state)].join();
        const ranges: Range<Decoration>[] = [];
        const replaced: Range<Decoration>[] = [];
        const replace = (d: Decoration, from: number, to: number) => {
          ranges.push(d.range(from, to));
          replaced.push(d.range(from, to));
        };
        const items = previewItems(
          state,
          state.facet(livePreviewOn),
          view.visibleRanges,
        );
        for (const item of items) {
          switch (item.type) {
            case "line":
              ranges.push(
                Decoration.line({ class: item.className }).range(item.from),
              );
              break;
            case "mark":
              ranges.push(
                Decoration.mark({ class: item.className }).range(
                  item.from,
                  item.to,
                ),
              );
              break;
            case "link":
              if (item.from < item.to)
                ranges.push(link.range(item.from, item.to));
              break;
            case "hide":
              replace(hidden, item.from, item.to);
              break;
            case "bullet":
              replace(bullet, item.from, item.to);
              break;
            case "rule":
              replace(rule, item.from, item.to);
              break;
            case "checkbox":
              replace(
                Decoration.replace({
                  widget: new CheckboxWidget(item.checked),
                }),
                item.from,
                item.to,
              );
              break;
            case "image": {
              const url = resolveImage(item.src);
              if (!url) break;
              const widget = new ImageWidget(url, item.alt);
              if (item.replace)
                replace(Decoration.replace({ widget }), item.from, item.to);
              else
                ranges.push(
                  Decoration.widget({ widget, side: 1 }).range(item.to),
                );
              break;
            }
          }
        }
        this.decorations = Decoration.set(ranges, true);
        this.atomic = Decoration.set(replaced, true);
      }
    },
    {
      decorations: (plugin) => plugin.decorations,
      // The cursor steps over hidden markup and widgets as one unit.
      provide: (plugin) =>
        EditorView.atomicRanges.of(
          (view) => view.plugin(plugin)?.atomic ?? Decoration.none,
        ),
      eventHandlers: {
        mousedown(event, view) {
          const target = event.target;
          if (
            !(target instanceof HTMLInputElement) ||
            !target.classList.contains("cm-md-checkbox")
          )
            return false;
          if (view.state.readOnly) return false;
          const changes = taskToggle(view.state, view.posAtDOM(target));
          if (!changes) return false;
          event.preventDefault();
          view.dispatch({ changes, userEvent: "input.toggle" });
          return true;
        },
      },
    },
  );
}

/**
 * Keeps the caret on screen when content above it changes height after it was
 * shown: an image finishing loading, or lines measured for the first time.
 * CodeMirror keeps the top of the view steady, which pushes a caret near the
 * bottom out of sight. A caret the user scrolled away from is left alone.
 */
const caretFollow = ViewPlugin.fromClass(
  class {
    visible = true;
    /** The caret's place at the last measurement, to tell a scroll from
     * content changing height. */
    last: CaretPlace | null = null;

    constructor(readonly view: EditorView) {}

    update(u: ViewUpdate) {
      if (u.transactions.some((tr) => tr.scrollIntoView)) {
        this.visible = true;
        this.view.requestMeasure({ read: () => this.measure() });
      } else if (u.selectionSet || u.docChanged) this.check();
      else if (u.heightChanged) this.follow();
    }

    measure() {
      this.last = caretPlace(this.view);
      return this.last;
    }

    check() {
      this.view.requestMeasure({
        read: () => this.measure(),
        write: (place) => {
          this.visible = place !== null && overflow(place) === 0;
        },
      });
    }

    /** A caret off screen after a scroll was scrolled away from only if it
     * was on screen before and the scroll alone would have taken it off.
     * Otherwise content above it changed height meanwhile, and `follow`
     * brings it back. */
    scrolled() {
      this.view.requestMeasure({
        read: () => {
          const before = this.last;
          const now = this.measure();
          if (!now) return false;
          if (overflow(now) === 0) return true;
          if (!before) return false;
          // Already off screen: a move not yet scrolled into view, or
          // content still settling.
          if (overflow(before) !== 0) return null;
          const moved = now.scrollTop - before.scrollTop;
          const scrolledOnly = {
            ...now,
            top: before.top - moved,
            bottom: before.bottom - moved,
          };
          return overflow(scrolledOnly) === 0 ? null : false;
        },
        write: (visible) => {
          if (visible !== null) this.visible = visible;
        },
      });
    }

    follow() {
      if (
        !this.visible ||
        this.view.root.activeElement !== this.view.contentDOM
      )
        return;
      this.view.requestMeasure({
        read: () => this.measure(),
        write: (place) => {
          const over = place ? overflow(place) : 0;
          if (over) this.view.scrollDOM.scrollTop += over + Math.sign(over) * 4;
        },
      });
    }
  },
  {
    eventHandlers: {
      scroll() {
        this.scrolled();
      },
    },
  },
);

/** The caret's top and bottom relative to the scroller's top edge. */
type CaretPlace = {
  top: number;
  bottom: number;
  height: number;
  scrollTop: number;
};

function caretPlace(view: EditorView): CaretPlace | null {
  const caret = view.coordsAtPos(view.state.selection.main.head);
  if (!caret) return null;
  const box = view.scrollDOM.getBoundingClientRect();
  return {
    top: caret.top - box.top,
    bottom: caret.bottom - box.top,
    height: box.height,
    scrollTop: view.scrollDOM.scrollTop,
  };
}

/** How far the caret is above (negative) or below (positive) the screen. */
function overflow(p: CaretPlace): number {
  if (p.top < 0) return p.top;
  return Math.max(0, p.bottom - p.height);
}

/**
 * Up and Down move one line at a time. CodeMirror moves by pixels, so it can
 * jump over a line that Live Preview draws as nothing but a tall image; then
 * the cursor lands at the edge of the skipped line instead.
 */
function lineStep(view: EditorView, range: SelectionRange, forward: boolean) {
  const moved = view.moveVertically(range, forward);
  const doc = view.state.doc;
  const from = doc.lineAt(range.head).number;
  if (Math.abs(doc.lineAt(moved.head).number - from) <= 1) return moved;
  const line = doc.line(from + (forward ? 1 : -1));
  return EditorSelection.cursor(forward ? line.from : line.to);
}

function moveByLine(forward: boolean, extend: boolean): Command {
  return (view) => {
    const { selection } = view.state;
    const next = EditorSelection.create(
      selection.ranges.map((range) => {
        if (extend) {
          const moved = lineStep(view, range, forward);
          return EditorSelection.range(
            range.anchor,
            moved.head,
            moved.goalColumn,
          );
        }
        if (!range.empty)
          return EditorSelection.cursor(forward ? range.to : range.from);
        return lineStep(view, range, forward);
      }),
      selection.mainIndex,
    );
    // No movement (first or last line): let the default keymap handle it.
    if (next.eq(selection)) return false;
    view.dispatch({
      selection: next,
      scrollIntoView: true,
      userEvent: "select",
    });
    return true;
  };
}

const lineKeys = [
  {
    key: "ArrowDown",
    run: moveByLine(true, false),
    shift: moveByLine(true, true),
  },
  {
    key: "ArrowUp",
    run: moveByLine(false, false),
    shift: moveByLine(false, true),
  },
];

function linkClicks(onOpenLink?: (link: NoteLink) => void): Extension {
  if (!onOpenLink) return [];
  return EditorView.domEventHandlers({
    mousedown(event, view) {
      if (!event.metaKey) return false;
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      const found = pos === null ? null : linkAt(view.state, pos);
      if (!found) return false;
      event.preventDefault();
      onOpenLink(found);
      return true;
    },
  });
}

const noteHighlight = HighlightStyle.define([
  { tag: tags.heading1, fontSize: "1.6em", fontWeight: "700" },
  { tag: tags.heading2, fontSize: "1.3em", fontWeight: "700" },
  { tag: tags.heading3, fontSize: "1.15em", fontWeight: "600" },
  { tag: [tags.heading4, tags.heading5, tags.heading6], fontWeight: "600" },
  { tag: tags.strong, fontWeight: "700" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.strikethrough, textDecoration: "line-through" },
  { tag: tags.monospace, fontFamily: "var(--mono)", fontSize: "0.9em" },
  { tag: tags.quote, color: "var(--fg-2)" },
  // Markup and link targets: visible but dimmed in Source mode.
  {
    tag: [
      tags.processingInstruction,
      tags.url,
      tags.labelName,
      tags.contentSeparator,
    ],
    color: "var(--faint)",
  },
]);

const noteTheme = EditorView.theme({
  "&": { fontSize: "15px", color: "var(--fg)", backgroundColor: "transparent" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "inherit", lineHeight: "1.6" },
  ".cm-content": { padding: "24px 0", caretColor: "var(--fg)" },
  ".cm-md-heading": { paddingTop: "0.5em" },
  ".cm-md-codeblock, .cm-md-table": {
    fontFamily: "var(--mono)",
    fontSize: "13px",
  },
  ".cm-md-codeblock": { backgroundColor: "var(--panel)" },
  ".cm-md-frontmatter": {
    fontFamily: "var(--mono)",
    fontSize: "12px",
    color: "var(--faint)",
  },
  ".cm-md-quote": { borderLeft: "3px solid var(--line)", paddingLeft: "12px" },
  // Inner highlight spans (a bare URL is tagged as a dimmed target) take the
  // link color too.
  ".cm-md-link, .cm-md-link span": {
    color: "var(--link)",
    textDecoration: "underline",
    textUnderlineOffset: "2px",
  },
  ".cm-md-done": { color: "var(--muted)", textDecoration: "line-through" },
  ".cm-md-bullet": { color: "var(--muted)" },
  ".cm-md-checkbox": {
    margin: "0 2px 0 0",
    verticalAlign: "-2px",
    cursor: "pointer",
  },
  ".cm-md-rule": {
    display: "inline-block",
    width: "100%",
    verticalAlign: "middle",
    borderTop: "1px solid var(--line)",
  },
  ".cm-md-image": {
    display: "block",
    maxWidth: "100%",
    maxHeight: "480px",
    margin: "4px 0",
    borderRadius: "4px",
  },
});

export function createNoteState(
  text: string,
  options: NoteEditorOptions,
): EditorState {
  const { onChange, onCursorLine } = options;
  let cursorLine = 0;
  return EditorState.create({
    doc: text,
    extensions: [
      keepLineEndings(text),
      noteMarkdown(),
      history(),
      drawSelection(),
      EditorView.lineWrapping,
      keymap.of([
        ...markdownKeys,
        ...lineKeys,
        ...defaultKeymap,
        ...historyKeymap,
        indentWithTab,
      ]),
      syntaxHighlighting(noteHighlight),
      mode.of(livePreviewOn.of(options.livePreview)),
      previewPlugin(options.resolveImage ?? (() => null)),
      linkClicks(options.onOpenLink),
      options.readOnly
        ? [EditorState.readOnly.of(true), EditorView.editable.of(false)]
        : [],
      caretFollow,
      onChange
        ? EditorView.updateListener.of((u) => {
            if (u.docChanged) onChange(noteText(u.state));
          })
        : [],
      onCursorLine
        ? EditorView.updateListener.of((u) => {
            if (!u.selectionSet && !u.docChanged) return;
            const line = u.state.doc.lineAt(u.state.selection.main.head).number;
            if (line === cursorLine) return;
            cursorLine = line;
            onCursorLine(line);
          })
        : [],
      noteTheme,
    ],
  });
}

export function isLivePreview(state: EditorState): boolean {
  return state.facet(livePreviewOn);
}

/** Switch modes, keeping the selection and the text at the top of the view. */
export function setLivePreview(view: EditorView, on: boolean) {
  if (isLivePreview(view.state) === on) return;
  view.dispatch({
    effects: [mode.reconfigure(livePreviewOn.of(on)), view.scrollSnapshot()],
  });
}
