import { useState } from "react";
import { CONCEPT_KINDS, conceptEdit, conceptKindChoices } from "../lib/explain";
import type { ConceptKind, KnownConcept } from "../lib/ipc";
import Dialog from "./Dialog";

/** Each kind in a few words, with examples, so the choice can be made here. */
const KIND_HELP: Record<ConceptKind, { label: string; help: string }> = {
  language: {
    label: "Language feature",
    help: "Part of one programming language: async/await, go:embed.",
  },
  library: {
    label: "Library",
    help: "A package or framework the code depends on: serde, React.",
  },
  protocol: {
    label: "Protocol or standard",
    help: "Something with a published spec: HTTP, JWT, DKIM.",
  },
  tool: {
    label: "Tool or service",
    help: "A program the code runs or talks to: git, PostgreSQL, Redis.",
  },
  technique: {
    label: "Technique",
    help: "A way of solving a problem that means the same in any codebase: idempotency, retry with backoff.",
  },
  project_pattern: {
    label: "Project pattern",
    help: "True of one repository only: its outbox table, a domain word such as “invoice”.",
  },
};

/**
 * Edit Concept (SPEC.md, section 14, Concepts You Know): the name and the
 * kind, with what the change does said before it is saved: the old name
 * stays with the concept, a project pattern becomes known everywhere, or
 * another concept already has the name.
 */
export default function ConceptEditDialog({
  concept,
  concepts,
  onClose,
  onSave,
}: {
  concept: KnownConcept;
  /** Every concept, names that stand for others included. */
  concepts: KnownConcept[];
  onClose: () => void;
  onSave: (name: string, kind: ConceptKind) => Promise<void>;
}) {
  const [name, setName] = useState(concept.name);
  const [kind, setKind] = useState<ConceptKind>(concept.kind);
  const [busy, setBusy] = useState(false);
  const allowed = conceptKindChoices(concept.kind);
  const edit = conceptEdit(concept, name, kind, concepts);
  const blank = !name.trim();
  const canSave = edit.changed && !blank && !edit.takenBy && !busy;
  const others = concepts
    .filter((c) => c.merged_into === concept.id)
    .map((c) => c.name);

  const save = async () => {
    if (!canSave) return;
    setBusy(true);
    try {
      await onSave(name.trim(), kind);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      title="Edit Concept"
      onClose={onClose}
      width={560}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!canSave}
            onClick={() => void save()}
          >
            {edit.becomesGlobal ? "Save and Know Everywhere" : "Save"}
          </button>
        </>
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <label className="flex flex-col gap-1.5">
          <span className="text-[12.5px] font-semibold">Name</span>
          <input
            className="field"
            value={name}
            maxLength={200}
            autoFocus
            onChange={(e) => setName(e.target.value)}
          />
          {edit.takenBy ? (
            <span role="alert" className="text-[12px] text-conflict">
              “{edit.takenBy.name}” is already a{" "}
              {KIND_HELP[edit.takenBy.kind].label.toLowerCase()} you know. To
              join the two, close this, select both, and use Merge into One.
            </span>
          ) : (
            others.length > 0 && (
              <span className="text-[12px] text-muted">
                Also called {others.join(", ")}
              </span>
            )
          )}
        </label>

        <fieldset className="m-0 flex flex-col gap-1 border-0 p-0">
          <legend className="mb-1.5 text-[12.5px] font-semibold">Kind</legend>
          {CONCEPT_KINDS.map((k) => {
            const disabled = !allowed.includes(k);
            return (
              <label
                key={k}
                className="flex items-start gap-2.5 rounded-md px-2.5 py-1.5"
                style={{
                  background: kind === k ? "var(--sel)" : undefined,
                  opacity: disabled ? 0.55 : undefined,
                }}
              >
                <input
                  type="radio"
                  name="kind"
                  className="mt-1"
                  value={k}
                  checked={kind === k}
                  disabled={disabled}
                  onChange={() => setKind(k)}
                />
                <span className="flex flex-col">
                  <span className="text-[13px]">{KIND_HELP[k].label}</span>
                  <span className="text-[12px] text-muted">
                    {disabled
                      ? "Not for a concept known in every repository: other repositories would teach it again."
                      : KIND_HELP[k].help}
                  </span>
                </span>
              </label>
            );
          })}
        </fieldset>

        {edit.becomesGlobal && (
          <div
            role="status"
            className="rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-[12.5px] text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100"
          >
            It belongs only to{" "}
            {concept.repository_name ?? "a removed repository"} now. As a{" "}
            {KIND_HELP[kind].label.toLowerCase()}, it is known in every
            repository: their explanations leave it out, and its name is listed
            for their agents.
          </div>
        )}
        {edit.keepsOldName && !edit.takenBy && !blank && (
          <p className="m-0 text-[12px] text-muted">
            “{concept.name}” ({KIND_HELP[concept.kind].label.toLowerCase()})
            stays with it as another name, so explanations that use that name
            still leave it out.
          </p>
        )}
        {concept.description && (
          <p className="m-0 border-t pt-3 text-[12px] text-fg-3">
            {concept.description}
          </p>
        )}
      </form>
    </Dialog>
  );
}
