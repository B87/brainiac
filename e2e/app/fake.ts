/**
 * An in-memory stand-in for Brainiac's backend, answering the commands the
 * frontend sends through `invoke`. It keeps notes, tasks, and a repository so
 * the app can be clicked through in WebKit without touching real data; the
 * backend's own behavior is tested by `cargo test`.
 */
import type {
  AgentSettings,
  AppSnapshot,
  Cell,
  Comment,
  CommentRequest,
  Conversation,
  CredentialOwner,
  DbConnection,
  FolderEntry,
  ForgeAccountSlot,
  HealthSample,
  ListPullRequestsRequest,
  MergeRequest,
  NoteContent,
  NoteSummary,
  PullRequest,
  PullRequestDiff,
  PullRequestDiffRequest,
  PullRequestList,
  QueryTab,
  ReplyRequest,
  RepositorySummary,
  ResolveThreadRequest,
  ReviewDraft,
  ReviewDrafts,
  RunStatementRequest,
  SaveDbConnectionRequest,
  SavedQuery,
  SaveForgeAccountRequest,
  SaveQueryRequest,
  SaveReviewDraftRequest,
  SearchHit,
  SecretsOverview,
  Settings,
  StatementRun,
  SubmitReviewRequest,
  Task,
  TaskFields,
  VaultState,
  Workspace,
  WriteOutcome,
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
  query_history: true,
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
  remote_url: "git@github.com:team/parser.git",
  forge: {
    kind: "github",
    owner: "team",
    name: "parser",
    reference: "github.com/team/parser",
    source: "origin",
  },
};

/** A workspace of the one repository; pull requests off until a test turns them on. */
export const workspace: Workspace = {
  id: "ws-1",
  name: "Team",
  discovery_mode: "manual",
  root_repository_id: null,
  discovery_root: null,
  discovery_path: null,
  members: [
    {
      origin: "manual",
      display_name: "parser",
      canonical_path: "/code/parser",
      repository_id: "repo-1",
      status: "ok",
    },
  ],
  activity: {
    watched_branches: ["main"],
    watched_tags: ["v*"],
    auto_fetch: false,
    notify_moves: false,
    morning_digest: false,
    warn_conflicts: true,
  },
  unseen_activity: 0,
  pull_requests: false,
};

const ALLOWED = { allowed: true, reason: null };
export const pullRequests: PullRequest[] = [
  {
    reference: "github.com/team/parser#12",
    number: 12,
    kind: "github",
    title: "Parse nested lists",
    description: "Handles **lists** inside lists.",
    description_html: "<p>Handles <strong>lists</strong> inside lists.</p>\n",
    author: { id: "7", login: "ada", display_name: "Ada" },
    state: "open",
    source_repository: "team/parser",
    source_branch: "nested-lists",
    head_sha: "a".repeat(40),
    base_sha: "b".repeat(40),
    reviewed_sha: "c".repeat(40),
    commits_since_review: 2,
    target_branch: "main",
    reviewers: [
      {
        user: { id: "42", login: "octo", display_name: "Octo Cat" },
        state: "requested",
        is_me: true,
      },
      {
        user: { id: "9", login: "bob", display_name: null },
        state: "approved",
        is_me: false,
      },
    ],
    checks: { state: "success", total: 2, passed: 2, failed: 0, pending: 0 },
    mergeability: "mergeable",
    counts: {
      comments: 1,
      unresolved_threads: 1,
      additions: 40,
      deletions: 3,
      changed_files: 2,
      commits: 3,
    },
    web_url: "https://github.com/team/parser/pull/12",
    created_at: "2026-09-30T10:00:00.000Z",
    updated_at: "2026-10-01T10:00:00.000Z",
    closed_at: null,
    version: "v1",
    actions: {
      comment: ALLOWED,
      review: ALLOWED,
      approve: ALLOWED,
      merge: ALLOWED,
      resolve: ALLOWED,
    },
    mine: false,
    awaiting_my_review: true,
  },
  {
    reference: "github.com/team/parser#13",
    number: 13,
    kind: "github",
    title: "Draft: faster tokenizer",
    description: "",
    description_html: "",
    author: { id: "42", login: "octo", display_name: "Octo Cat" },
    state: "draft",
    source_repository: "team/parser",
    source_branch: "main",
    head_sha: "b".repeat(40),
    base_sha: "b".repeat(40),
    reviewed_sha: null,
    commits_since_review: null,
    target_branch: "main",
    reviewers: [],
    checks: { state: null, total: 0, passed: 0, failed: 0, pending: 0 },
    mergeability: "computing",
    counts: {
      comments: 0,
      unresolved_threads: 0,
      additions: 5,
      deletions: 5,
      changed_files: 1,
      commits: 1,
    },
    web_url: "https://github.com/team/parser/pull/13",
    created_at: "2026-10-02T10:00:00.000Z",
    updated_at: "2026-10-02T12:00:00.000Z",
    closed_at: null,
    version: "v2",
    actions: {
      comment: ALLOWED,
      review: ALLOWED,
      approve: {
        allowed: false,
        reason: "GitHub does not let you approve your own pull request.",
      },
      merge: { allowed: false, reason: "A draft cannot be merged." },
      resolve: ALLOWED,
    },
    mine: true,
    awaiting_my_review: false,
  },
];

const octo = { id: "42", login: "octo", display_name: "Octo Cat" };
const ada = { id: "7", login: "ada", display_name: "Ada" };
const conversation: Conversation = {
  reference: "github.com/team/parser#12",
  threads: [
    {
      id: "T2",
      anchor: {
        path: "docs/lists.md",
        side: "new",
        line: 3,
        start_line: null,
        commit: null,
      },
      resolved: true,
      outdated: true,
      comments: [
        {
          id: "302",
          author: octo,
          body: "typo",
          html: "<p>typo</p>\n",
          review: null,
          created_at: "2026-09-30T12:00:00.000Z",
          updated_at: null,
          mine: true,
          web_url: null,
        },
      ],
    },
    {
      id: "T1",
      anchor: {
        path: "src/parse.ts",
        side: "new",
        line: 12,
        start_line: null,
        commit: "c".repeat(40),
      },
      resolved: false,
      outdated: false,
      comments: [
        {
          id: "300",
          author: octo,
          body: "Why not recurse here?",
          html: "<p>Why not recurse here?</p>\n",
          review: null,
          created_at: "2026-10-01T08:00:00.000Z",
          updated_at: "2026-10-01T08:30:00.000Z",
          mine: true,
          web_url: null,
        },
        {
          id: "301",
          author: ada,
          body: "Stack depth, see https://example.com/why",
          html: '<p>Stack depth, see <a href="https://example.com/why">https://example.com/why</a></p>\n',
          review: null,
          created_at: "2026-10-01T08:10:00.000Z",
          updated_at: null,
          mine: false,
          web_url: null,
        },
      ],
    },
    {
      id: "review:200",
      anchor: null,
      resolved: false,
      outdated: false,
      comments: [
        {
          id: "200",
          author: { id: "9", login: "bob", display_name: null },
          body: "LGTM",
          html: "<p>LGTM</p>\n",
          review: "approved",
          created_at: "2026-10-01T09:00:00.000Z",
          updated_at: null,
          mine: false,
          web_url: null,
        },
      ],
    },
  ],
  fetched_at: NOW,
};

const changedFiles = [
  {
    path: "src/parse.ts",
    old_path: null,
    status: "modified" as const,
    additions: 38,
    deletions: 3,
    binary: false,
  },
  {
    path: "docs/lists.md",
    old_path: "docs/list.md",
    status: "renamed" as const,
    additions: 2,
    deletions: 0,
    binary: false,
  },
  {
    path: "pnpm-lock.yaml",
    old_path: null,
    status: "modified" as const,
    additions: 120,
    deletions: 80,
    binary: false,
  },
];

function pullRequestDiff(req: PullRequestDiffRequest): PullRequestDiff {
  const local = req.path === "src/parse.ts";
  return {
    reference: req.reference,
    source: local ? "local" : "provider",
    base_sha: req.since ?? "b".repeat(40),
    head_sha: "a".repeat(40),
    diff: {
      repository_id: "repo-1",
      selector: {
        kind: "range",
        base: "b".repeat(40),
        head: "a".repeat(40),
        path: req.path,
        old_path: req.old_path,
      },
      content: {
        kind: "text",
        old_path: req.old_path,
        new_path: req.path,
        hunks: [
          {
            header: local ? "function parse()" : "",
            old_start: 10,
            old_lines: 2,
            new_start: 10,
            new_lines: 3,
            lines: [
              {
                kind: "context",
                old_no: 10,
                new_no: 10,
                text: "  if (open) {",
              },
              {
                kind: "delete",
                old_no: 11,
                new_no: null,
                text: "    push(item);",
              },
              {
                kind: "add",
                old_no: null,
                new_no: 11,
                text: "    push(parse(item));",
              },
              { kind: "add", old_no: null, new_no: 12, text: "    depth++;" },
            ],
          },
        ],
        truncated: false,
        total_lines: 4,
      },
    },
  };
}

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
  /** Settings → Agents: nothing set up yet, OrbStack running. */
  agentSettings: AgentSettings = {
    profile: {
      id: "claude-code",
      engine_socket: null,
      payment: "api_key",
      credential_source: { kind: "none" },
      credential: { needs_approval: false, pending: null, revision: 1 },
      credential_saved_at: null,
      credential_ageing: false,
      test_passed_at: null,
      test_current: false,
      sends_code_agreed: false,
      permissions: "ask",
      time_limit_minutes: 60,
      cpus: 4,
      memory_mib: 8192,
      workspace_gib: 20,
      model: "",
      image: null,
      version: 1,
    },
    plan_offered: true,
    missing: ["Choose where runs execute.", "Add an API key."],
  };
  /** Settings → Secrets, from the accounts and connections. */
  secrets(): SecretsOverview {
    const accounts = this.accounts.flatMap((s) =>
      s.account
        ? [
            {
              owner: { kind: "forge_account", provider: s.kind } as const,
              label: s.kind === "github" ? "GitHub" : "Bitbucket",
              destination: `${s.kind === "github" ? "api.github.com" : "api.bitbucket.org"} as ${s.account.login}`,
              source: s.account.token_source,
              state: s.account.credential,
              input_required: false,
              last_test: null,
            },
          ]
        : [],
    );
    const connections = this.dbConnections
      .filter((c) => c.password.kind !== "none")
      .map((c) => ({
        owner: { kind: "db_connection", id: c.id } as const,
        label: c.name,
        destination: `${c.host}:${c.port}/${c.database} as ${c.user}`,
        source: c.password,
        state: c.credential,
        input_required: c.password.kind === "ask" && !c.password_ready,
        last_test: null,
      }));
    return {
      store: "macOS login keychain",
      entries: [...accounts, ...connections],
    };
  }

  credentialOf(owner: CredentialOwner) {
    if (owner.kind === "forge_account")
      return this.accounts.find((s) => s.kind === owner.provider)?.account
        ?.credential;
    return this.dbConnections.find((c) => c.id === owner.id)?.credential;
  }

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
  /** Review drafts, per pull request reference. */
  reviewDraftRows: ReviewDraft[] = [];
  /** Conversations as written to; starts from the fixture. */
  conversations = new Map<string, Conversation>();

  conversationOf(reference: string): Conversation {
    let c = this.conversations.get(reference);
    if (!c) {
      c =
        reference === conversation.reference
          ? structuredClone(conversation)
          : { reference, threads: [], fetched_at: NOW };
      this.conversations.set(reference, c);
    }
    return c;
  }

  reviewDrafts(reference: string): ReviewDrafts {
    return {
      reference,
      drafts: this.reviewDraftRows.filter((d) => d.reference === reference),
      pending: null,
    };
  }

  myComment(id: string, body: string): Comment {
    return {
      id,
      author: octo,
      body,
      html: `<p>${body}</p>\n`,
      review: null,
      created_at: NOW,
      updated_at: null,
      mine: true,
      web_url: null,
    };
  }

  /** Fresh objects, as the real IPC deserializes them. */
  outcome(reference: string): WriteOutcome {
    return structuredClone({
      pull_request:
        pullRequests.find((p) => p.reference === reference) ?? pullRequests[0],
      conversation: this.conversationOf(reference),
    });
  }
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
      workspaces: [workspace],
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

  // --- Databases (v0.4) ---------------------------------------------------
  dbConnections: DbConnection[] = [];
  savedQueries: SavedQuery[] = [];
  queryTabs: QueryTab[] = [];
  /** Tabs with a transaction open, and how many statements ran in each. */
  openTransactions = new Map<string, number>();

  private dbConnection(req: SaveDbConnectionRequest): DbConnection {
    const base = this.dbConnections.find((c) => c.id === req.id);
    if (
      base &&
      req.expected_version !== null &&
      base.version !== req.expected_version
    )
      throw {
        code: "CONFLICT",
        message: "This connection was changed since the form opened.",
        retryable: false,
        details: null,
      };
    if (!req.name.trim())
      throw {
        code: "VALIDATION",
        message: "Name the connection.",
        retryable: false,
        details: null,
      };
    return {
      id: base?.id ?? uid("conn"),
      name: req.name.trim(),
      kind: req.kind,
      environment: req.environment,
      access: req.access,
      file_path: req.kind === "sqlite" ? req.file_path : null,
      host: req.kind === "postgres" ? req.host : null,
      port: req.kind === "postgres" ? (req.port ?? 5432) : null,
      database: req.kind === "postgres" ? req.database : null,
      user: req.kind === "postgres" ? req.user : null,
      tls: req.kind === "postgres" ? req.tls : null,
      ca_file: req.ca_file,
      password:
        req.kind === "postgres" ? req.password_source : { kind: "none" },
      password_ready: req.password_source.kind !== "ask",
      credential: {
        needs_approval: false,
        pending: null,
        revision: (base?.credential.revision ?? 0) + 1,
      },
      statement_timeout_seconds: req.statement_timeout_seconds,
      file_size: req.kind === "sqlite" ? 24576 : null,
      repository_ids: base?.repository_ids ?? [],
      runs_on: req.runs_on,
      version: (base?.version ?? 0) + 1,
    };
  }

  /** What the fake database answers: rows of invoices, a failure for
   * `nowhere`, a command for writes, and 5,000 rows for `generate_series`. */
  private dbRun(req: RunStatementRequest): StatementRun[] {
    const text = req.text;
    const statements: { from: number; to: number }[] = [];
    let start = 0;
    for (let i = 0; i <= text.length; i += 1) {
      if (i === text.length || text[i] === ";") {
        const raw = text.slice(start, i + (i < text.length ? 1 : 0));
        const lead = raw.length - raw.trimStart().length;
        if (raw.trim() && raw.trim() !== ";")
          statements.push({
            from: start + lead,
            to: start + raw.trimEnd().length,
          });
        start = i + 1;
      }
    }
    const chosen = req.all
      ? statements
      : req.from !== req.to
        ? statements.filter((s) => s.to > req.from && s.from < req.to)
        : [
            [...statements].reverse().find((s) => s.from <= req.from) ??
              statements[0],
          ].filter(Boolean);
    const runs: StatementRun[] = [];
    for (const s of chosen) {
      const sql = text.slice(s.from, s.to).replace(/;$/, "").trim();
      const lower = sql.toLowerCase();
      const tx = this.openTransactions.get(req.tab_id);
      let result: StatementRun["result"];
      let ranReadOnly = true;
      const at = lower.indexOf("nowhere");
      if (at >= 0) {
        result = {
          kind: "failed",
          failure: {
            reason: "sql",
            message: 'relation "nowhere" does not exist',
            code: "42P01",
            detail: null,
            hint: null,
            position: s.from + at,
          },
        };
      } else if (/^(insert|update|delete)/.test(lower)) {
        if (req.mode === "read_only") {
          result = {
            kind: "failed",
            failure: {
              reason: "read_only",
              message:
                "This connection is read only: cannot execute INSERT in a read-only transaction.",
              code: "25006",
              detail: null,
              hint: null,
              position: null,
            },
          };
        } else {
          ranReadOnly = false;
          result = {
            kind: "command",
            tag: lower.startsWith("insert") ? "INSERT 0 1" : "UPDATE 3",
          };
          if (req.mode === "manual")
            this.openTransactions.set(req.tab_id, (tx ?? 0) + 1);
        }
      } else if (req.explain) {
        result = {
          kind: "plan",
          plan: {
            label: "Seq Scan on invoices",
            startup_cost: 0,
            total_cost: 12.5,
            rows: 3,
            actual_rows: req.explain === "analyze" ? 3 : null,
            actual_ms: req.explain === "analyze" ? 0.02 : null,
            loops: req.explain === "analyze" ? 1 : null,
            details: ["Filter: (paid_at IS NULL)"],
            children: [],
          },
          planning_ms: 0.1,
          execution_ms: req.explain === "analyze" ? 0.05 : null,
        };
      } else if (lower.includes("generate_series")) {
        const cap = req.fetch_all ? 5000 : 1000;
        const rows: Cell[][] = Array.from(
          { length: Math.min(cap, 5000) },
          (_, i) => [i + 1],
        );
        result = {
          kind: "rows",
          columns: [{ name: "n", type_name: "int4", kind: "number" }],
          rows,
          more: !req.fetch_all,
        };
      } else {
        const days = req.parameters.find((p) => p.name === "days")?.value;
        result = {
          kind: "rows",
          columns: [
            { name: "id", type_name: "int4", kind: "number" },
            { name: "email", type_name: "text", kind: "text" },
            { name: "amount", type_name: "numeric", kind: "numeric" },
            { name: "metadata", type_name: "jsonb", kind: "json" },
          ],
          rows: [
            [917, "a@example.com", "120.00", '{"tier": "gold"}'],
            [921, "b@example.com", "80.50", null],
            [930, days ? `overdue ${days} days` : "NULL", "5.00", "{}"],
          ],
          more: false,
        };
      }
      const open = this.openTransactions.get(req.tab_id);
      runs.push({
        from: s.from,
        to: s.to,
        sql,
        result,
        elapsed_ms: 18,
        reconnected: false,
        ran_read_only: ranReadOnly,
        transaction:
          open !== undefined
            ? { statements: open, since: NOW, failed: false }
            : null,
      });
      if (result.kind === "failed") break;
    }
    return runs;
  }

  healthSample(connectionId: string): HealthSample {
    return {
      connection_id: connectionId,
      at: NOW,
      max_connections: 100,
      active: 3,
      idle: 30,
      idle_in_transaction: 1,
      total_connections: 34,
      cache_hit_ratio: 0.993,
      transactions_per_second: 41.2,
      rollbacks_per_second: 0.1,
      deadlocks: 0,
      temp_files_hour: 2,
      longest_transaction_seconds: 125,
      sessions: [
        {
          pid: 4242,
          user: "app",
          application: "billing-worker",
          client: "10.0.0.5",
          state: "idle in transaction",
          state_seconds: 125,
          transaction_seconds: 130,
          wait: null,
          blocked_by: [],
          query: "update invoices set paid_at = now() where id = $1",
          own: false,
        },
        {
          pid: 4300,
          user: "report",
          application: "metabase",
          client: "10.0.0.9",
          state: "active",
          state_seconds: 12,
          transaction_seconds: 12,
          wait: "Lock: transactionid",
          blocked_by: [4242],
          query: "select count(*) from invoices",
          own: false,
        },
      ],
      statements: null,
      tables: [
        {
          schema: "public",
          name: "invoices",
          total_bytes: 52428800,
          live_rows: 48000,
          dead_ratio: 0.04,
          last_autovacuum: NOW,
        },
      ],
      sees_all: true,
      machine: null,
      problem: null,
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
      case "get_agent_settings":
        return this.agentSettings;
      case "list_agent_engines":
        return [
          {
            socket: "/Users/someone/.orbstack/run/docker.sock",
            name: "OrbStack",
            reachable: true,
            supported: true,
            problem: null,
            server_version: "28.3.2",
            api_version: "1.51",
            cpus: 8,
            memory_bytes: 16 * 1024 ** 3,
          },
        ];
      case "save_agent_settings": {
        const { expected_version: _, ...fields } = args.request as Record<
          string,
          unknown
        >;
        const profile = this.agentSettings.profile;
        Object.assign(profile, fields, { version: profile.version + 1 });
        this.agentSettings.missing = profile.engine_socket
          ? ["Add an API key."]
          : this.agentSettings.missing;
        return this.agentSettings;
      }
      case "save_agent_credential": {
        const request = args.request as {
          payment: "claude_plan" | "api_key";
          source: AgentSettings["profile"]["credential_source"];
        };
        const profile = this.agentSettings.profile;
        Object.assign(profile, {
          payment: request.payment,
          credential_source: request.source,
          credential_saved_at: "2026-10-05T12:00:00Z",
          version: profile.version + 1,
        });
        this.agentSettings.missing = [];
        return this.agentSettings;
      }
      case "agent_dockerfile":
        return "FROM node:22-bookworm-slim\n";
      case "list_agent_runs":
        return { runs: [], controller_running: false };
      case "get_run_controller_status":
        return { running: false, pid: null, live_runs: 0 };
      case "test_agent_setup":
        return {
          steps: [{ name: "Start a run", passed: false, detail: "No engine" }],
          passed: false,
          tested_at: NOW,
        };
      case "list_forge_accounts":
        return this.accounts;
      case "update_workspace_pull_requests":
        workspace.pull_requests = !!args.enabled;
        return workspace;
      case "get_review_counts":
        return workspace.pull_requests
          ? [
              {
                workspace_id: workspace.id,
                awaiting: pullRequests.filter(
                  (p) =>
                    p.awaiting_my_review &&
                    (p.state === "open" || p.state === "draft"),
                ).length,
              },
            ]
          : [];
      case "set_repository_forge": {
        const forge = (
          args.request as {
            forge: { kind: "github"; owner: string; name: string } | null;
          }
        ).forge;
        repository.forge = forge
          ? {
              ...forge,
              reference: `github.com/${forge.owner}/${forge.name}`,
              source: "override",
            }
          : {
              kind: "github",
              owner: "team",
              name: "parser",
              reference: "github.com/team/parser",
              source: "origin",
            };
        return repository;
      }
      case "list_pull_requests": {
        const req = args.request as ListPullRequestsRequest;
        const enabled = workspace.pull_requests;
        const list: PullRequestList = {
          enabled,
          tracked_by: req.repository_id && enabled ? ["Team"] : [],
          missing_accounts: this.accounts[0].account ? [] : ["github"],
          groups: enabled
            ? [
                {
                  repository_id: "repo-1",
                  repository_name: "parser",
                  forge: "github.com/team/parser",
                  kind: "github",
                  pull_requests: req.closed ? [] : pullRequests,
                  fetched_at: NOW,
                  error: null,
                },
              ]
            : [],
          budgets: [
            {
              kind: "github",
              used: 12,
              limit: 5000,
              resets_at: null,
              retry_at: null,
            },
          ],
        };
        return list;
      }
      case "get_pull_request":
        return (
          pullRequests.find((p) => p.reference === args.reference) ??
          pullRequests[0]
        );
      case "list_pull_request_files":
        return {
          reference: args.reference,
          head_sha: "a".repeat(40),
          base_sha: args.since ?? "b".repeat(40),
          partial: !!args.since,
          files: args.since ? changedFiles.slice(0, 1) : changedFiles,
          fetched_at: NOW,
        };
      case "get_pull_request_conversation":
        return structuredClone(this.conversationOf(String(args.reference)));
      case "get_pull_request_repository":
        return repository;
      // Reviewing (SPEC.md, Reviewing): drafts stay here; the other writes
      // land in the conversation at once.
      case "list_review_drafts":
        return this.reviewDrafts(String(args.reference));
      case "save_review_draft": {
        const req = args.request as SaveReviewDraftRequest;
        const existing = req.id
          ? this.reviewDraftRows.find((d) => d.id === req.id)
          : undefined;
        if (existing) {
          existing.body = req.body;
          existing.html = `<p>${req.body}</p>\n`;
          existing.anchor = req.anchor;
        } else
          this.reviewDraftRows.push({
            id: uid("draft"),
            reference: req.reference,
            anchor: req.anchor,
            body: req.body,
            html: `<p>${req.body}</p>\n`,
            remote_id: null,
            created_at: NOW,
            updated_at: NOW,
          });
        return this.reviewDrafts(req.reference);
      }
      case "delete_review_draft":
        this.reviewDraftRows = this.reviewDraftRows.filter(
          (d) => d.id !== args.id,
        );
        return this.reviewDrafts(String(args.reference));
      case "move_review_drafts":
        for (const d of this.reviewDraftRows)
          if (d.reference === args.reference)
            d.anchor = { ...d.anchor, commit: String(args.headSha) };
        return this.reviewDrafts(String(args.reference));
      case "comment_on_pull_request": {
        const req = args.request as CommentRequest;
        const c = this.conversationOf(req.reference);
        const id = uid("c");
        c.threads.push({
          id: `comment:${id}`,
          anchor: null,
          resolved: false,
          outdated: false,
          comments: [this.myComment(id, req.body)],
        });
        return this.outcome(req.reference);
      }
      case "reply_to_thread": {
        const req = args.request as ReplyRequest;
        const c = this.conversationOf(req.reference);
        const t = c.threads.find((t) => t.id === req.thread_id);
        if (!t) throw { code: "NOT_FOUND", message: "The thread is gone." };
        t.comments.push(this.myComment(uid("c"), req.body));
        return this.outcome(req.reference);
      }
      case "resolve_thread": {
        const req = args.request as ResolveThreadRequest;
        const c = this.conversationOf(req.reference);
        const t = c.threads.find((t) => t.id === req.thread_id);
        if (!t) throw { code: "NOT_FOUND", message: "The thread is gone." };
        t.resolved = req.resolved;
        return this.outcome(req.reference);
      }
      case "submit_review": {
        const req = args.request as SubmitReviewRequest;
        const pr = pullRequests.find((p) => p.reference === req.reference);
        if (pr && pr.head_sha !== req.expected_head_sha)
          throw {
            code: "CONFLICT",
            message:
              "New commits arrived since you looked at this pull request.",
          };
        const c = this.conversationOf(req.reference);
        for (const d of this.reviewDraftRows.filter(
          (d) => d.reference === req.reference,
        ))
          c.threads.push({
            id: uid("T"),
            anchor: d.anchor,
            resolved: false,
            outdated: false,
            comments: [this.myComment(uid("c"), d.body)],
          });
        const id = uid("r");
        c.threads.push({
          id: `review:${id}`,
          anchor: null,
          resolved: false,
          outdated: false,
          comments: [
            {
              ...this.myComment(id, req.body),
              review:
                req.verdict === "approve"
                  ? "approved"
                  : req.verdict === "request_changes"
                    ? "changes_requested"
                    : null,
            },
          ],
        });
        this.reviewDraftRows = this.reviewDraftRows.filter(
          (d) => d.reference !== req.reference,
        );
        if (pr && req.verdict === "approve")
          pr.reviewers = pr.reviewers.map((r) =>
            r.is_me ? { ...r, state: "approved" } : r,
          );
        return this.outcome(req.reference);
      }
      case "get_merge_options":
        return {
          reference: args.reference,
          methods: ["merge_commit", "squash", "rebase"],
          default_method: "squash",
          can_delete_branch: true,
          delete_branch: true,
          deletes_branch_itself: false,
        };
      case "merge_pull_request": {
        const req = args.request as MergeRequest;
        const pr = pullRequests.find((p) => p.reference === req.reference);
        if (!pr) throw { code: "NOT_FOUND", message: "No such pull request." };
        if (pr.head_sha !== req.expected_head_sha)
          throw {
            code: "CONFLICT",
            message:
              "New commits arrived since you looked at this pull request. Nothing was merged.",
          };
        pr.state = "merged";
        pr.closed_at = NOW;
        pr.actions = {
          ...pr.actions,
          merge: { allowed: false, reason: "The pull request is merged." },
          review: { allowed: false, reason: "The pull request is merged." },
          approve: { allowed: false, reason: "The pull request is merged." },
        };
        // The service emits the committed change after every write.
        this.later("pr_changed", {
          reference: req.reference,
          version: pr.version,
          origin: "app",
        });
        return { ...this.outcome(req.reference), warning: null };
      }
      case "get_pull_request_diff":
        return pullRequestDiff(args.request as PullRequestDiffRequest);
      case "get_pull_request_checks":
        return {
          reference: args.reference,
          head_sha: "a".repeat(40),
          checks: [
            {
              name: "build",
              state: "success",
              description: null,
              url: "https://ci.example.com/1",
            },
            { name: "lint", state: "success", description: "clean", url: null },
          ],
          fetched_at: NOW,
        };
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
          token_source: req.source,
          credential: { needs_approval: false, pending: null, revision: 1 },
        } as const;
        this.accounts = this.accounts.map((s) =>
          s.kind === req.kind ? { ...s, account, keychain_token: false } : s,
        );
        return { outcome: "saved", account, warning: null };
      }
      case "test_forge_account": {
        const req = args.request as SaveForgeAccountRequest;
        const bitbucket = req.kind === "bitbucket_cloud";
        return {
          login: bitbucket ? "jo" : "octo",
          missing: bitbucket ? ["write:pullrequest:bitbucket"] : [],
        };
      }
      case "list_secrets":
        return this.secrets();
      case "approve_secret_source": {
        const owner = args.owner as CredentialOwner;
        const credential = this.credentialOf(owner);
        if (credential) credential.needs_approval = false;
        return this.secrets();
      }
      case "refresh_credential":
        return null;
      case "retry_credential_cleanup": {
        const credential = this.credentialOf(args.owner as CredentialOwner);
        if (credential) credential.pending = null;
        return this.secrets();
      }
      case "find_secret_program":
        return String(args.name).startsWith("/")
          ? args.name
          : `/opt/homebrew/bin/${args.name}`;
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
      case "list_db_connections":
        return this.dbConnections;
      case "save_db_connection": {
        const c = this.dbConnection(args.request as SaveDbConnectionRequest);
        this.dbConnections = [
          ...this.dbConnections.filter((x) => x.id !== c.id),
          c,
        ].sort((a, b) => a.name.localeCompare(b.name));
        return c;
      }
      case "delete_db_connection":
        this.dbConnections = this.dbConnections.filter((c) => c.id !== args.id);
        return null;
      case "test_db_connection":
        return { server_version: "PostgreSQL 16.4" };
      case "parse_db_url": {
        const m =
          /^postgres(?:ql)?:\/\/(?:([^:@]+)(?::([^@]*))?@)?([^:/]+)(?::(\d+))?\/?([^?]*)/.exec(
            String(args.url),
          );
        return {
          host: m?.[3] ?? null,
          port: m?.[4] ? Number(m[4]) : null,
          database: m?.[5] || null,
          user: m?.[1] ?? null,
          password: m?.[2] ?? null,
          tls: null,
        };
      }
      case "unlock_db_connection": {
        const c = this.dbConnections.find((x) => x.id === args.id);
        if (c) c.password_ready = true;
        return c;
      }
      case "get_db_schema":
        return {
          connection_id: args.connectionId,
          default_schema: "public",
          read_at: NOW,
          schemas: [
            {
              name: "public",
              relations: ["customers", "invoices"].map((name) => ({
                name,
                kind: "table",
                estimated_rows: name === "invoices" ? 48000 : 12000,
                columns: [
                  {
                    name: "id",
                    type_name: "integer",
                    nullable: false,
                    default: null,
                    primary_key: true,
                  },
                  {
                    name: "email",
                    type_name: "text",
                    nullable: true,
                    default: null,
                    primary_key: false,
                  },
                ],
                indexes: [
                  {
                    name: `${name}_pkey`,
                    definition: "",
                    unique: true,
                    primary: true,
                  },
                ],
                foreign_keys: [],
              })),
            },
          ],
        };
      case "statement_parameters": {
        const req = args.request as RunStatementRequest;
        return [
          ...new Set(
            [...req.text.matchAll(/(?<!:):([a-z_]\w*)/g)].map((m) => m[1]),
          ),
        ];
      }
      case "run_statement":
        return this.dbRun(args.request as RunStatementRequest);
      case "end_transaction":
        this.openTransactions.delete(String(args.tabId));
        return null;
      case "open_db_transactions":
        return [...this.openTransactions.keys()];
      case "cancel_statement":
      case "close_db_session":
      case "stop_db_health":
      case "clear_query_history":
        return null;
      case "export_result":
        return { rows: 3, path: (args.request as { path: string }).path };
      case "list_saved_queries":
        return this.savedQueries;
      case "save_query": {
        const req = args.request as SaveQueryRequest;
        const base = this.savedQueries.find((q) => q.id === req.id);
        const q: SavedQuery = {
          id: base?.id ?? uid("query"),
          name: req.name.trim(),
          folder: req.folder.trim(),
          description: req.description,
          connection_id: req.connection_id,
          sql: req.sql,
          parameters: base?.parameters ?? [],
          version: (base?.version ?? 0) + 1,
          updated_at: NOW,
        };
        this.savedQueries = [
          ...this.savedQueries.filter((x) => x.id !== q.id),
          q,
        ];
        return q;
      }
      case "delete_query":
        this.savedQueries = this.savedQueries.filter((q) => q.id !== args.id);
        return null;
      case "remember_query_parameters": {
        const q = this.savedQueries.find((x) => x.id === args.id);
        if (q) q.parameters = args.values as SavedQuery["parameters"];
        return q;
      }
      case "query_history":
        return [];
      case "list_query_tabs":
        return this.queryTabs;
      case "save_query_tabs":
        this.queryTabs = args.tabs as QueryTab[];
        return null;
      case "start_db_health":
        return {
          points: [],
          latest: this.healthSample(String(args.connectionId)),
        };
      case "signal_db_backend":
        return true;
      case "list_docker_containers":
        return { socket: "/var/run/docker.sock", containers: [] };
      case "plugin:dialog|open":
      case "plugin:dialog|save":
        return this.nextPick;
      case "plugin:dialog|ask":
      case "plugin:dialog|confirm":
        return this.nextAsk;
      case "plugin:dialog|message": {
        // `ask` with its own labels answers with the label of the button pressed.
        const buttons = args.buttons as
          | { OkCancelCustom?: [string, string] }
          | string
          | undefined;
        const custom =
          typeof buttons === "object" ? buttons.OkCancelCustom : undefined;
        if (custom) return this.nextAsk ? custom[0] : custom[1];
        return this.nextAsk ? "Yes" : "No";
      }
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
