/** The Settings page's sections and how its fields read and write values (SPEC.md, Main window — v0.2). */

export type SettingsSection =
  | "general"
  | "notes"
  | "repositories"
  | "databases"
  | "accounts"
  | "secrets"
  | "agents"
  | "runs"
  | "explanations"
  | "backup";

export const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "general", label: "General" },
  { id: "notes", label: "Notes and Search" },
  { id: "repositories", label: "Repositories" },
  { id: "databases", label: "Databases" },
  { id: "accounts", label: "Accounts" },
  { id: "secrets", label: "Secrets" },
  { id: "agents", label: "Agent Access" },
  { id: "runs", label: "Agents" },
  { id: "explanations", label: "Explanations" },
  { id: "backup", label: "Backup" },
];

/** The window event that asks App to open a Settings section from anywhere. */
export const OPEN_SETTINGS_EVENT = "brainiac:open-settings";

/** Open a Settings section from a view that has no callback for it. */
export function requestSettings(section: SettingsSection) {
  window.dispatchEvent(
    new CustomEvent<SettingsSection>(OPEN_SETTINGS_EVENT, { detail: section }),
  );
}

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

export type EditorSettings = {
  executable: string;
  repo_args: string[];
  file_args: string[];
};

export type EditorPresetId = "vscode" | "cursor" | "warp" | "chatgpt";

/**
 * Open in Editor presets (SPEC.md, Main window — v0.2). Each runs a program
 * macOS always finds, never one looked up on PATH, which an app opened from
 * the Dock does not get from the shell.
 */
export const EDITOR_PRESETS: {
  id: EditorPresetId;
  label: string;
  note: string;
  editor: EditorSettings;
}[] = [
  {
    id: "vscode",
    label: "VS Code",
    note: "Opens a file at its line, with the command-line tool inside Visual Studio Code in Applications.",
    editor: {
      executable:
        "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
      repo_args: ["{path}"],
      file_args: ["-g", "{path}:{line}"],
    },
  },
  {
    id: "cursor",
    label: "Cursor",
    note: "Opens a file at its line, with the command-line tool inside Cursor in Applications.",
    editor: {
      executable: "/Applications/Cursor.app/Contents/Resources/app/bin/cursor",
      repo_args: ["{path}"],
      file_args: ["-g", "{path}:{line}"],
    },
  },
  {
    id: "warp",
    label: "Warp",
    note: "Opens the folder or file in a new tab of Warp, at the top of the file.",
    editor: {
      executable: "/usr/bin/open",
      repo_args: ["warp://action/new_tab?path={path_url}"],
      // Warp documents `path` as a folder, but given a file it opens the file
      // in its editor in a new tab (checked with Warp 0.2026.09.23).
      file_args: ["warp://action/new_tab?path={path_url}"],
    },
  },
  {
    id: "chatgpt",
    label: "ChatGPT",
    note: "Opens the folder or file in the ChatGPT app, at the top of the file.",
    editor: {
      executable: "/usr/bin/open",
      repo_args: ["-a", "ChatGPT", "{path}"],
      file_args: ["-a", "ChatGPT", "{path}"],
    },
  },
];

const sameArgs = (a: string[], b: string[]) =>
  a.length === b.length && a.every((x, i) => x === b[i]);

/** The preset these settings are, or null for Custom. */
export function editorPreset(editor: EditorSettings): EditorPresetId | null {
  const found = EDITOR_PRESETS.find(
    (p) =>
      // Brainiac's default, `code` on PATH, is VS Code too.
      (p.editor.executable === editor.executable ||
        (p.id === "vscode" && editor.executable === "code")) &&
      sameArgs(p.editor.repo_args, editor.repo_args) &&
      sameArgs(p.editor.file_args, editor.file_args),
  );
  return found?.id ?? null;
}

/** An argument the program cannot do without: where the folder or file goes. */
export function hasPathArgument(args: string[]): boolean {
  return args.some((a) => a.includes("{path}") || a.includes("{path_url}"));
}
