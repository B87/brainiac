/**
 * Secrets on a run host (docs/design/scheduled-runs.md): how each share's
 * state reads, and what can be done about it. A copy on a host is a copy: it
 * does not follow a change here until it is shared again.
 */
import type { HostSecretRow, HostSecretState } from "./ipc";

export type ShareTone = "ok" | "warn" | "bad" | "idle";

export function stateText(state: HostSecretState): string {
  switch (state) {
    case "shared":
      return "On the host";
    case "out_of_date":
      return "Out of date";
    case "missing_on_host":
      return "Missing on the host";
    case "failed":
      return "Did not finish";
    case "not_shared":
      return "Not on this host";
  }
}

export function stateTone(state: HostSecretState): ShareTone {
  switch (state) {
    case "shared":
      return "ok";
    case "out_of_date":
    case "missing_on_host":
      return "warn";
    case "failed":
      return "bad";
    case "not_shared":
      return "idle";
  }
}

/** A sentence for what the state means and what to do. */
export function stateHelp(row: HostSecretRow): string {
  switch (row.state) {
    case "shared":
      return `Runs on this host see it as ${row.env_name}.`;
    case "out_of_date":
      return "The source or value changed here since it was sent, or the host has a copy Brainiac did not send. Share it again to be sure the host has the current one.";
    case "missing_on_host":
      return "Brainiac sent it, but the host no longer holds it. Share it again.";
    case "failed":
      return row.failure ?? "The last Share or Remove did not finish.";
    case "not_shared":
      return "Share it so a run started on this host can use it without this Mac.";
  }
}

/** The button for a row: Share, Share again, Retry, or none. */
export function shareLabel(state: HostSecretState): string {
  switch (state) {
    case "not_shared":
      return "Share";
    case "failed":
      return "Retry";
    default:
      return "Send Again";
  }
}

/** Whether the host may hold a copy, so Remove from Host is offered. */
export function removable(state: HostSecretState): boolean {
  return state !== "not_shared";
}
