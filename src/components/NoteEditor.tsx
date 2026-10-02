/**
 * The note editor as a React component. The CodeMirror view is created once
 * and owns the text while the user types; a `text` prop that differs from what
 * the editor last reported (a reload from disk, another note) replaces the
 * editor state, which also resets undo history and the line-ending choice.
 */
import { EditorView } from "@codemirror/view";
import { useEffect, useRef } from "react";
import {
  createNoteState,
  type NoteEditorOptions,
  type NoteLink,
  setLivePreview,
} from "../lib/editor/livePreview";

type Props = {
  text: string;
  livePreview: boolean;
  onChange: (text: string) => void;
  onOpenLink?: (link: NoteLink) => void;
  resolveImage?: (src: string) => string | null;
  className?: string;
};

export function NoteEditor(props: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const reported = useRef(props.text);
  // The latest callbacks, so the editor never needs recreating for new ones.
  const latest = useRef(props);
  latest.current = props;

  const options = (): NoteEditorOptions => ({
    livePreview: latest.current.livePreview,
    onChange: (text) => {
      reported.current = text;
      latest.current.onChange(text);
    },
    onOpenLink: (link) => latest.current.onOpenLink?.(link),
    resolveImage: (src) => latest.current.resolveImage?.(src) ?? null,
  });

  // biome-ignore lint/correctness/useExhaustiveDependencies: created once; text changes are applied below.
  useEffect(() => {
    if (!host.current) return;
    const created = new EditorView({
      parent: host.current,
      state: createNoteState(props.text, options()),
    });
    view.current = created;
    return () => {
      created.destroy();
      view.current = null;
    };
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: options() reads the latest props.
  useEffect(() => {
    if (!view.current || props.text === reported.current) return;
    reported.current = props.text;
    view.current.setState(createNoteState(props.text, options()));
  }, [props.text]);

  useEffect(() => {
    if (view.current) setLivePreview(view.current, props.livePreview);
  }, [props.livePreview]);

  return <div ref={host} className={props.className} />;
}
