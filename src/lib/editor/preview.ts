/**
 * What the note editor draws over the text, as plain data. `previewItems`
 * reads the syntax tree and returns line styles and link ranges (both modes)
 * plus, in Live Preview, the markup to hide and the widgets that replace it.
 * Nothing here changes the document; `taskToggle` is the one edit, made only
 * when the user clicks a checkbox. The CodeMirror layer is `livePreview.ts`.
 */
import { syntaxTree } from "@codemirror/language";
import type { ChangeSpec, EditorState } from "@codemirror/state";
import type { SyntaxNode } from "@lezer/common";

export type LinkKind = "url" | "note";

export type PreviewItem =
  /** A class for the whole line starting at `from`. */
  | { type: "line"; from: number; className: string }
  /** A styled range in both modes, such as the text of a done task. */
  | { type: "mark"; from: number; to: number; className: string }
  /** Link text, styled as a link in both modes. */
  | { type: "link"; from: number; to: number; kind: LinkKind; target: string }
  /** Markup hidden in Live Preview. */
  | { type: "hide"; from: number; to: number }
  | { type: "bullet"; from: number; to: number }
  | { type: "checkbox"; from: number; to: number; checked: boolean }
  | { type: "rule"; from: number; to: number }
  /** An image; `replace` is false on the line being edited, where the
   * Markdown stays visible and the image shows after it. */
  | {
      type: "image";
      from: number;
      to: number;
      src: string;
      alt: string;
      replace: boolean;
    };

const LINK_PARENTS = new Set(["Link", "Image", "Autolink", "LinkReference"]);

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;

export function linkKind(target: string): LinkKind {
  return SCHEME.test(target) ? "url" : "note";
}

/** Line numbers touched by any selection range: where markup stays visible. */
export function activeLines(state: EditorState): Set<number> {
  const lines = new Set<number>();
  for (const r of state.selection.ranges) {
    const last = state.doc.lineAt(r.to).number;
    for (let n = state.doc.lineAt(r.from).number; n <= last; n++) lines.add(n);
  }
  return lines;
}

const HEADINGS: Record<string, number> = {
  ATXHeading1: 1,
  ATXHeading2: 2,
  ATXHeading3: 3,
  ATXHeading4: 4,
  ATXHeading5: 5,
  ATXHeading6: 6,
  SetextHeading1: 1,
  SetextHeading2: 2,
};

const BLOCK_LINES: Record<string, string> = {
  FencedCode: "cm-md-codeblock",
  CodeBlock: "cm-md-codeblock",
  Frontmatter: "cm-md-frontmatter",
  Blockquote: "cm-md-quote",
  Table: "cm-md-table",
};

/** The label and target of a Markdown link, image, or autolink node. */
function linkParts(state: EditorState, node: SyntaxNode) {
  const marks = node.getChildren("LinkMark");
  const url = node.getChild("URL");
  const label =
    marks.length >= 2 && node.name !== "Autolink"
      ? { from: marks[0].to, to: marks[1].from }
      : url
        ? { from: url.from, to: url.to }
        : null;
  return {
    label,
    target: url ? state.sliceDoc(url.from, url.to) : null,
  };
}

function wikiParts(state: EditorState, node: SyntaxNode) {
  const marks = node.getChildren("WikiLinkMark");
  const alias = node.getChild("WikiLinkTarget");
  const inner = state.sliceDoc(marks[0].to, marks[1].from);
  return {
    label: { from: alias ? alias.to : marks[0].to, to: marks[1].from },
    target: inner.split("|")[0],
  };
}

export function previewItems(
  state: EditorState,
  live: boolean,
  ranges: readonly { from: number; to: number }[] = [
    { from: 0, to: state.doc.length },
  ],
): PreviewItem[] {
  const doc = state.doc;
  const active = activeLines(state);
  const items: PreviewItem[] = [];
  const isActive = (from: number, to: number) => {
    const last = doc.lineAt(to).number;
    for (let n = doc.lineAt(from).number; n <= last; n++)
      if (active.has(n)) return true;
    return false;
  };
  // Hidden markup never crosses a line break: an inline replacement there
  // would join lines.
  const hide = (from: number, to: number) => {
    if (from < to && doc.lineAt(from).number === doc.lineAt(to).number)
      items.push({ type: "hide", from, to });
  };
  const spacesAfter = (pos: number) => {
    const line = doc.lineAt(pos);
    let end = pos;
    while (end < line.to && doc.sliceString(end, end + 1) === " ") end++;
    return end;
  };
  const eachLine = (
    range: { from: number; to: number },
    from: number,
    to: number,
    className: string,
  ) => {
    for (
      let pos = Math.max(from, range.from);
      pos <= Math.min(to, range.to);
    ) {
      const line = doc.lineAt(pos);
      items.push({ type: "line", from: line.from, className });
      pos = line.to + 1;
    }
  };

  for (const range of ranges) {
    syntaxTree(state).iterate({
      from: range.from,
      to: range.to,
      enter(ref) {
        const node = ref.node;
        const parent = node.parent;
        // Markup is hidden unless the construct it belongs to is on a line
        // being edited.
        const hidden = (n: SyntaxNode | null) =>
          live && !isActive((n ?? node).from, (n ?? node).to);
        const heading = HEADINGS[node.name];
        if (heading) {
          items.push({
            type: "line",
            from: doc.lineAt(node.from).from,
            className: `cm-md-heading cm-md-h${heading}`,
          });
          return;
        }
        const block = BLOCK_LINES[node.name];
        if (block) eachLine(range, node.from, node.to, block);

        switch (node.name) {
          case "HeaderMark": {
            if (!hidden(parent)) return;
            const line = doc.lineAt(node.from);
            if (parent?.name.startsWith("Setext")) hide(node.from, node.to);
            else if (node.from === line.from)
              hide(node.from, spacesAfter(node.to));
            else {
              // A closing `###`: hide it with the spaces before it.
              let start = node.from;
              while (
                start > line.from &&
                doc.sliceString(start - 1, start) === " "
              )
                start--;
              hide(start, node.to);
            }
            return;
          }
          case "EmphasisMark":
          case "StrikethroughMark":
            if (hidden(parent)) hide(node.from, node.to);
            return;
          case "CodeMark":
            if (parent?.name === "InlineCode") {
              if (hidden(parent)) hide(node.from, node.to);
            } else if (hidden(node)) {
              // A code fence: hide it with its info string, up to the line end.
              hide(node.from, doc.lineAt(node.from).to);
            }
            return;
          case "QuoteMark":
            if (hidden(node)) hide(node.from, spacesAfter(node.to));
            return;
          case "ListMark": {
            // A list item spans its nested items, so only the marker's own
            // line counts.
            if (!hidden(node) || !parent) return;
            const task = parent.getChild("Task");
            if (task) hide(node.from, spacesAfter(node.to));
            else if (parent.parent?.name === "BulletList")
              items.push({ type: "bullet", from: node.from, to: node.to });
            return;
          }
          case "Task": {
            const marker = node.getChild("TaskMarker");
            if (
              marker &&
              /x/i.test(doc.sliceString(marker.from + 1, marker.to - 1))
            )
              items.push({
                type: "mark",
                from: spacesAfter(marker.to),
                to: node.to,
                className: "cm-md-done",
              });
            return;
          }
          case "TaskMarker":
            if (hidden(node))
              items.push({
                type: "checkbox",
                from: node.from,
                to: node.to,
                checked: /x/i.test(doc.sliceString(node.from + 1, node.to - 1)),
              });
            return;
          case "HorizontalRule":
            if (hidden(node))
              items.push({ type: "rule", from: node.from, to: node.to });
            return;
          case "Escape":
            if (hidden(node)) hide(node.from, node.from + 1);
            return;
          case "Link":
          case "Autolink": {
            const { label, target } = linkParts(state, node);
            if (!label) return;
            // `[text]` without a target or reference label is ordinary text.
            if (!target && !node.getChild("LinkLabel")) return;
            if (target)
              items.push({
                type: "link",
                ...label,
                kind: linkKind(target),
                target,
              });
            if (hidden(node)) {
              hide(node.from, label.from);
              hide(label.to, node.to);
            }
            return;
          }
          case "Image": {
            const { label, target } = linkParts(state, node);
            if (!live || !label || !target) return;
            items.push({
              type: "image",
              from: node.from,
              to: node.to,
              src: target,
              alt: doc.sliceString(label.from, label.to),
              replace: hidden(node),
            });
            // The image replaces its own markup, so nothing inside is drawn.
            return false;
          }
          case "URL":
            // A bare URL; one inside a link or definition is drawn by it.
            if (!parent || !LINK_PARENTS.has(parent.name))
              items.push({
                type: "link",
                from: node.from,
                to: node.to,
                kind: "url",
                target: doc.sliceString(node.from, node.to),
              });
            return;
          case "WikiLink": {
            const { label, target } = wikiParts(state, node);
            items.push({ type: "link", ...label, kind: "note", target });
            if (hidden(node)) {
              hide(node.from, label.from);
              hide(label.to, node.to);
            }
            return;
          }
        }
      },
    });
  }
  return items;
}

/** The link at a document position, for `Cmd`+click. */
export function linkAt(
  state: EditorState,
  pos: number,
): { kind: LinkKind; target: string; wiki?: boolean } | null {
  for (const side of [1, -1] as const) {
    for (
      let node: SyntaxNode | null = syntaxTree(state).resolveInner(pos, side);
      node;
      node = node.parent
    ) {
      if (node.name === "WikiLink") {
        const { target } = wikiParts(state, node);
        return { kind: "note", target, wiki: true };
      }
      if (node.name === "Link" || node.name === "Autolink") {
        const { target } = linkParts(state, node);
        return target ? { kind: linkKind(target), target } : null;
      }
      if (node.name === "URL") {
        const target = state.sliceDoc(node.from, node.to);
        return { kind: linkKind(target), target };
      }
    }
  }
  return null;
}

/**
 * The edit a checkbox click makes: `[ ]` becomes `[x]`, and `[x]` or `[X]`
 * becomes `[ ]`. Only the character between the brackets changes.
 */
export function taskToggle(state: EditorState, pos: number): ChangeSpec | null {
  for (const side of [1, -1] as const) {
    const node = syntaxTree(state).resolveInner(pos, side);
    if (node.name !== "TaskMarker") continue;
    const checked = /x/i.test(state.sliceDoc(node.from + 1, node.to - 1));
    return {
      from: node.from + 1,
      to: node.to - 1,
      insert: checked ? " " : "x",
    };
  }
  return null;
}
