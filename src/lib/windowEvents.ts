/**
 * Requests a view deep in the window makes of App, without a callback
 * threaded through every view in between: open a run's conversation, or
 * say something in the window's notice line. (Settings has its own, in
 * `settings.ts`.)
 */

export const OPEN_RUN_EVENT = "brainiac:open-run";
export const NOTICE_EVENT = "brainiac:notice";
export const TOGGLE_EXPLANATION_EVENT = "brainiac:toggle-explanation";
export const OPEN_SUBJECT_EVENT = "brainiac:open-subject";

/** Open a run's conversation, such as an explanation's How it was written. */
export function requestRun(runId: string) {
  window.dispatchEvent(
    new CustomEvent<string>(OPEN_RUN_EVENT, { detail: runId }),
  );
}

/** A short message in the window, such as "Saved as …". */
export function requestNotice(message: string) {
  window.dispatchEvent(
    new CustomEvent<string>(NOTICE_EVENT, { detail: message }),
  );
}

/** Show or hide the explanation panel of the patch on screen (⇧⌘B). */
export function requestToggleExplanation() {
  window.dispatchEvent(new CustomEvent(TOGGLE_EXPLANATION_EVENT));
}

/** What Settings → Explanations' Open shows: a commit, a pull request, or a run. */
export type SubjectToOpen = {
  repositoryId: string;
  kind: "commit" | "pull_request" | "run";
  reference: string;
};

/** Open an explanation's subject where its explanation is shown. */
export function requestOpenSubject(subject: SubjectToOpen) {
  window.dispatchEvent(
    new CustomEvent<SubjectToOpen>(OPEN_SUBJECT_EVENT, { detail: subject }),
  );
}
