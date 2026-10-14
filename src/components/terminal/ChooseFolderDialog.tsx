/**
 * New terminal on a manual workspace, which has no one folder: which
 * member to open in. Brainiac remembers the choice for the workspace.
 */
import { useState } from "react";
import type { FolderChoice, TerminalTarget } from "../../lib/terminal";
import Dialog from "../Dialog";

export default function ChooseFolderDialog({
  choice,
  onOpen,
  onClose,
}: {
  choice: FolderChoice;
  onOpen: (target: TerminalTarget) => void;
  onClose: () => void;
}) {
  const [picked, setPicked] = useState(choice.members[0]?.cwd ?? "");
  const member = choice.members.find((m) => m.cwd === picked);
  return (
    <Dialog
      title="Open a terminal in"
      width={420}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!member}
            onClick={() =>
              member &&
              onOpen({
                cwd: member.cwd,
                title: member.title,
                workspaceId: choice.workspace.id,
              })
            }
          >
            Open
          </button>
        </>
      }
    >
      <div className="flex flex-col gap-1">
        {choice.members.map((m) => (
          <label
            key={m.cwd}
            className="flex items-center gap-2 rounded-md px-2 py-1.5 has-checked:bg-accent-soft"
          >
            <input
              type="radio"
              name="terminal-folder"
              checked={picked === m.cwd}
              onChange={() => setPicked(m.cwd)}
            />
            <span>{m.title}</span>
            <span className="mono truncate text-[12px] text-muted">
              {m.cwd}
            </span>
          </label>
        ))}
        <p className="m-0 mt-2 text-[12px] text-muted">
          {choice.workspace.name} is a workspace with no shared folder. Brainiac
          remembers your choice.
        </p>
        {choice.members.length === 0 && (
          <p className="m-0 text-[12px] text-muted">
            It has no members on this Mac to open in.
          </p>
        )}
      </div>
    </Dialog>
  );
}
