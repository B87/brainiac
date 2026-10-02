import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { insertNewlineContinueMarkup } from "@codemirror/lang-markdown";
import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import { EditorState, type Transaction } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { keepLineEndings, lineSeparatorOf, noteText } from "./lineEndings";
import { continueMarkup, noteMarkdown } from "./markdown";
import { linkAt, type PreviewItem, previewItems, taskToggle } from "./preview";

const FIXTURES = join(__dirname, "fixtures");
const fixtures = readdirSync(FIXTURES)
  .filter((f) => f.endsWith(".md"))
  .map((f) => ({ name: f, text: readFileSync(join(FIXTURES, f), "utf8") }));
const fixture = (name: string) =>
  fixtures.find((f) => f.name === name)?.text ?? "";

/** A parsed note with the cursor at `cursor` (or at the first `|` marker). */
function note(text: string, cursor?: number): EditorState {
  let doc = text;
  let anchor = cursor ?? 0;
  if (cursor === undefined && text.includes("|^")) {
    anchor = text.indexOf("|^");
    doc = text.replace("|^", "");
  }
  const state = EditorState.create({
    doc,
    selection: { anchor },
    extensions: [keepLineEndings(doc), noteMarkdown()],
  });
  ensureSyntaxTree(state, state.doc.length, 5000);
  return state;
}

const nodes = (state: EditorState, name: string) => {
  const found: string[] = [];
  syntaxTree(state).iterate({
    enter: (n) => {
      if (n.name === name) found.push(state.sliceDoc(n.from, n.to));
    },
  });
  return found;
};

const slices = (state: EditorState, items: PreviewItem[], type: string) =>
  items
    .filter((i) => i.type === type)
    .map((i) => state.sliceDoc(i.from, "to" in i ? i.to : i.from));

describe("line endings", () => {
  it("are rewritten by CodeMirror's default, which is why the note sets its own", () => {
    const state = EditorState.create({ doc: "a\r\nb\r\n" });
    expect(noteText(state)).toBe("a\nb\n");
    // doc.toString() ignores the separator; noteText does not.
    expect(note("a\r\nb\r\n").doc.toString()).toBe("a\nb\n");
  });

  it("round-trip every fixture exactly", () => {
    for (const { name, text } of fixtures)
      expect(noteText(note(text)), name).toBe(text);
  });

  it("keep mixed endings and stray carriage returns", () => {
    for (const text of ["a\r\nb\nc\r", "a\rb", "\r\n", "no newline"])
      expect(noteText(note(text))).toBe(text);
    expect(lineSeparatorOf("a\r\nb\r\n")).toBe("\r\n");
    expect(lineSeparatorOf("a\r\nb\n")).toBe("\n");
  });

  it("insert the note's own line break", () => {
    const state = note("a\r\nb\r\n", 1);
    const tr = state.update(state.replaceSelection(state.lineBreak));
    expect(noteText(tr.state)).toBe("a\r\n\r\nb\r\n");
  });
});

describe("dialect", () => {
  it("reads frontmatter only at the top and only when closed", () => {
    expect(
      nodes(note(fixture("09-frontmatter.md")), "Frontmatter"),
    ).toHaveLength(1);
    expect(
      nodes(note("---\ntitle: x\n---\n\n---\n"), "HorizontalRule"),
    ).toEqual(["---"]);
    expect(nodes(note("Text\n\n---\ntitle: x\n---\n"), "Frontmatter")).toEqual(
      [],
    );
    expect(nodes(note("---\nno closing line\n"), "Frontmatter")).toEqual([]);
  });

  it("reads wikilinks but not embeds or code", () => {
    const state = note("[[A note]] [[A note|alias]] ![[image.png]] `[[code]]`");
    expect(nodes(state, "WikiLink")).toEqual([
      "[[A note]]",
      "[[A note|alias]]",
    ]);
    expect(nodes(state, "WikiLinkTarget")).toEqual(["A note|"]);
  });
});

describe("Enter in lists", () => {
  const enter = (text: string, command = continueMarkup) => {
    const state = note(text);
    let next = state;
    command({ state, dispatch: (tr: Transaction) => (next = tr.state) });
    return noteText(next);
  };

  it("continues bullets, tasks, and numbers", () => {
    expect(enter("- a|^")).toBe("- a\n- ");
    expect(enter("- [x] a|^")).toBe("- [x] a\n- [ ] ");
    expect(enter("1. a|^")).toBe("1. a\n2. ");
  });

  it("never renumbers the items below, unlike the stock command", () => {
    expect(enter("1. a|^\n2. b\n3. c", insertNewlineContinueMarkup)).toBe(
      "1. a\n2. \n3. b\n4. c",
    );
    expect(enter("1. a|^\n2. b\n3. c")).toBe("1. a\n2. \n2. b\n3. c");
  });
});

describe("Live Preview", () => {
  const typical = fixture("15-typical.md");

  it("hides markup away from the line being edited", () => {
    const state = note(typical, typical.length);
    const items = previewItems(state, true);
    const hidden = slices(state, items, "hide");
    expect(hidden).toEqual(
      expect.arrayContaining([
        "# ",
        "## ",
        "[",
        "](https://example.com/org/payments-api)",
        "`",
        "- ",
        "```bash",
        "```",
        "[[",
        "]]",
      ]),
    );
    expect(items.filter((i) => i.type === "bullet")).toHaveLength(2);
    expect(
      items.flatMap((i) => (i.type === "checkbox" ? [i.checked] : [])),
    ).toEqual([false, true]);
    // Frontmatter is never hidden.
    expect(hidden.some((h) => h.includes("---") || h.includes("title"))).toBe(
      false,
    );
  });

  it("shows the markup of the edited line", () => {
    const at = typical.indexOf("# Payments service") + 3;
    const state = note(typical, at);
    const line = state.doc.lineAt(at);
    const items = previewItems(state, true);
    const onLine = items.filter(
      (i) => i.type !== "line" && i.from >= line.from && i.from <= line.to,
    );
    expect(onLine).toEqual([]);
    expect(slices(state, items, "hide")).toContain("## ");
  });

  it("draws only styles and links in Source mode", () => {
    const state = note(typical, typical.length);
    const types = new Set(previewItems(state, false).map((i) => i.type));
    expect([...types].sort()).toEqual(["line", "link", "mark"]);
  });

  it("leaves bracketed text that is not a link alone", () => {
    const state = note("[WIP] draft\n", 12);
    expect(slices(state, previewItems(state, true), "hide")).toEqual([]);
  });

  it("never hides across lines or overlaps hidden ranges, in any fixture", () => {
    for (const { name, text } of fixtures) {
      const state = note(text, 0);
      const ranges = previewItems(state, true)
        .filter(
          (i) => i.type !== "line" && i.type !== "link" && i.type !== "mark",
        )
        .map((i) => ({ from: i.from, to: i.to }))
        .sort((a, b) => a.from - b.from);
      for (const [i, r] of ranges.entries()) {
        expect(state.doc.lineAt(r.from).number, name).toBe(
          state.doc.lineAt(r.to).number,
        );
        if (i > 0)
          expect(r.from, name).toBeGreaterThanOrEqual(ranges[i - 1].to);
      }
    }
  });

  it("styles code blocks, quotes, frontmatter, tables, and headings by line", () => {
    const state = note(typical, typical.length);
    const classes = new Set(
      previewItems(state, false).flatMap((i) =>
        i.type === "line" ? [i.className] : [],
      ),
    );
    for (const c of [
      "cm-md-frontmatter",
      "cm-md-codeblock",
      "cm-md-table",
      "cm-md-heading cm-md-h1",
    ])
      expect(classes).toContain(c);
  });
});

describe("checkboxes and links", () => {
  it("toggle only the character between the brackets", () => {
    for (const [before, after] of [
      ["- [ ] a", "- [x] a"],
      ["- [x] a", "- [ ] a"],
      ["- [X] a", "- [ ] a"],
    ]) {
      const state = note(before, 0);
      const change = taskToggle(state, 2);
      expect(change).not.toBeNull();
      expect(noteText(state.update({ changes: change ?? [] }).state)).toBe(
        after,
      );
    }
    expect(taskToggle(note("plain text", 0), 2)).toBeNull();
  });

  it("find the link under the pointer", () => {
    const text =
      "[web](https://example.com) [note](other.md) [[Target|alias]] https://bare.example.com <https://auto.example.com> plain";
    const state = note(text, 0);
    const at = (s: string) => linkAt(state, text.indexOf(s) + 1);
    expect(at("web")).toEqual({ kind: "url", target: "https://example.com" });
    expect(at("note]")).toEqual({ kind: "note", target: "other.md" });
    expect(at("alias")).toEqual({ kind: "note", target: "Target", wiki: true });
    expect(at("bare")).toEqual({
      kind: "url",
      target: "https://bare.example.com",
    });
    expect(at("auto")).toEqual({
      kind: "url",
      target: "https://auto.example.com",
    });
    expect(at("plain")).toBeNull();
  });
});
