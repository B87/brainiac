/**
 * An in-memory stand-in for Brainiac's backend, answering the commands the
 * frontend sends through `invoke`. It keeps notes, tasks, and a repository so
 * the app can be clicked through in WebKit without touching real data; the
 * backend's own behavior is tested by `cargo test`.
 */
import type {
  AppSnapshot,
  FolderEntry,
  ForgeAccountSlot,
  NoteContent,
  NoteSummary,
  RepositorySummary,
  SaveForgeAccountRequest,
  SearchHit,
  Settings,
  Task,
  TaskFields,
  VaultState,
} from "../../src/lib/ipc";

type FakeNote = {
  id: string;
  path: string;
  text: string;
  version: string;
  opened: string | null;
  missing?: boolean;
};

const NOW = "2026-10-02T09:00:00.000Z";

const stemOf = (path: string) => path.replace(/^.*\//, "").replace(/\.md$/, "");
/** The backend's `file_name_for`, enough for the tests' titles. */
const fileNameFor = (title: string) =>
  title.trim().replace(/[/:\\]/g, "-") || "Untitled";
/** The backend's `name_matches`: `name`, or `name` plus a number from 2. */
const nameMatches = (stem: string, name: string) => {
  if (stem === name) return true;
  const n = stem.startsWith(`${name} `) ? stem.slice(name.length + 1) : "";
  return /^\d+$/.test(n) && Number(n) >= 2;
};

const settings: Settings = {
  editor: {
    executable: "code",
    repo_args: ["{path}"],
    file_args: ["-g", "{path}:{line}"],
  },
  refresh_interval_seconds: 60,
  status_timeout_seconds: 5,
  diff_limits: { max_bytes: 1048576, max_lines: 10000 },
  auto_fetch_interval_minutes: 15,
  fetch_timeout_seconds: 60,
  write_note_ids: true,
  agent_access: "off",
};

export const repository: RepositorySummary = {
  id: "repo-1",
  name: "parser",
  canonical_root: "/code/parser",
  display_path: "/code/parser",
  state: "fresh",
  last_checked_at: NOW,
  last_commit_at: NOW,
  head: { kind: "branch", branch: "main", commit_id: "abc1234" },
  counts: {
    staged: 0,
    unstaged: 2,
    untracked: 0,
    conflicted: 0,
    unique_paths: 2,
  },
  upstream: { ref: "origin/main", ahead: 1, behind: 0 },
  error: null,
  last_tab: null,
  last_fetch_at: NOW,
  fetch_error: null,
  remote_url: "git@example.com:team/parser.git",
  forge: null,
};

let counter = 0;
const uid = (prefix: string) => `${prefix}-${++counter}`;
const versionOf = (text: string) => `v${text.length}-${++counter}`;

export class FakeBackend {
  calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
  /** Settings → Accounts: no GitHub account; a Bitbucket token stored from Terminal. */
  accounts: ForgeAccountSlot[] = [
    { kind: "github", account: null, keychain_token: false },
    { kind: "bitbucket_cloud", account: null, keychain_token: true },
  ];
  vault: VaultState = {
    vault: null,
    index: {
      state: "no_vault",
      done: 0,
      total: 0,
      pending_repairs: 0,
      message: null,
    },
  };
  notes = new Map<string, FakeNote>();
  tasks = new Map<string, Task>();
  links = new Map<string, Set<string>>();
  /** Unsaved edits per note, as history.db keeps them. */
  drafts = new Map<string, { text: string; base_version: string }>();
  pins: AppSnapshot["pins"] = [];
  /** What the next native picker returns. */
  nextPick: string | null = "/tmp/Notes";
  /** What the next confirmation answers. */
  nextAsk = true;
  /** Delivers backend events, as the real services emit them after a commit. */
  emit: (name: string, payload: unknown) => void = () => {};

  private later(name: string, payload: unknown) {
    setTimeout(() => this.emit(name, payload), 0);
  }

  snapshot(): AppSnapshot {
    return {
      snapshot_version: ++counter,
      git: {
        available: true,
        version: "2.50.0",
        path: "/usr/bin/git",
        message: null,
      },
      repositories: [repository],
      workspaces: [],
      pins: this.pins,
      recent_repository_ids: [],
      settings,
    };
  }

  /** Change a note as another editor would. */
  editOutside(path: string, text: string) {
    for (const n of this.notes.values())
      if (n.path === path) {
        n.text = text;
        n.version = versionOf(text);
      }
  }

  /** Delete a note's file as another program would. */
  deleteOutside(path: string) {
    for (const n of this.notes.values())
      if (n.path === path) {
        n.missing = true;
        this.later("note_missing", { note_id: n.id });
      }
  }

  summary(n: FakeNote): NoteSummary {
    const heading = /^#\s+(.+)$/m.exec(n.text)?.[1];
    const title = heading ?? stemOf(n.path);
    const name = fileNameFor(title);
    return {
      id: n.id,
      relative_path: n.path,
      title,
      title_file_name:
        n.missing || nameMatches(stemOf(n.path), name) ? null : `${name}.md`,
      text_state: "text",
      missing: !!n.missing,
      trashed: false,
      id_conflict: false,
      has_embedded_id: n.text.includes("brainiac_id"),
      modified_at: NOW,
      last_opened_at: n.opened,
    };
  }

  private noteOf(id: unknown): FakeNote {
    const n = this.notes.get(String(id));
    if (!n)
      throw {
        code: "NOT_FOUND",
        message: "That note does not exist.",
        retryable: false,
        details: null,
      };
    return n;
  }

  private task(fields: TaskFields, base?: Task): Task {
    const note = fields.note_id ? this.notes.get(fields.note_id) : undefined;
    const open = fields.status === "todo" || fields.status === "in_progress";
    const hasDate = !!(fields.planned_date || fields.due_date);
    return {
      id: base?.id ?? uid("task"),
      title: fields.title.trim(),
      description: fields.description,
      status: fields.status,
      to_sort: open && !fields.sorted && !hasDate,
      planned_date: fields.planned_date,
      due_date: fields.due_date,
      note: note
        ? {
            id: note.id,
            title: this.summary(note).title,
            relative_path: note.path,
            missing: false,
          }
        : null,
      repository_id: fields.repository_id,
      created_at: base?.created_at ?? NOW,
      updated_at: NOW,
      completed_at:
        fields.status === "done"
          ? (base?.completed_at ?? new Date().toISOString())
          : null,
      version: (base?.version ?? 0) + 1,
    };
  }

  private hit(
    kind: "note" | "task",
    id: string,
    title: string,
    detail: string,
  ): SearchHit {
    return {
      kind,
      id,
      title: [{ text: title, highlight: false }],
      detail,
      snippet: [],
    };
  }

  handle(cmd: string, raw: unknown): unknown {
    const args = (raw ?? {}) as Record<string, unknown>;
    this.calls.push({ cmd, args });
    const today = new Date();
    const todayStr = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
    switch (cmd) {
      case "get_app_snapshot":
        return this.snapshot();
      case "get_vault_state":
        return this.vault;
      case "select_vault":
        this.vault = {
          vault: {
            id: "vault-1",
            name: "Notes",
            root_path: String(args.path),
            available: true,
          },
          index: {
            state: "ready",
            done: 0,
            total: 0,
            pending_repairs: 0,
            message: null,
          },
        };
        return this.vault;
      case "list_folder": {
        const folder = (args.folder as string | null) ?? "";
        const entries: FolderEntry[] = [];
        const seen = new Set<string>();
        for (const n of this.notes.values()) {
          const rest = folder
            ? n.path.startsWith(`${folder}/`)
              ? n.path.slice(folder.length + 1)
              : null
            : n.path;
          if (rest === null) continue;
          const slash = rest.indexOf("/");
          if (slash === -1)
            entries.push({
              name: rest,
              relative_path: n.path,
              kind: "note",
              note: this.summary(n),
            });
          else {
            const name = rest.slice(0, slash);
            if (seen.has(name)) continue;
            seen.add(name);
            entries.push({
              name,
              relative_path: folder ? `${folder}/${name}` : name,
              kind: "folder",
              note: null,
            });
          }
        }
        return { relative_path: folder, entries };
      }
      case "get_note_lists":
        return {
          pinned: this.pins
            .filter((p) => p.entity_type === "note")
            .map((p) => this.summary(this.noteOf(p.entity_id))),
          recent: [...this.notes.values()]
            .filter((n) => n.opened)
            .map((n) => this.summary(n)),
        };
      case "read_note": {
        const n = this.noteOf(args.noteId);
        const content: NoteContent = {
          note: this.summary(n),
          text: n.text,
          version: n.version,
          draft: this.drafts.has(n.id)
            ? {
                ...(this.drafts.get(n.id) as {
                  text: string;
                  base_version: string;
                }),
                updated_at: NOW,
              }
            : null,
        };
        return content;
      }
      case "mark_note_opened":
        this.noteOf(args.noteId).opened = NOW;
        return null;
      case "save_note": {
        const req = args.request as {
          note_id: string;
          expected_version: string;
          text: string;
        };
        const n = this.noteOf(req.note_id);
        this.drafts.set(n.id, {
          text: req.text,
          base_version: req.expected_version,
        });
        if (req.expected_version !== n.version)
          throw {
            code: "CONFLICT",
            message:
              "The note changed on disk since it was opened. Your edits are kept as a draft.",
            retryable: false,
            details: null,
          };
        this.drafts.delete(n.id);
        n.text = req.text;
        n.version = versionOf(req.text);
        this.later("note_changed", {
          note_id: n.id,
          version: n.version,
          relative_path: n.path,
          origin: "app",
        });
        return {
          note: this.summary(n),
          version: n.version,
          search_pending: false,
        };
      }
      case "create_note": {
        const req = args.request as {
          folder: string | null;
          title: string | null;
          repository_id: string | null;
        };
        const title = req.title || "Untitled";
        const id = uid("note");
        let path = `${req.folder ? `${req.folder}/` : ""}${title}.md`;
        let i = 2;
        while ([...this.notes.values()].some((n) => n.path === path))
          path = `${req.folder ? `${req.folder}/` : ""}${title} ${i++}.md`;
        const text = `---\nbrainiac_id: ${id}\n---\n\n# ${title}\n\n`;
        const n = { id, path, text, version: versionOf(text), opened: null };
        this.notes.set(id, n);
        if (req.repository_id) this.links.set(id, new Set([req.repository_id]));
        this.later("note_changed", {
          note_id: id,
          version: n.version,
          relative_path: path,
          origin: "app",
        });
        return this.summary(n);
      }
      case "get_note_context": {
        const n = this.noteOf(args.noteId);
        return {
          note_id: n.id,
          repositories: [...(this.links.get(n.id) ?? [])].map((rid) => ({
            repository_id: rid,
            name: repository.name,
            remote_url: repository.remote_url,
            registered: true,
            reconnect_to: null,
          })),
          tasks: [...this.tasks.values()].filter((t) => t.note?.id === n.id),
          backlinks: [],
          unresolved: [],
          suggestions:
            n.text.includes("parser") && !this.links.get(n.id)?.size
              ? [{ repository_id: repository.id, name: repository.name }]
              : [],
        };
      }
      case "link_repository": {
        const set = this.links.get(String(args.noteId)) ?? new Set<string>();
        set.add(String(args.repositoryId));
        this.links.set(String(args.noteId), set);
        const linked = this.noteOf(args.noteId);
        this.later("note_changed", {
          note_id: linked.id,
          version: linked.version,
          relative_path: linked.path,
          origin: "app",
        });
        return null;
      }
      case "get_repository_notes":
        return {
          repository_id: args.repositoryId,
          notes: [...this.notes.values()]
            .filter((n) => this.links.get(n.id)?.has(String(args.repositoryId)))
            .map((n) => this.summary(n)),
          tasks: [...this.tasks.values()].filter(
            (t) => t.repository_id === args.repositoryId,
          ),
          suggested: [],
        };
      case "preview_rename": {
        const n = this.noteOf(args.noteId);
        return {
          new_path: args.newPath,
          linking_notes: [] as NoteSummary[],
          note: n.id,
        };
      }
      case "rename_note": {
        const req = args.request as { note_id: string; new_path: string };
        const n = this.noteOf(req.note_id);
        n.path = req.new_path;
        this.later("note_changed", {
          note_id: n.id,
          version: n.version,
          relative_path: n.path,
          origin: "app",
        });
        return { note: this.summary(n), updated: [], failed: [] };
      }
      case "follow_note_title": {
        // The backend also leaves a note that other notes link to alone.
        const n = this.noteOf(args.noteId);
        const target = this.summary(n).title_file_name;
        if (
          !target ||
          !nameMatches(stemOf(n.path), fileNameFor(String(args.fromTitle)))
        )
          return this.summary(n);
        const folder = n.path.includes("/")
          ? n.path.replace(/\/[^/]*$/, "/")
          : "";
        const base = target.replace(/\.md$/, "");
        let path = `${folder}${base}.md`;
        for (
          let i = 2;
          [...this.notes.values()].some((o) => o.path === path);
          i++
        )
          path = `${folder}${base} ${i}.md`;
        n.path = path;
        this.later("note_changed", {
          note_id: n.id,
          version: n.version,
          relative_path: n.path,
          origin: "app",
        });
        return this.summary(n);
      }
      case "update_settings":
        Object.assign(settings, args.settings);
        return settings;
      case "rebuild_search":
        return null;
      case "get_agent_access_status":
        return {
          access: settings.agent_access,
          connections: settings.agent_access === "off" ? 0 : 1,
          executable: "/Applications/Brainiac.app/Contents/MacOS/brainiac",
          problem: null,
        };
      case "list_forge_accounts":
        return this.accounts;
      case "save_forge_account": {
        // A Bitbucket token that cannot write; any GitHub token is fine.
        const req = args.request as SaveForgeAccountRequest;
        const bitbucket = req.kind === "bitbucket_cloud";
        const missing = bitbucket ? ["write:pullrequest:bitbucket"] : [];
        if (missing.length && !req.read_only)
          return { outcome: "read_only", login: "jo", missing };
        const account = {
          kind: req.kind,
          login: bitbucket ? "jo" : "octo",
          user_id: bitbucket ? "{0a1b}" : "42",
          display_name: null,
          email: req.email,
          token_kind: bitbucket ? "api_token" : "fine_grained",
          expires_at: bitbucket ? null : "2027-10-02T22:00:00.000Z",
          scopes: bitbucket ? ["read:pullrequest:bitbucket"] : null,
          read_only: missing.length > 0,
          missing,
          checked_at: NOW,
        } as const;
        this.accounts = this.accounts.map((s) =>
          s.kind === req.kind ? { ...s, account, keychain_token: false } : s,
        );
        return { outcome: "saved", account };
      }
      case "remove_forge_account":
        this.accounts = this.accounts.map((s) =>
          s.kind === args.kind
            ? { kind: s.kind, account: null, keychain_token: false }
            : s,
        );
        return this.accounts;
      case "save_draft":
        this.drafts.set(String(args.noteId), {
          text: String(args.text),
          base_version: String(args.baseVersion),
        });
        return null;
      case "discard_draft":
        this.drafts.delete(String(args.noteId));
        return null;
      case "list_revisions":
        return [];
      case "list_trash":
        return [];
      case "set_pinned": {
        this.pins = this.pins.filter(
          (p) =>
            !(
              p.entity_type === args.entityType && p.entity_id === args.entityId
            ),
        );
        if (args.pinned)
          this.pins.push({
            entity_type: args.entityType as "note",
            entity_id: String(args.entityId),
            position: this.pins.length + 1,
          });
        return null;
      }
      case "list_tasks":
        return [...this.tasks.values()];
      case "get_task":
        return this.tasks.get(String(args.taskId));
      case "create_task": {
        const t = this.task(args.fields as TaskFields);
        this.tasks.set(t.id, t);
        this.later("task_changed", { task_id: t.id, version: t.version });
        return t;
      }
      case "update_task": {
        const req = args.request as {
          task_id: string;
          expected_version: number;
          fields: TaskFields;
        };
        const base = this.tasks.get(req.task_id);
        if (!base)
          throw {
            code: "NOT_FOUND",
            message: "That task does not exist.",
            retryable: false,
            details: null,
          };
        if (base.version !== req.expected_version)
          throw {
            code: "CONFLICT",
            message: "This task was changed elsewhere.",
            retryable: false,
            details: null,
          };
        const t = this.task(req.fields, base);
        this.tasks.set(t.id, t);
        this.later("task_changed", { task_id: t.id, version: t.version });
        return t;
      }
      case "get_today": {
        const all = [...this.tasks.values()];
        const open = all.filter(
          (t) =>
            (t.status === "todo" || t.status === "in_progress") &&
            ((t.due_date && t.due_date <= todayStr) ||
              (t.planned_date && t.planned_date <= todayStr)),
        );
        return {
          date: todayStr,
          open,
          completed: all.filter((t) => t.status === "done"),
          to_sort: all.filter((t) => t.to_sort),
          repository_ids: [
            ...new Set(
              open.flatMap((t) => (t.repository_id ? [t.repository_id] : [])),
            ),
          ],
        };
      }
      case "search": {
        const req = args.request as {
          query: string;
          kinds: Array<"note" | "task"> | null;
        };
        const q = req.query.toLowerCase();
        const wants = (k: "note" | "task") =>
          !req.kinds || req.kinds.includes(k);
        const notes =
          wants("note") && this.vault.vault
            ? [...this.notes.values()]
                .filter((n) => n.text.toLowerCase().includes(q))
                .map((n) =>
                  this.hit("note", n.id, this.summary(n).title, n.path),
                )
            : [];
        const tasks = wants("task")
          ? [...this.tasks.values()]
              .filter((t) => t.title.toLowerCase().includes(q))
              .map((t) => this.hit("task", t.id, t.title, ""))
          : [];
        return {
          query: req.query,
          notes: { hits: notes, total: notes.length },
          tasks: { hits: tasks, total: tasks.length },
          index: this.vault.index,
        };
      }
      case "list_changes":
        return {
          repository_id: repository.id,
          observed_at: NOW,
          head: repository.head,
          counts: repository.counts,
          entries: [],
        };
      case "open_repository":
      case "set_repository_tab":
        return null;
      case "plugin:dialog|open":
      case "plugin:dialog|save":
        return this.nextPick;
      case "plugin:dialog|ask":
      case "plugin:dialog|confirm":
        return this.nextAsk;
      case "plugin:updater|check":
        return null;
      default:
        if (cmd.startsWith("plugin:")) return null;
        throw {
          code: "VALIDATION",
          message: `The fake backend does not know ${cmd}.`,
          retryable: false,
          details: null,
        };
    }
  }
}
