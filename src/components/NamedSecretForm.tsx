import { useId, useState } from "react";
import { errorMessage, ipc, type NamedSecret } from "../lib/ipc";
import {
  draftOf,
  type SourceDraft,
  type SourceKind,
  sourceOf,
} from "../lib/secrets";
import SecretSourceFields from "./SecretSourceFields";

/**
 * Add or change a named secret (docs/design/scheduled-runs.md): a label, the
 * variable a run sees it as, and where its value is read from. The value is
 * pasted for the Keychain and never shown again; any other source is read
 * when it is shared or used.
 */
export default function NamedSecretForm({
  secret,
  onSaved,
  onCancel,
}: {
  /** The secret being changed, or null to add one. */
  secret: NamedSecret | null;
  /** Gets the secret as saved, so a picker can tick a new one. */
  onSaved: (saved: NamedSecret) => void;
  onCancel: () => void;
}) {
  const id = useId();
  const [label, setLabel] = useState(secret?.label ?? "");
  const [envName, setEnvName] = useState(secret?.env_name ?? "");
  const [asFile, setAsFile] = useState(secret?.as_file ?? false);
  const [note, setNote] = useState(secret?.note ?? "");
  const [source, setSource] = useState<SourceDraft>(() =>
    draftOf(
      secret?.credential_source.kind === "none"
        ? null
        : secret?.credential_source,
      "store",
    ),
  );
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const keepsItem =
    secret?.credential_source.kind === "store" &&
    secret.credential.pending !== "save";
  const ready =
    label.trim() !== "" &&
    envName.trim() !== "" &&
    (source.kind === "store"
      ? value.trim() !== "" || keepsItem
      : source.kind === "environment"
        ? source.name.trim() !== ""
        : source.program.trim() !== "");

  const submit = async () => {
    if (!ready || busy) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await ipc.saveNamedSecret({
        id: secret?.id ?? null,
        expected_version: secret?.version ?? 0,
        label,
        env_name: envName.trim(),
        as_file: asFile,
        note,
        source: sourceOf(source),
        // Multi-line values (a service account key) keep their line breaks.
        secret: source.kind === "store" && value.trim() ? value : null,
      });
      setValue("");
      onSaved(saved);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      aria-label={secret ? `Edit ${secret.label}` : "Add secret"}
      className="flex flex-col gap-2 rounded-[10px] border bg-app px-3.5 py-3"
      onSubmit={(e) => {
        e.preventDefault();
        // Inside a routine's form (Access, New Access…), saving the secret
        // must not also submit the routine.
        e.stopPropagation();
        void submit();
      }}
    >
      <label className="flex flex-col gap-1" htmlFor={`${id}-label`}>
        <span className="text-[12px] font-medium text-fg-2">Name</span>
        <input
          id={`${id}-label`}
          className="text-input"
          value={label}
          placeholder="Sentry token"
          autoComplete="off"
          onChange={(e) => setLabel(e.target.value)}
        />
      </label>
      <label className="flex flex-col gap-1" htmlFor={`${id}-env`}>
        <span className="text-[12px] font-medium text-fg-2">
          Variable a run sees it as
        </span>
        <input
          id={`${id}-env`}
          className="text-input mono"
          value={envName}
          placeholder="SENTRY_TOKEN"
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => setEnvName(e.target.value)}
        />
        <span className="text-[11.5px] text-muted">
          Capitals, digits and underscores. Names that load code or change where
          traffic goes (PATH, NODE_*, GIT_*, *_PROXY …) are refused.
        </span>
      </label>
      <label className="flex items-start gap-2" htmlFor={`${id}-file`}>
        <input
          id={`${id}-file`}
          type="checkbox"
          className="mt-[3px]"
          checked={asFile}
          onChange={(e) => setAsFile(e.target.checked)}
        />
        <span className="flex flex-col gap-0.5">
          <span className="text-[12px] font-medium text-fg-2">
            Give a run a file, not the value
          </span>
          <span className="text-[11.5px] text-muted">
            The variable then holds the path of a file with the value, kept in
            memory for that run. For tools that read a key file, such as{" "}
            <span className="mono">GOOGLE_APPLICATION_CREDENTIALS</span> and a
            service account key. Change it only while the secret is not on a
            host.
          </span>
        </span>
      </label>
      <label className="flex flex-col gap-1" htmlFor={`${id}-source`}>
        <span className="text-[12px] font-medium text-fg-2">Value from</span>
        <select
          id={`${id}-source`}
          className="text-input"
          value={source.kind}
          onChange={(e) =>
            setSource({ ...source, kind: e.target.value as SourceKind })
          }
        >
          <option value="store">The Keychain</option>
          <option value="environment">An environment variable</option>
          <option value="command">A command</option>
        </select>
      </label>
      {source.kind === "store" && (
        <label className="flex flex-col gap-1" htmlFor={`${id}-value`}>
          <span className="text-[12px] font-medium text-fg-2">Value</span>
          <textarea
            id={`${id}-value`}
            className="text-input mono min-h-[64px]"
            value={value}
            spellCheck={false}
            autoComplete="off"
            placeholder={
              keepsItem ? "Leave empty to keep the saved one" : undefined
            }
            onChange={(e) => setValue(e.target.value)}
          />
        </label>
      )}
      <SecretSourceFields draft={source} onChange={setSource} what="value" />
      <label className="flex flex-col gap-1" htmlFor={`${id}-note`}>
        <span className="text-[12px] font-medium text-fg-2">
          Note (what it can do)
        </span>
        <input
          id={`${id}-note`}
          className="text-input"
          value={note}
          placeholder="Read-only, project X"
          autoComplete="off"
          onChange={(e) => setNote(e.target.value)}
        />
        <span className="text-[11.5px] text-muted">
          Only a reminder. Brainiac cannot check a token's scope: give a run a
          token that can read and nothing more, because a run can print it.
        </span>
      </label>
      {error && (
        <div role="alert" className="text-[12.5px] text-conflict">
          {error}
        </div>
      )}
      <div className="flex gap-2">
        <button
          type="submit"
          className="btn btn-sm btn-primary"
          disabled={busy || !ready}
        >
          {busy ? "Saving…" : "Save"}
        </button>
        <button type="button" className="btn btn-sm" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
