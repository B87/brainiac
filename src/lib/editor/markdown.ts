/**
 * The Markdown dialect the note editor parses: CommonMark, GFM (tables, task
 * lists, strikethrough, autolinks), YAML frontmatter, and [[wikilinks]]. The
 * parse only decides how text is drawn; it never changes the note's text.
 */
import {
  commonmarkLanguage,
  deleteMarkupBackward,
  insertNewlineContinueMarkup,
  markdown,
} from "@codemirror/lang-markdown";
import type { LanguageSupport } from "@codemirror/language";
import type { ChangeSpec, StateCommand } from "@codemirror/state";
import { styleTags, tags } from "@lezer/highlight";
import { GFM, type MarkdownConfig } from "@lezer/markdown";

const OPEN = /^---\s*$/;
const CLOSE = /^(---|\.\.\.)\s*$/;

/**
 * Frontmatter: a first line of `---`, up to the next `---` or `...` line.
 * Without a closing line nothing is frontmatter, but the lines are consumed
 * and stay unstyled until the user types it (the parser cannot look further
 * ahead than one line).
 */
export const Frontmatter: MarkdownConfig = {
  defineNodes: [{ name: "Frontmatter", block: true }, "FrontmatterMark"],
  parseBlock: [
    {
      name: "Frontmatter",
      before: "HorizontalRule",
      parse(cx, line) {
        if (cx.lineStart !== 0 || !OPEN.test(line.text)) return false;
        const marks = [cx.elt("FrontmatterMark", 0, line.text.length)];
        while (cx.nextLine()) {
          if (CLOSE.test(line.text)) {
            const end = cx.lineStart + line.text.length;
            marks.push(cx.elt("FrontmatterMark", cx.lineStart, end));
            cx.nextLine();
            cx.addElement(cx.elt("Frontmatter", 0, end, marks));
            return true;
          }
        }
        return true;
      },
    },
  ],
  props: [styleTags({ FrontmatterMark: tags.processingInstruction })],
};

/**
 * `[[target]]` and `[[target|label]]`. An embed (`![[file]]`) is left as text:
 * Brainiac does not render embeds.
 */
export const WikiLink: MarkdownConfig = {
  defineNodes: ["WikiLink", "WikiLinkMark", "WikiLinkTarget"],
  parseInline: [
    {
      name: "WikiLink",
      before: "Link",
      parse(cx, next, pos) {
        if (next !== 91 || cx.char(pos + 1) !== 91 || cx.char(pos - 1) === 33)
          return -1;
        const close = cx.slice(pos + 2, cx.end).indexOf("]]");
        if (close <= 0) return -1;
        const inner = cx.slice(pos + 2, pos + 2 + close);
        if (/[[\]\n]/.test(inner)) return -1;
        const end = pos + close + 4;
        const children = [cx.elt("WikiLinkMark", pos, pos + 2)];
        const bar = inner.indexOf("|");
        if (bar > 0)
          children.push(cx.elt("WikiLinkTarget", pos + 2, pos + 3 + bar));
        children.push(cx.elt("WikiLinkMark", end - 2, end));
        return cx.addElement(cx.elt("WikiLink", pos, end, children));
      },
    },
  ],
  props: [
    styleTags({
      WikiLinkMark: tags.processingInstruction,
      WikiLinkTarget: tags.processingInstruction,
    }),
  ],
};

/**
 * Enter continues a list or quote, but never edits lines after the cursor:
 * the stock command renumbers the rest of an ordered list, which would save
 * changes the user did not make.
 */
export const continueMarkup: StateCommand = ({ state, dispatch }) =>
  insertNewlineContinueMarkup({
    state,
    dispatch(tr) {
      const limit = Math.max(
        ...state.selection.ranges.map((r) => state.doc.lineAt(r.to).to),
      );
      const kept: ChangeSpec[] = [];
      let dropped = false;
      tr.changes.iterChanges((from, to, _fromB, _toB, insert) => {
        if (from > limit) dropped = true;
        else kept.push({ from, to, insert });
      });
      if (!dropped) return dispatch(tr);
      // Dropped changes all come after every cursor, so the selection the
      // command chose is still valid in the shorter change set.
      dispatch(
        state.update({
          changes: kept,
          selection: tr.newSelection,
          scrollIntoView: true,
          userEvent: "input",
        }),
      );
    },
  });

export const markdownKeys = [
  { key: "Enter", run: continueMarkup },
  { key: "Backspace", run: deleteMarkupBackward },
];

/**
 * The language for notes. Its own keymap is replaced by `markdownKeys`, and a
 * pasted URL is pasted as text rather than turned into a link.
 */
export function noteMarkdown(): LanguageSupport {
  return markdown({
    base: commonmarkLanguage,
    extensions: [GFM, Frontmatter, WikiLink],
    addKeymap: false,
    pasteURLAsLink: false,
  });
}
