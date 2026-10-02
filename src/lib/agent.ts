/** Agent access (SPEC.md, section 9): what Settings shows and copies. */
import type { AgentAccess } from "./ipc";

/** Quote a path for a shell when it needs it. */
export function shellQuote(word: string): string {
  return /^[\w@%+=:,./-]+$/.test(word)
    ? word
    : `'${word.replaceAll("'", `'\\''`)}'`;
}

/** The command that adds Brainiac to Claude Code for every project. */
export function claudeCommand(executable: string): string {
  return `claude mcp add --scope user brainiac -- ${shellQuote(executable)} mcp`;
}

export const ACCESS_CHOICES: { value: AgentAccess; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "read_only", label: "Read only" },
  { value: "read_write", label: "Read and write" },
];

/** What agents may do in each mode. */
export function accessSummary(access: AgentAccess): string {
  switch (access) {
    case "off":
      return "Agents such as Claude Code cannot use Brainiac. They can still read and edit the vault's files like any editor.";
    case "read_only":
      return "Agents can search and read notes, tasks, Today, and the repositories Brainiac tracks.";
    case "read_write":
      return "Agents can also create and edit notes, create and complete tasks, and link notes to repositories. They never delete anything, and each note edit can be undone from the note's history.";
  }
}

/** "No agents connected", "1 agent connected", … */
export function connectedText(count: number, access: AgentAccess): string {
  const agents =
    count === 0
      ? "No agents connected"
      : `${count} agent${count === 1 ? "" : "s"} connected`;
  return count > 0 && access === "off" ? `${agents}, without access` : agents;
}
