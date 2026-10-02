/**
 * CodeMirror splits lines on `\r\n`, `\r`, and `\n` and joins them with `\n`
 * unless told otherwise, so a CRLF note would come back with every line ending
 * changed. The note's own separator is set on the editor state instead: CRLF
 * when every line break is CRLF, otherwise LF, which keeps any stray `\r` as an
 * ordinary character. Either way the editor's text equals the file's.
 */
import { EditorState, type Extension } from "@codemirror/state";

export function lineSeparatorOf(text: string): "\n" | "\r\n" {
  const lf = text.split("\n").length - 1;
  const crlf = text.split("\r\n").length - 1;
  return lf > 0 && lf === crlf ? "\r\n" : "\n";
}

export function keepLineEndings(text: string): Extension {
  return EditorState.lineSeparator.of(lineSeparatorOf(text));
}

/**
 * The note's text as it is saved. `state.doc.toString()` always joins lines
 * with `\n`; only `sliceDoc` uses the separator set above.
 */
export function noteText(state: EditorState): string {
  return state.sliceDoc();
}
