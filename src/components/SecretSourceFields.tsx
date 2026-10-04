import { useId, useState } from "react";
import { errorMessage, ipc } from "../lib/ipc";
import { commandPreview, type SourceDraft } from "../lib/secrets";

/**
 * The fields of a secret source Brainiac only reads (SPEC.md, Secrets): an
 * environment variable's name, or a program and its arguments, each its own
 * field, with the exact argument array it runs with. The kind is chosen by
 * the form around it.
 */
export default function SecretSourceFields({
  draft,
  onChange,
  what,
}: {
  draft: SourceDraft;
  onChange: (draft: SourceDraft) => void;
  /** "token" or "password", for the hints. */
  what: string;
}) {
  const id = useId();
  const [finding, setFinding] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (draft.kind === "environment") {
    return (
      <div className="flex flex-col gap-1">
        <label className="flex flex-col gap-1" htmlFor={`${id}-name`}>
          <span className="text-[12px] font-medium text-fg-2">
            Variable name
          </span>
          <input
            id={`${id}-name`}
            className="text-input mono"
            value={draft.name}
            placeholder="GITHUB_TOKEN"
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => onChange({ ...draft, name: e.target.value })}
          />
        </label>
        <span className="text-[11.5px] text-muted">
          Read from Brainiac's own environment. An app opened from Finder or the
          Dock does not get a shell's variables, so this suits{" "}
          <span className="mono">pnpm tauri dev</span> and scripts; otherwise
          use a command.
        </span>
      </div>
    );
  }
  if (draft.kind !== "command") return null;

  const find = async () => {
    setFinding(true);
    setError(null);
    try {
      const path = await ipc.findSecretProgram(draft.program);
      onChange({ ...draft, program: path });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setFinding(false);
    }
  };
  const setArg = (i: number, value: string) =>
    onChange({
      ...draft,
      args: draft.args.map((a, j) => (j === i ? value : a)),
    });
  const removeArg = (i: number) =>
    onChange({ ...draft, args: draft.args.filter((_, j) => j !== i) });

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-col gap-1">
        <label className="flex flex-col gap-1" htmlFor={`${id}-program`}>
          <span className="text-[12px] font-medium text-fg-2">Program</span>
          <div className="flex gap-2">
            <input
              id={`${id}-program`}
              className="text-input mono flex-1"
              value={draft.program}
              placeholder="gh, op, or a full path"
              spellCheck={false}
              autoComplete="off"
              onChange={(e) => onChange({ ...draft, program: e.target.value })}
            />
            <button
              type="button"
              className="btn"
              disabled={finding || !draft.program.trim()}
              onClick={() => void find()}
            >
              {finding ? "Finding…" : "Find…"}
            </button>
          </div>
        </label>
        <span className="text-[11.5px] text-muted">
          Saved with its full path; Find… looks a name up in Brainiac's PATH and
          Homebrew's folders.
        </span>
      </div>
      <fieldset className="m-0 flex flex-col gap-1 border-0 p-0">
        <legend className="mb-1 p-0 text-[12px] font-medium text-fg-2">
          Arguments, one per field
        </legend>
        {draft.args.map((arg, i) => (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: arguments are positional.
            key={i}
            className="flex gap-2"
          >
            <input
              className="text-input mono flex-1"
              aria-label={`Argument ${i + 1}`}
              value={arg}
              spellCheck={false}
              autoComplete="off"
              onChange={(e) => setArg(i, e.target.value)}
            />
            <button
              type="button"
              className="btn"
              aria-label={`Remove argument ${i + 1}`}
              onClick={() => removeArg(i)}
            >
              −
            </button>
          </div>
        ))}
        <button
          type="button"
          className="btn btn-sm self-start"
          onClick={() => onChange({ ...draft, args: [...draft.args, ""] })}
        >
          Add Argument
        </button>
      </fieldset>
      <div className="flex flex-col gap-1">
        <span className="text-[12px] font-medium text-fg-2">Runs as</span>
        <code className="mono selectable break-all rounded-md border bg-app px-2 py-1 text-[11.5px]">
          {commandPreview(draft.program.trim(), draft.args)}
        </code>
        <span className="text-[11.5px] text-muted">
          Started without a shell, and it must print only the {what}. The
          program and its arguments are saved and included in backups: use a
          reference such as <span className="mono">op://Vault/item/field</span>,
          never the {what} itself. A password manager may show its unlock
          window.
        </span>
      </div>
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}
    </div>
  );
}
