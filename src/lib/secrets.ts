/**
 * Where secrets come from (SPEC.md, Secrets): the source pickers' drafts and
 * how a source and its state read. A source is a reference, never a value.
 */
import type {
  CredentialPending,
  CredentialState,
  SecretEntry,
  SecretSource,
} from "./ipc";

export type SourceKind = SecretSource["kind"];

/**
 * A picker's draft: every kind's fields at once, so switching kinds and back
 * keeps what was typed.
 */
export type SourceDraft = {
  kind: SourceKind;
  name: string;
  program: string;
  args: string[];
};

export function draftOf(
  source: SecretSource | null | undefined,
  fallback: SourceKind,
): SourceDraft {
  const draft: SourceDraft = {
    kind: source?.kind ?? fallback,
    name: "",
    program: "",
    args: [],
  };
  if (source?.kind === "environment") draft.name = source.name;
  if (source?.kind === "command") {
    draft.program = source.program;
    draft.args = [...source.args];
  }
  return draft;
}

export function sourceOf(draft: SourceDraft): SecretSource {
  switch (draft.kind) {
    case "environment":
      return { kind: "environment", name: draft.name.trim() };
    case "command":
      return {
        kind: "command",
        program: draft.program.trim(),
        args: draft.args,
      };
    default:
      return { kind: draft.kind };
  }
}

export function sameSource(a: SecretSource, b: SecretSource): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** The exact argument array a command runs with, as the picker previews it. */
export function commandPreview(program: string, args: string[]): string {
  return JSON.stringify([program, ...args]);
}

function basename(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? path;
}

/** "Keychain", "Environment variable GITHUB_TOKEN", "Command op read …". */
export function sourceLabel(source: SecretSource): string {
  switch (source.kind) {
    case "store":
      return "Keychain";
    case "ask":
      return "Asked each run";
    case "none":
      return "No password";
    case "environment":
      return `Environment variable ${source.name}`;
    case "command":
      return `Command ${[basename(source.program), ...source.args].join(" ")}`;
  }
}

const PENDING: Record<CredentialPending, string> = {
  save: "The last save did not finish. Save it again, entering the secret or choosing another source.",
  cleanup:
    "The old Keychain item could not be deleted. The new source is in use.",
  removal: "Removing it did not finish: its Keychain item is still there.",
};

export function pendingLabel(pending: CredentialPending): string {
  return PENDING[pending];
}

/** The one line Settings → Secrets shows for an entry's state. */
export function stateLabel(entry: SecretEntry, now = Date.now()): string {
  if (entry.state.needs_approval)
    return "Restored from a backup: not read until you allow it.";
  if (entry.state.pending) return pendingLabel(entry.state.pending);
  if (entry.input_required) return "Asks for the password when first used.";
  const test = entry.last_test;
  if (!test) return "Not tested.";
  const minutes = Math.max(0, Math.round((now - Date.parse(test.at)) / 60000));
  const when = minutes < 1 ? "just now" : `${minutes} min ago`;
  return `${test.ok ? "Tested" : "Test failed"} ${when}: ${test.message}`;
}

/** Whether the state blocks every use until the user acts. */
export function blocked(state: CredentialState): boolean {
  return (
    state.needs_approval ||
    state.pending === "save" ||
    state.pending === "removal"
  );
}

/** Whether a variable name is one the backend accepts. */
export function validVariable(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(name.trim());
}
