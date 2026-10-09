/**
 * The explanations of one subject, kept current: the one shown, its notes'
 * places on a moved branch, and its run's progress while it works (SPEC.md,
 * section 14).
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { pickRecord } from "./explain";
import {
  type ExplainSubject,
  type ExplanationPlacement,
  type ExplanationRecord,
  errorMessage,
  ipc,
  onAgentRunChanged,
  onExplanationChanged,
  subscribe,
} from "./ipc";

export type ExplanationState = {
  records: ExplanationRecord[];
  current: ExplanationRecord | null;
  placement: ExplanationPlacement | null;
  choose: (id: string | null) => void;
  reload: () => void;
  error: string | null;
};

export function useExplanation(
  repositoryId: string,
  subject: ExplainSubject | null,
  /** Changes when the subject moves (a pull request's head): read again. */
  version: string | null = null,
): ExplanationState {
  const [records, setRecords] = useState<ExplanationRecord[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [placement, setPlacement] = useState<ExplanationPlacement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const kind = subject?.kind;
  const reference = subject?.reference;

  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` asks for a new read when the subject moved.
  const reload = useCallback(() => {
    if (!kind || !reference) {
      setRecords([]);
      return;
    }
    ipc
      .listSubjectExplanations(repositoryId, { kind, reference })
      .then((r) => {
        setRecords(r);
        setError(null);
      })
      .catch((e) => setError(errorMessage(e)));
  }, [repositoryId, kind, reference, version]);

  useEffect(() => {
    setChosen(null);
    setPlacement(null);
    reload();
  }, [reload]);

  const working = records.find((r) => r.state === "working");
  const workingRun = working?.run_id ?? null;
  useEffect(
    () =>
      subscribe(
        onExplanationChanged((e) => {
          if (e.repository_id === repositoryId || e.repository_id === "")
            reload();
        }),
        // A working explanation's progress comes from its run.
        onAgentRunChanged((e) => {
          if (workingRun && e.run_id === workingRun) reload();
        }),
      ),
    [repositoryId, reload, workingRun],
  );

  const current = useMemo(() => pickRecord(records, chosen), [records, chosen]);
  const currentId = current?.state === "ready" ? current.id : null;
  // A branch's or a pull request's notes move with it; the explanation
  // shown may be the other's, with the same changes.
  const moves =
    current?.subject.kind === "branch" ||
    current?.subject.kind === "pull_request";
  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` places the notes again when the subject moved.
  useEffect(() => {
    if (!currentId || !moves) {
      setPlacement(null);
      return;
    }
    let live = true;
    ipc
      .placeExplanation(currentId)
      .then((p) => live && setPlacement(p))
      .catch(() => live && setPlacement(null));
    return () => {
      live = false;
    };
  }, [currentId, moves, version]);

  return {
    records,
    current,
    placement,
    choose: setChosen,
    reload,
    error,
  };
}
