/** The Settings page's sections and how its fields read and write values (SPEC.md, Main window — v0.2). */

export type SettingsSection =
  | "general"
  | "notes"
  | "repositories"
  | "accounts"
  | "agents"
  | "backup";

export const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "general", label: "General" },
  { id: "notes", label: "Notes and Search" },
  { id: "repositories", label: "Repositories" },
  { id: "accounts", label: "Accounts" },
  { id: "agents", label: "Agent Access" },
  { id: "backup", label: "Backup" },
];

export function sectionLabel(id: SettingsSection): string {
  return SECTIONS.find((s) => s.id === id)?.label ?? "Settings";
}

/** The editor's arguments as typed: separated by spaces. */
export function argsText(args: string[]): string {
  return args.join(" ");
}

export function parseArgs(text: string): string[] {
  return text.split(/\s+/).filter((a) => a !== "");
}

/**
 * A whole number typed into a field with limits (the backend checks the same
 * ones): the number, or why it cannot be saved.
 */
export function parseInRange(
  text: string,
  min: number,
  max: number,
): { value: number } | { error: string } {
  const trimmed = text.trim().replace(/[,\s]/g, "");
  if (!/^\d+$/.test(trimmed)) return { error: "Enter a whole number." };
  const value = Number(trimmed);
  if (value < min) return { error: `At least ${min.toLocaleString("en")}.` };
  if (value > max) return { error: `At most ${max.toLocaleString("en")}.` };
  return { value };
}

export const MIB = 1024 * 1024;
