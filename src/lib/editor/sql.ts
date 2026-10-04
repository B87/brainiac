/**
 * The query editor (SPEC.md, Databases: The query view): CodeMirror with SQL
 * in the connection's dialect, completion from the schema, the run keys, and
 * the error position underlined.
 */
import {
  autocompletion,
  type Completion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
} from "@codemirror/autocomplete";
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import {
  PostgreSQL,
  SQLite,
  type SQLNamespace,
  sql,
} from "@codemirror/lang-sql";
import {
  bracketMatching,
  HighlightStyle,
  syntaxHighlighting,
} from "@codemirror/language";
import {
  Compartment,
  EditorState,
  type Extension,
  StateEffect,
  StateField,
} from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  drawSelection,
  EditorView,
  highlightActiveLine,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { tags } from "@lezer/highlight";
import type { DbKind } from "../ipc";

export type SqlEditorOptions = {
  dialect: DbKind;
  /** Schemas, tables, and their columns, for completion. */
  schema: SQLNamespace;
  defaultSchema?: string;
  onChange?: (text: string) => void;
  /** ⌘⏎: the statement under the cursor, or the selection. */
  onRun?: () => void;
  /** ⇧⌘⏎ */
  onRunAll?: () => void;
  /** ⌘. */
  onCancel?: () => void;
  /** ⌘E */
  onExplain?: () => void;
};

const language = new Compartment();

function languageFor(options: SqlEditorOptions): Extension {
  return sql({
    dialect: options.dialect === "sqlite" ? SQLite : PostgreSQL,
    schema: options.schema,
    defaultSchema: options.defaultSchema,
    upperCaseKeywords: false,
  });
}

/** Give the editor a new schema or dialect without losing its text or undo. */
export function setSqlLanguage(view: EditorView, options: SqlEditorOptions) {
  view.dispatch({ effects: language.reconfigure(languageFor(options)) });
}

const setError = StateEffect.define<{ from: number; to: number } | null>();

const errorField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(marks, tr) {
    let next = marks.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(setError)) {
        next = effect.value
          ? Decoration.set([
              Decoration.mark({ class: "cm-sql-error" }).range(
                effect.value.from,
                effect.value.to,
              ),
            ])
          : Decoration.none;
      }
    }
    // Typing clears the underline: it described the text as it was.
    return tr.docChanged ? Decoration.none : next;
  },
  provide: (f) => EditorView.decorations.from(f),
});

/**
 * Underline the word at a UTF-16 offset, where the database said the error
 * is, and put the cursor there; `null` clears it.
 */
export function markError(
  view: EditorView,
  offset: number | null,
  focus = false,
) {
  if (offset === null) {
    view.dispatch({ effects: setError.of(null) });
    return;
  }
  const doc = view.state.doc;
  const from = Math.min(Math.max(0, offset), doc.length);
  const line = doc.lineAt(from);
  const rest = line.text.slice(from - line.from);
  const word = /^[\w$".]+/.exec(rest)?.[0].length ?? 1;
  const to = Math.min(line.to, from + Math.max(1, word));
  view.dispatch({
    effects: [setError.of(from < to ? { from, to } : null)],
    selection: focus ? { anchor: from } : undefined,
    scrollIntoView: focus,
  });
  if (focus) view.focus();
}

const sqlHighlight = HighlightStyle.define([
  { tag: tags.keyword, color: "var(--sql-keyword)", fontWeight: "500" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--sql-string)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--sql-number)" },
  {
    tag: [tags.lineComment, tags.blockComment],
    color: "var(--muted)",
    fontStyle: "italic",
  },
  { tag: [tags.typeName, tags.standard(tags.name)], color: "var(--sql-type)" },
  { tag: [tags.special(tags.name), tags.quote], color: "var(--fg)" },
  { tag: tags.operator, color: "var(--fg-2)" },
]);

const sqlTheme = EditorView.theme({
  "&": { height: "100%", fontSize: "13px", backgroundColor: "transparent" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--mono)", lineHeight: "1.55" },
  ".cm-content": { padding: "8px 0", caretColor: "var(--fg)" },
  ".cm-gutters": {
    backgroundColor: "transparent",
    border: "none",
    color: "var(--faint)",
  },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 10px 0 12px" },
  ".cm-activeLine": {
    backgroundColor: "color-mix(in srgb, var(--fg) 4%, transparent)",
  },
  ".cm-sql-error": {
    textDecoration: "underline wavy var(--conflict-dot)",
    textUnderlineOffset: "3px",
  },
  // Completion looks like the app's menus, in both light and dark.
  ".cm-tooltip": {
    backgroundColor: "var(--header)",
    color: "var(--fg)",
    border: "1px solid var(--control-line)",
    borderRadius: "8px",
    boxShadow: "0 8px 24px rgb(0 0 0 / 0.25)",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul": {
    fontFamily: "var(--mono)",
    fontSize: "12px",
    maxHeight: "16em",
    minWidth: "220px",
    padding: "4px",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li": {
    display: "flex",
    alignItems: "center",
    gap: "12px",
    padding: "3px 8px",
    borderRadius: "5px",
    lineHeight: "1.5",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]": {
    backgroundColor: "var(--accent)",
    color: "var(--accent-fg)",
  },
  ".cm-completionMatchedText": {
    textDecoration: "none",
    fontWeight: "600",
  },
  ".cm-completionType": {
    marginLeft: "auto",
    fontFamily: "var(--sans)",
    fontSize: "11px",
    color: "var(--muted)",
  },
  "li[aria-selected] .cm-completionType": { color: "var(--accent-fg)" },
  ".cm-completionDetail": { color: "var(--muted)", fontStyle: "normal" },
  ".cm-completionInfo": { padding: "6px 10px" },
});

/** The kind of a completion as a quiet word, in place of the default emoji icons. */
const completionType = {
  render(completion: Completion): Node | null {
    if (!completion.type) return null;
    const label = document.createElement("span");
    label.className = "cm-completionType";
    // lang-sql calls built-in functions "variable"; the schema's own are labelled.
    label.textContent =
      completion.type === "variable" ? "function" : completion.type;
    return label;
  },
  position: 80,
};

export function createSqlState(
  text: string,
  options: SqlEditorOptions,
): EditorState {
  const run = (f?: () => void) => () => {
    f?.();
    return true;
  };
  return EditorState.create({
    doc: text,
    extensions: [
      lineNumbers(),
      history(),
      drawSelection(),
      highlightActiveLine(),
      bracketMatching(),
      closeBrackets(),
      autocompletion({
        activateOnTyping: true,
        icons: false,
        addToOptions: [completionType],
      }),
      language.of(languageFor(options)),
      syntaxHighlighting(sqlHighlight),
      errorField,
      keymap.of([
        { key: "Mod-Enter", run: run(options.onRun) },
        { key: "Shift-Mod-Enter", run: run(options.onRunAll) },
        { key: "Mod-.", run: run(options.onCancel) },
        { key: "Mod-e", run: run(options.onExplain) },
        ...closeBracketsKeymap,
        ...completionKeymap,
        ...defaultKeymap,
        ...historyKeymap,
        indentWithTab,
      ]),
      options.onChange
        ? EditorView.updateListener.of((u) => {
            if (u.docChanged) options.onChange?.(u.state.doc.toString());
          })
        : [],
      sqlTheme,
    ],
  });
}

/** The selection as UTF-16 offsets, for the run request. */
export function selectionOf(view: EditorView): { from: number; to: number } {
  const { from, to } = view.state.selection.main;
  return { from, to };
}
