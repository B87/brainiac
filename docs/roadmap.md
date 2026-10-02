# Roadmap

Milestones, open questions, and the designs of releases after the current one. When work on a release starts, its design moves into [`SPEC.md`](../SPEC.md) and its architecture into [`architecture.md`](architecture.md); this file keeps only its checklist.

## Milestones

Milestones use working-software exit gates rather than fixed calendar promises. Complete and use each release before expanding scope.

### M0 — Validate the Git foundations

- [x] Scaffold Tauri v2 + React/TypeScript; verify dev and packaged builds.
- [x] Open the main window with macOS menus and keyboard navigation (App, File, Edit, View, Window menus; ⌘O, ⌘R, ⌘K wired through menu events).
- [x] Prove Rust command/error DTOs and stale-response handling (`ts-rs` bindings in `src/lib/generated/`, `AppError` codes, `createLatest` guard with tests).
- [x] Detect the system Git binary; resolve a repository and linked worktree correctly.
- [x] Read status, history, and one bounded text diff through the Rust Git module.
- [x] Prove SQLite workspace/settings persistence, migrations, and backup.
- [x] Exercise repository notifications against staged and unstaged changes (integration test observes an unstaged edit with no `.git` change).

M0 landed on 1 October 2026 with 58 Rust tests and 3 frontend tests. Known M0 simplifications to revisit in v0.1: diff output is not virtualized, commit details show metadata only (no changed-file list), the Branches tab is a placeholder, and the periodic refresh runs regardless of dashboard visibility. The first three were resolved after 0.1.0 (diffs over 1,000 rows are windowed with fixed-height rows).

**Exit gate:** a packaged app opens a local repository, shows its current changes and history, displays a selected diff, and refreshes after an external edit without modifying Git state.

### v0.1 — Git viewer and repository tracker

- [x] Add/open one repository directly; persist recent/pinned repositories.
- [ ] Relocate a repository whose folder moved (today: remove and add again).
- [x] Create a workspace manually or by discovering independent repositories inside a chosen folder, with optional root grouping and rescan.
- [x] Let the user choose which discovered repositories to track and manage membership independently of their IDE.
- [x] Aggregate status table, unique changed-file counts, dirty/conflicted/stale filters, partial-error handling.
- [x] Changes list with staged/unstaged diffs, untracked previews, conflict and binary/large-file states.
- [x] Paginated commit history, message/hash filtering, metadata, changed files, and per-file commit patches.
- [x] Read-only branch/tag lists and ref-scoped history selection.
- [x] Debounced tracking, bounded subprocesses, manual refresh, wake/activation reconciliation.
- [x] Repository/workspace command palette, copy hashes/paths, Open in editor, Reveal in Finder.
- [x] Fetch now and opt-in per-workspace auto-fetch of watched branches, with lock detection, backoff, and non-interactive credentials.
- [x] Workspace Activity tab: watched refs, advanced/rewritten/created/tagged events, unread state, conflict-risk and drift warnings, team pulse, fetch freshness, optional notifications and morning digest.
- [x] Viewer refinements: line counts in Changes, Both comparison, ignore whitespace, split layout, word highlights, hunk navigation, author filter, comparison with the default branch, keyboard shortcuts, AA contrast.
- [x] SQLite snapshot backup of settings/workspaces (export manifest is v0.2); no note vault, account, FTS index, or model required.

**Exit gate:** use the app for a week with a standalone repository, a manual workspace, and a discovered workspace whose folder is itself a repository. Identify dirty/conflicted repos, inspect staged and unstaged patches, navigate history and refs, observe external changes, see a teammate's merge in the Activity tab after a fetch, retain registrations after restart, and recover from one slow/missing repo while the rest remain usable. Confirm viewing does not alter working files, refs, or the index, and that fetching changes only remote-tracking refs and tags.

**Topology acceptance:** select a folder once and discover the repositories directly inside its discovery folder, both when the discovery folder is the selected folder and when it is a subfolder, and both when the selected folder is a repository and when it is not. Root and member changes remain separately attributed even when the root ignores the discovery folder. Adding/removing a child is detected, plain child folders do not become false repositories, and repeated discovery/manual selection does not duplicate registrations. Include independent nested repos, a linked worktree, and an actual submodule in fixtures.

**Setup acceptance:** configure the root plus the selected project repositories through folder discovery/selection, persist that selection across restart, and inspect all of them without reading or depending on the IDE workspace file.

### v0.2 — Knowledge, tasks, and code context

- [ ] Vault onboarding, scan/reconciliation, folder navigation, pinned/recent notes.
- [ ] Editor fidelity spike; note CRUD, auto-save, source fallback, conflicts, trash, draft/history recovery.
- [ ] Tasks with status, planned dates, deadlines, linked note and linked repository; tasks not yet sorted are marked as such.
- [ ] Today, Tasks, Notes alongside the existing Workspaces/Git views; tasks to sort appear as a folded line in Today and a filter in Tasks.
- [ ] FTS5 note/task ingestion, ranked keyword search, safe snippets, rebuild status.
- [ ] Note/repository associations, standard Markdown links/backlinks, unresolved targets.
- [ ] Complete vault/database export and restore preserving tasks and associations.
- [ ] One write path for every caller: each write goes through a domain service that emits the committed change event; task writes carry the expected version; a note changed outside Brainiac keeps its previous text as a revision.

**Exit gate:** use Brainiac for real notes and daily tasks alongside Git tracking; edits survive restart, knowledge search works offline, Today behaves correctly across midnight, and restore recovers files, tasks, and context links.

### v0.2.x — Agent access

- [ ] Local MCP server inside the running app, reached through a small `brainiac-mcp` stdio helper; off by default, read-only or read-write.
- [ ] Tools for search, notes, tasks, and repository links that call the same domain services as the UI.

**Exit gate:** an agent such as Claude Code finds notes, creates and completes a task, and links a note to a repository while the app is open; the UI updates live, a concurrent UI edit produces a conflict rather than an overwrite, and the agent's note edits can be undone from revision history.

### v0.3 — Content imports and global capture

- [ ] Inbox view for captured and imported items, with triage: file or move, link to a task or repository, create a task.
- [ ] Text/selection capture, URL bookmarks, Markdown copies, source provenance, resumable import jobs.
- [ ] Public article extraction with bookmark/paste fallback.
- [ ] `.eml` import, safe body conversion, message identity, original retention and export/restore.
- [ ] Duplicate/update preview, personal-annotation preservation, source filters, content-scope labels.
- [ ] Video reference and supplied timestamped transcript support; validate richer transcript acquisition separately.
- [ ] Configurable global shortcut, floating capture, menu bar entry, background lifetime preference.
- [ ] Shared backend state, snapshot recovery, capture draft retention, macOS focus tests.

**Exit gate:** import representative articles/emails/issue excerpts/video references, find their captured content offline, identify metadata-only items, preserve annotations, retry failed imports without duplicates, and capture from another app without losing drafts or main-window updates.

**v0.3.x follow-up:** add one authenticated source adapter chosen by daily use, Jira or the email provider. Validate its authentication/deployment and selected-item import before expanding providers. Native share/browser entry points and a forwarding service remain separate follow-ups.

### v0.4 — Semantic retrieval

- [ ] Validate sqlite-vec/Rust/bundled-SQLite release integration.
- [ ] Choose and document a local embedding profile using actual notes.
- [ ] Chunk/hash/version pipeline, bounded indexing, progress, cancellation.
- [ ] Vector retrieval, hybrid ranking, model-change reindexing.

**Exit gate:** a labeled set of roughly 30 real queries demonstrates useful retrieval beyond keyword search; deleted/stale chunks are excluded and Ollama failure leaves core search functional.

### v0.5 — Grounded AI answers

- [ ] Context assembly, generation model configuration, streamed channel output.
- [ ] Openable citations, stale-source labeling, insufficient-evidence behavior.
- [ ] Cancellation/timeouts and explicit Save as note.

**Exit gate:** representative questions produce answers supported by the displayed notes, empty-evidence cases are handled honestly, and no generated output writes files or executes commands automatically.

### Later — Life management and integrations

Expand the v0.3 import adapters and consider GitHub/GitLab PR and CI summaries; Linear references; local calendar context; recurring tasks and reminders; daily/weekly review templates; optional graph navigation; explicit branch/dev-server actions; multiple vaults; sync; automation or plugin APIs.

Remote services require their own authentication, rate-limit, cache, error, and privacy requirements. Polling with backoff is sufficient for an initial desktop integration; webhooks need an explicitly designed delivery mechanism. Do not make these dependencies of the core app.

## Open questions

| Unknown | Resolution method | Needed before |
| --- | --- | --- |
| Rich-editor Markdown fidelity | Round-trip fixture suite on existing notes | v0.2 editor commitment |
| Actual note languages and syntax extensions | Sample a representative vault; document unsupported constructs | v0.2 compatibility claim |
| macOS minimum and Intel requirement | Verify target hardware, dependencies, and packaged builds | Release distribution |
| Real-world workspace size and Git cost | Benchmark representative workspace/repositories | v0.1 watcher tuning |
| Quick-capture focus behavior | macOS/Spaces/fullscreen prototype | v0.3 |
| Vault size and note sizes | Count notes, largest file, and full-ingest time on a real vault | v0.2 batch sizes and revision budget |
| Snapshot size and retention | Measure `brainiac.db` and `history.db` on a real vault after a month | v0.2 backup defaults |
| Restore onto another Mac | Fixture: vault and snapshot restored where paths and repository locations differ | v0.2 restore |
| Embedding model and dimensions | Local retrieval evaluation, model availability and license review | v0.4 schema/profile selection |
| Chunking notes, code, and imports | Labelled set of about 30 real queries; compare heading-aware splits, size caps, and overlap by recall@k | v0.4 chunker version 1 |
| SQLite-vector integration compatibility | Build and query a bundled release executable | v0.4 |
| `sqlite-vec` approximate indexes | Track whether ANN indexes reach a stable release; brute force is enough at personal scale | Only if a vault outgrows brute force |
| Remote integrations and company data constraints | Choose the first provider and its allowed scopes | Integration release |
| Preferred mail source and capture gesture | Choose `.eml`, connected mailbox, or optional forwarding workflow | Email connector implementation |
| Jira deployment/authentication | Confirm Cloud versus Data Center and company-supported access | Jira connector implementation |
| Article extraction and transcript availability | Evaluate representative pages/videos; document fallback coverage | Rich import compatibility claims |
| Sync expectations and encryption | Separate design; define conflicts and authoritative stores | Any sync implementation |

These unknowns do not prevent implementing the core domain and persistence services. The first concrete development step is M0, followed by the smallest complete open repository → inspect status → view diff/history → track changes workflow.

## Designs for later releases

### v0.2 additions to views, storage, and contracts

#### Knowledge views — v0.2

Add Today, Tasks, and Notes (vault tree/editor) alongside the existing Workspaces view. Tasks link to notes and repositories. The section is called Notes, not Brain: "brain" names the whole app, as in **Add to brain** and **Ask my brain**.

v0.2 has no separate Inbox view. An inbox earns its place when items arrive faster than they are sorted, which starts with v0.3's capture from other apps and imports; in v0.2 everything is created inside the app, at a moment when the user can give it a date or not. Until then:

- A task is *to sort* until it gets a planned date, a deadline, or an explicit **Sorted** (`triaged_at` set). Today shows these as one folded **To sort · N** line above its sections; expanding it lists them. Tasks has a **To sort** filter.
- Quick notes go to an ordinary `Inbox/` folder in the vault, which stays usable outside Brainiac. v0.3's Inbox view lists that folder's notes alongside imported items.

Today shows open tasks planned for today, due today, or overdue, with completed items in a separate section. It uses local calendar dates. Planning a task and setting its deadline remain separate actions. Search finds saved notes and tasks with excerpts.

#### Editor decision gate — v0.2

TipTap's Markdown support documents limitations. Treat fidelity as a release gate rather than assuming any Markdown file can be serialized without loss. [TipTap Markdown documentation](https://tiptap.dev/docs/editor/markdown)

- Test headings, lists, checkboxes, tables, fenced code, links, images, frontmatter, HTML, and unknown syntax.
- Keep frontmatter separate from the rich document and preserve unknown keys.
- Opening a note without editing must never rewrite it.
- Unsupported or lossy constructs use source mode, preserving original text.
- If the rich editor cannot pass the fixture suite, ship v0.2 with source editing and preview; add rich editing when it meets the contract.

#### Note identity and compatibility — v0.2

- New notes receive a UUID in a namespaced frontmatter key, `brainiac_id`.
- Existing notes with no such key receive a persisted application ID without modifying the file. Brainiac writes `brainiac_id` into an existing note the first time it gets a task or repository link (a setting, on by default), because those notes carry context that a rename must not lose; otherwise adding it is an explicit action. Opening or indexing a note never writes it.
- The ID is a convention the app cannot enforce: a copied note duplicates it and an edit can remove it. Handle both as described below rather than trusting it blindly. Dendron relies on the same convention for notes moved outside the app. [Dendron FAQ](https://wiki.dendron.so/notes/683740e3-70ce-4a47-a1f4-1f140e80b558/)
- Title comes from frontmatter `title`, then the first H1, then the filename.
- App-driven renames preserve identity. External moves use embedded IDs where available; otherwise match the same relative path, then a unique content hash within the reconciliation batch.
- A note's identity is its vault ID plus its vault-relative path, never an absolute path, so a vault can move or be restored elsewhere.
- Renaming a note in Brainiac offers to update links to it in other notes, listing the files; the option is off by default because it edits other files.
- Ambiguous moves become missing/new entries and can be relinked; identical contents alone do not prove identity.
- Duplicate embedded IDs produce a visible conflict; never merge two notes silently.
- Preserve arbitrary frontmatter keys, code fences, and relative links. Changing a title does not automatically rename the file.
- In v0.2, supported notes are UTF-8 `.md` files on a local filesystem. Unsupported encoding and files over the proposed 5 MiB editing limit receive a clear message and an Open Externally action.

#### Shortcuts

| Shortcut | Action |
| --- | --- |
| `Cmd+N`, v0.2 | New note |
| `Cmd+Shift+N`, v0.2 | New task |
| `Cmd+S`, v0.2 | Save note immediately |
| `Cmd+Shift+Space`, v0.3 | Configurable global quick capture |
| `Cmd+K` | Knowledge search joins the repository/workspace palette |

#### Storage layout — v0.2

Only two things cannot be rebuilt: the vault's Markdown files and a small core database. Everything else is a cache that can be discarded and rebuilt from them, which decides where data lives, what is backed up, and how restore works.

| File | Holds | Rebuildable | Backed up |
| --- | --- | --- | --- |
| Vault (`.md` files) | Note text, frontmatter including `brainiac_id`, links written in notes | Source of truth | By the user, and in Brainiac's export |
| `brainiac.db` | Settings, repositories, workspaces, pins, activity, vaults, note identity, tasks and their search table, note-to-repository links, embedding profiles | No | Daily snapshots and export |
| `index.db` | Note bodies, the notes search table, parsed links between notes, chunks and vectors (v0.4) | Yes, from the vault | Never; rebuilt after a restore |
| `history.db` | Note revisions and draft checkpoints | No, but optional | Its own snapshots, less often than `brainiac.db` |

- Under WAL, a transaction across attached database files is atomic within each file but not across them; a crash during commit can leave one file updated and another not. [SQLite ATTACH](https://www.sqlite.org/lang_attach.html) Splitting is therefore safe only because `index.db` is rebuildable: each indexed row records the content hash it was built from, and startup re-indexes rows whose hash no longer matches `brainiac.db`.
- The split keeps snapshots small (seven daily copies of the core, not of every note body and revision) and lets indexing write to `index.db` on its own connection instead of queueing behind Git queries on the core database's worker.
- Note-to-repository links have no foreign key to `repositories`: removing a repository keeps the link, shown as a removed repository, with the name and remote URL copied onto the link when it was made. Re-adding a repository with the same remote offers to reconnect it. `repositories` gains a `remote_url` column (the `origin` fetch URL, refreshed on observation).

#### Data model of later releases

| Entity | File | Essential fields and constraints | Release |
| --- | --- | --- | --- |
| `vaults` | core | `id`, `name`, `root_path`; one active vault initially | v0.2 |
| `notes` | core | `id`, `vault_id`, `relative_path`, `embedded_id?`, `title`, `content_hash`, `mtime`, `last_opened_at?`, `missing_at?`; unique live path per vault | v0.2 |
| `tasks` | core | `id`, `title`, `description`, `status`, `triaged_at?`, `planned_date?`, `due_date?`, `linked_note_id?`, `linked_repository_id?`, `created_at`, `updated_at`, `completed_at?`, `version` | v0.2 |
| `task_search` | core | External-content FTS5 over `tasks` (title, description), kept in step by triggers in the task's own transaction | v0.2 |
| `note_repository_links` | core | `note_id`, `repository_id`, `repository_name`, `remote_url?`, `created_at`; no foreign key to `repositories` | v0.2 |
| `note_bodies` | index | `note_id`, `content_hash`, `title`, `body`; the content table for `note_search` | v0.2 |
| `note_search` | index | External-content FTS5 over `note_bodies` | v0.2 |
| `note_links` | index | `source_note_id`, `target_note_id?`, `raw_target`, source location; unresolved links retained | v0.2 |
| `note_revisions` | history | `id`, `note_id`, `content`, `content_hash`, `created_at`, `reason` (`app_save`, `external_change`, `restore`); bounded local history | v0.2 |
| `note_sources` | core | `note_id`, source type, canonical URL/provider ID, source time, import time, content scope, source hash; derived from note provenance | v0.3 |
| `import_jobs` | core | `id`, input reference, adapter, state, staged payload reference, error, result note ID, idempotency key | v0.3 |
| `embedding_profiles` | core | `id`, provider, model name and digest, dimensions, distance metric, normalization, chunker version, state | v0.4 |
| `note_chunks` | index | `id`, `note_id`, profile, source hash, heading, byte/line range, text, chunk hash | v0.4 |
| vector tables | index | One `vec0` table per embedding profile: chunk ID, embedding, filter columns | v0.4 |

Task status is `todo`, `in_progress`, `done`, or `cancelled`, held in a `task_statuses` lookup table like the other enumerations in `0001_init.sql`. Completing sets `completed_at`; reopening clears it. A task is *to sort* while `triaged_at` is empty. `planned_date` and `due_date` are local calendar dates (`YYYY-MM-DD`), not UTC timestamps, so Today does not shift with time zones or daylight saving; `created_at`, `updated_at`, and `completed_at` stay UTC like the rest of the schema. A task links to at most one note and one repository; work spanning several repositories links a note that covers them. Task descriptions stay short plain text; longer material belongs in a linked note. Markdown checkboxes remain note content from v0.2 and do not automatically create or synchronize task records.
Keep missing-note tombstones to preserve task context. Pinning a note adds `note` to `pin_entity_types`.

#### Commands of later releases

| Command | Important input/output |
| --- | --- |
| `select_vault` | Native folder selection; returns vault and scan state |
| `list_notes` / `read_note` | IDs and pagination; content plus hash version |
| `create_note` / `rename_note` / `trash_note` / `restore_note` | Vault-relative destination, collision handling, expected version |
| `save_note` | Note ID, expected hash, Markdown; new version or conflict |
| `list_tasks` / `create_task` / `update_task` / `delete_task` | Filter or mutation DTO; expected integer version on existing records |
| `search` | Query, entity filters, pagination; ranked escaped excerpts |
| `prepare_import` / `commit_import` / `list_import_jobs` | Source input or job ID; preview/provenance, duplicate choice, resulting note, retry state |
| `rebuild_search` / `export_backup` / `restore_backup` | Job ID, progress and explicit completion |
| `configure_shortcut` | v0.3: validated binding and registration result |
| `semantic_search` / `ask_brain` / `cancel_job` | v0.4+: profile/request IDs, results or response channel |

### Safe note editing and indexing — v0.2

#### Save contract

`read_note` returns Markdown and a version based on its SHA-256 hash. `save_note` submits the expected version and new Markdown.

1. Persist a recoverable draft checkpoint and serialize app-originated writes to this note.
2. Read the current file and compare its hash with the expected version.
3. If divergent, return `CONFLICT`; keep the editor draft and the disk version available.
4. Preserve the pre-save file in local revision history. If that fails, stop the save and retain the draft.
5. Write a temporary sibling file, flush it, recheck the expected disk version, and atomically replace the destination. Preserve file permissions and sync the directory where supported.
6. Update note metadata in `brainiac.db`, then its body, search row, and links in `index.db`. The two are separate transactions; the index row carries the content hash it was built from, so an interrupted update is found and redone.
7. Return the new version and emit a committed change event. If indexing fails after the file write, report **Saved; search update pending** and enqueue repair.

The filesystem and SQLite do not share a transaction. Recovery must reconcile a completed file write with an interrupted database update. Hash checks detect observed conflicts; they cannot provide a filesystem compare-and-swap against arbitrary external editors. Revision history and draft recovery reduce that residual race risk.

Auto-save after a proposed 750 ms of inactivity. `Cmd+S` flushes immediately. Display Saving, Saved, Save failed, or Conflict accurately; switching views retains an unflushed draft.

#### External modifications

- Watch directories so atomic replacement saves are observed. Debounce note events for roughly 300 ms, then read and hash.
- A hash equal to the accepted version is a no-op, including notifications from the app's own writes.
- Before re-indexing a file changed outside Brainiac, store the previously indexed text from `index.db` as a revision with reason `external_change`, so edits from another editor or an agent can be undone like the app's own.
- For a clean active editor, reload the changed file and refresh the base version.
- For a dirty active editor, stop auto-save and offer **Reload disk**, **Save draft as copy**, or **Compare versions**. Preserve the draft before discarding it.
- If a file disappears while open, keep the draft and offer restore as a new file or relink.
- Startup, wake, watcher errors, and manual refresh trigger reconciliation. Notifications accelerate indexing; they are not the sole correctness mechanism.
- Watcher callbacks enqueue work instead of parsing or accessing SQLite directly. `notify` is the Rust watcher abstraction. [notify documentation](https://docs.rs/notify/latest/notify/)

#### Delete and recovery

App deletion moves the note into a recoverable vault-local trash location and removes it from active search. Keep its tombstone and associations. Restore resolves destination collisions explicitly. Never permanently delete the user's file as the default action.

External deletion marks a note missing and removes its search row. User-visible task context remains available as a missing note reference. Reconciliation must not treat an inaccessible vault as mass deletion.

#### Backup and restore

- Create a consistent snapshot of `brainiac.db` before each migration and once per active day; retain seven daily snapshots by default. Snapshot `history.db` on its own, less often. Never back up `index.db`.
- Snapshots use SQLite's backup API on the app's own connection, copying everything in one pass, so the copy is consistent while the app keeps running. `VACUUM INTO` also produces a consistent, compacted copy at more CPU cost and suits exports. Write either to a temporary name and rename it when complete, so a crash never leaves a partial file that looks like a snapshot. [SQLite backup API](https://www.sqlite.org/backup.html), [VACUUM INTO](https://www.sqlite.org/lang_vacuum.html)
- Every database file carries Brainiac's `PRAGMA application_id`; restore and open reject a file without it.
- Keep draft checkpoints and a bounded revision history: proposed 30 days, at most 20 revisions per note, and a global 250 MiB budget, excluding unresolved conflicts and active drafts.
- Provide an export containing vault files, a consistent database snapshot, and a versioned manifest. The vault and the database cannot share a transaction, so the manifest is what lets them be reconciled: schema version, vault ID, and for each note its ID, relative path, and content hash; for each linked repository its name and remote URL. Serialize application writes during snapshot/export and detect externally changed files; retry or report an incomplete export rather than claiming an atomic snapshot across independent editors.
- Export tasks as JSON with IDs, dates, statuses, and associations for portability.
- Restore validates the application ID, manifest, and schema before replacement. It then matches notes by `brainiac_id`, then by relative path and content hash; matches repositories by remote URL, offering Locate… for the rest; and rebuilds `index.db` from the vault.
- Local snapshots are recovery aids; a complete backup must include both vault files and application data, preferably on another device or backup system.

### Keyword search — v0.2

Index note titles, saved note body including code blocks, and task titles/descriptions. Search starts during vault ingestion and becomes complete when ingestion finishes. No embedding model is required.

Use external-content FTS5 tables, which index text stored once in a table Brainiac owns: `note_search` over `note_bodies` in `index.db`, and `task_search` over `tasks` in `brainiac.db`, each kept in step by triggers in the same transaction as the write. Their `integrity-check` command detects drift and `rebuild` regenerates the index from the content table, which a contentless table cannot do; snippets and highlights need the stored text either way. A search queries both tables and merges the results. FTS5 provides ranking, phrase, and prefix queries. [SQLite FTS5 documentation](https://www.sqlite.org/fts5.html)

Search behavior:

- Default input is literal user text; compile it into safe FTS expressions rather than passing arbitrary query syntax through.
- Support quoted phrases and final-token prefix matching; handle malformed quotes without exposing SQL errors.
- Boost title matches and include a title/path substring fallback for developer identifiers that tokenize poorly.
- Filter by Note or Task. Return a bounded first page of 50 results with stable pagination.
- Cancel or ignore obsolete requests when the query changes.
- Render snippets as escaped text plus highlight ranges, never trusted HTML.
- Clearly distinguish No matches, Indexing incomplete, and Search unavailable.
- Update task FTS rows in the task write transaction. Note indexing follows the successful file write and can be repaired.

### Agent access — v0.2.x

Agents such as Claude Code can already read and edit the vault's Markdown files, and the vault watcher treats them like any other editor. Tasks, links to repositories, and pins live only in `brainiac.db`, whose schema is internal; writing it directly would skip version checks, rules such as setting `completed_at`, and change events, so the UI would not update. Agents use a supported interface instead.

- The running app hosts a local MCP server. A small `brainiac-mcp` helper, which an agent starts over stdio, forwards requests to the app through a Unix socket in the app's data folder, as editor CLIs reach a running editor. If the app is not running, the helper reports that.
- Tools mirror the commands of later releases: `search`, `read_note`, `list_tasks`, `create_task`, `update_task`, `link_repository`, and `repository_status`. Each calls the same domain service as the matching Tauri command, so validation, version conflicts, and committed change events are identical and the UI updates live.
- Off by default; Settings chooses read-only or read-write. Local only. No tool writes to a Git repository, runs commands, or changes settings.
- Note edits made through the vault rather than the server get a revision with reason `external_change`, so they can be undone.

v0.2 prepares for this without the server: every write goes through a domain service that emits its change event (not through a Tauri handler), and every task write carries the expected version.

### Capture and imports — v0.3

Import is a core way to create knowledge, available from the same capture interface as writing a note. Sources feed a common Rust ingestion pipeline and become ordinary Markdown notes in the Inbox. Importers do not create separate knowledge silos or require AI to produce a useful result.

#### Capture interface

Provide an **Add to brain** action in the main window and command palette. Accept pasted text, a URL, or a selected/dropped supported file. Show the detected source and allow the user to correct the title, add a personal comment, and choose a destination. Default to `Inbox/`.

After saving, the item is immediately readable and searchable using the content actually captured. Show a small source badge, capture date, and **Open original** action. The inspector exposes provenance without making the note editor feel like an import-management tool. Inbox triage offers file/move, link to a task or repository, and create a task explicitly.

Later, the same capture action can be reached from global quick capture, a browser extension, or a macOS share extension. Those entry points submit to the same backend pipeline. Native share-extension packaging and browser handoff need their own technical spike; they are not assumed to come automatically with Tauri.

#### Source behaviors

| Source | Captured note | Initial path | Richer integration |
| --- | --- | --- | --- |
| Pasted text/selection | Supplied text, optional source URL, and personal comment | v0.3 paste | Browser extension supplies selected text and page metadata |
| Markdown file | Independent copy in the vault, preserving original source reference | v0.3 picker/drop | Batch import with preview |
| Web page | Article title, source URL, author/date when available, extracted readable body | v0.3 URL bookmark; v0.3 extraction | Browser capture for authenticated or client-rendered pages |
| Email | Subject, sender/recipients/date, selected message body, message identity | Paste in v0.3; `.eml` in v0.3 | Selected-message import through an authorized mailbox connector |
| Jira issue | Issue key/title, description, status/assignee snapshot, source link | Paste text or bookmark in v0.3 | Explicit fetch through configured Jira deployment/account |
| YouTube video | Video link/title and user's notes; timestamped transcript when supplied/available | Bookmark and pasted transcript in v0.3 | Metadata and supported transcript acquisition |

Saving a URL is distinct from capturing its contents. Every import has a `content_scope`: `full_text`, `excerpt`, `transcript`, or `metadata_only`. A bookmark-only note must not appear to have the full article or video available for search or RAG.

For email, “send to my brain” initially means supplying message content or an exported `.eml`. A dedicated forwarding address would require an optional mail-receiving service or reading a configured mailbox; an offline desktop application cannot independently receive public email while closed. Connected mailbox import uses provider authorization, such as the Gmail API. [Gmail API overview](https://developers.google.com/workspace/gmail/api/guides)

For Jira, start with importing individual selected issues. Distinguish Cloud versus Data Center during connector design; keep provider-specific authentication and document formats inside the adapter. The issue becomes a captured snapshot, optionally linked to a task. Importing it does not establish two-way task/status synchronization.

For YouTube, do not promise automatic transcripts for every public video. The official caption-download API requires permission to edit the video. Accept pasted/uploaded transcripts, or a separately verified acquisition method; otherwise save the reference and label transcript unavailable. Preserve transcript timestamps so citations can open the relevant video position. [YouTube caption-download API](https://developers.google.com/youtube/v3/docs/captions/download)

#### Common ingestion pipeline

```text
Paste / file / URL / provider selection
  → identify source and persist import job
  → acquire content, if needed
  → extract and normalize to Markdown
  → preview content and provenance
  → check duplicates and choose disposition
  → save Inbox note through the normal safe-write pipeline
  → index captured content
  → optional enrichment in later releases
```

Implement `imports.rs` with a small source-kind enum and separate adapter functions initially. Each adapter returns a common `ImportedContent` DTO: suggested title, Markdown, source identity/URL, source timestamps, content scope, content hash, warnings, and optional original-file references. Parsing/network acquisition does not write a note itself; the import service commits through `NoteService`.

Job states are `queued`, `acquiring`, `extracting`, `awaiting_review`, `saving`, `complete`, `failed`, and `cancelled`. Simple pasted captures can skip a separate preview screen because the capture form is already editable. Network/file extraction shows a preview before committing. Retry resumes safely with the same job ID; cancelled/failed jobs never produce apparently complete notes. Completion means the note exists; search may independently show an indexing-pending state.

#### Provenance and user edits

- Embed a `brainiac_source` frontmatter object containing source type, stable source ID/URL, imported time, source modified time when known, content scope, and source-content hash. Keep provider-specific metadata in a namespaced object.
- Put personal commentary in a clearly separate section from captured source text. Summaries, when added later, are labeled generated content and never replace the source body.
- Keep source material useful offline after import. Original web content may later change or disappear; Open original is navigation, not the storage strategy.
- Retain raw originals such as `.eml` in a vault-local `attachments/imports/<import-id>/` folder in v0.3. Save relative references in frontmatter and include them in export/restore. Unsupported attachments stay as originals, with no implied indexing.
- Imported Markdown copies get a new Brainiac ID; preserve any original ID as source metadata so copying does not create identity collisions. Do not modify the source file. Detect dependencies such as relative images and warn when they cannot be copied/resolved by the supported importer.
- Source caches are reconstructible from provenance. Pending acquisition credentials and job state stay outside note files.

#### Duplicates and updates

Deduplicate by provider account plus stable item ID where available: Jira issue ID, email Message-ID/provider message ID, video ID, or conservatively normalized URL. Use content hashes as a second signal, not the sole identity for unrelated items.

For a repeated unchanged capture, offer **Open existing** or **Save another copy**. If the source changed, offer an explicit new snapshot or a previewed update of the imported section. Never silently overwrite a user's annotations or independently edited note. URL normalization must not strip query parameters that identify different content.

Snapshot import is the default. Live synchronization, scheduled bulk acquisition, and remote mutations are separate later features with their own contracts.

#### Extraction limits and search behavior

- Apply bounded download sizes, redirect counts, timeouts, and cancellation. Use source-specific parsers and strip active HTML, scripts, and tracking pixels.
- Generic URL import fetches public `http`/`https` pages; it does not borrow browser cookies, bypass login walls, or silently execute page JavaScript. If extraction fails, offer bookmark-only or paste selection.
- Authenticated/provider imports use explicit configured accounts, with credentials in Keychain. Configure company Jira hosts deliberately rather than forwarding credentials to arbitrary pasted hosts or redirects.
- Keep external page resources from triggering incidental downloads. Fetch attachments only through an explicit supported import action.
- Index the resulting Markdown using the existing FTS/chunking pipeline. Add source-type filters and display original-source provenance on results.
- RAG uses only captured bodies/transcripts and distinguishes personal comments from source excerpts. It cannot answer from the uncaptured contents of a saved URL.
- Import verification covers duplicate retries, source identity collisions, partial failures, MIME/HTML conversion, unavailable transcripts, user edits during refresh, and backup/restore of original files.

#### Import rollout

1. **v0.3 core:** text/selection capture, URL bookmarks, Markdown copies, provenance, Inbox triage, duplicate/retry handling.
2. **v0.3 source adapters:** public-page extraction and `.eml` parsing/original retention, with explicit bookmark/paste fallback.
3. **v0.3.x:** one authenticated connector, selected by daily use—Jira or the email provider—and validated video metadata/transcript improvements.
4. **Later:** browser/share entry points, optional forwarding service, batches, and explicit synchronization.

All external content-import behavior starts in v0.3. Selecting local repositories and discovering them inside a folder in v0.1 is workspace configuration, separate from importing knowledge content.

**Acceptance scenario:** capture an email, an issue excerpt, an article, and a video reference into the Inbox; add commentary, create a task from one item, find captured material offline, open each original source, retry an interrupted import without duplicates, and identify which items contain only metadata.

### Global capture and window state — v0.3

Use the official `tauri-plugin-global-shortcut` from Rust. Default to `Cmd+Shift+Space`, allow rebinding, and show shortcut registration conflicts. Handle pressed events once; key release must not trigger a second capture. [Tauri global shortcut plugin](https://v2.tauri.app/plugin/global-shortcut/)

The capture window is a small undecorated window with text input, Note/Task selection, and optional search. Enter saves, Escape dismisses, and dismissal preserves an unsaved draft. Keep it pre-created and hidden if measurement supports faster activation.

Prototype activation over other apps, multiple displays, Spaces, and fullscreen windows. Test focus return on dismissal. Prefer supported Tauri window APIs and an opaque fallback before adding macOS-specific code. Translucency is visual polish, not a dependency for capture. [Tauri window customization](https://v2.tauri.app/learn/window-customization/)

Do not assume Accessibility or Full Disk Access is required for global shortcuts. Determine actual permissions from the implemented mechanism on supported macOS versions; ask for an OS permission only when that feature needs it.

Rust is the shared authority across windows:

1. Each WebView submits a command.
2. Rust validates, commits, and returns canonical state.
3. Rust emits a versioned change event to authorized windows.
4. Each window refreshes affected state; newly opened or resumed windows fetch a snapshot.

Events invalidate caches; they are not a durable event log. Include entity ID, version, and origin window. Frontends discard older events and unsubscribe on teardown. Dirty editors use the conflict contract even when a change originated in another app window.

### Semantic search — v0.4

Keep lexical search fully functional when semantic search is disabled, indexing is incomplete, or Ollama is unavailable.

#### Embedding ingestion

1. Parse Markdown into heading-aware chunks while preserving code blocks.
2. Start with a target around 400 tokens, bounded overlap, and a hard cap below the selected model's context limit. Long sections require splitting; headings alone are insufficient.
3. Store source hash, heading context, line/byte ranges, text, and chunker version.
4. Hash the actual embedding input, including added heading context. Reuse unchanged vectors only within the same model/profile and chunker configuration.
5. Request embeddings from a configured local Ollama model through `/api/embed`. Validate returned dimensions and avoid silent input truncation. [Ollama embedding API](https://docs.ollama.com/api/embed)
6. Publish the replacement chunk set only if the note's source hash still matches the job's input. Discard superseded jobs.
7. Remove superseded/deleted chunks from retrieval promptly; periodically clean orphaned storage.

The embedding model, its dimensions, language coverage, and context limit are selected during a representative-corpus spike. Do not hard-code 384 dimensions or assume the model examples from the source discussion are interchangeable.

#### Vector integration and hybrid retrieval

Use the `sqlite-vec` Rust binding with the same bundled SQLite used by `rusqlite`. Register the extension through a vetted initialization path, encapsulate required unsafe FFI, and verify release packaging before adopting it. Never load arbitrary extensions supplied by a note or workspace. [sqlite-vec Rust binding](https://docs.rs/sqlite-vec/latest/sqlite_vec/)

- Keep chunk metadata in ordinary tables and vector data in compatible profile-specific tables, all in `index.db`.
- Schema dimensions and the query embedding must match the active profile. Model changes create a new index generation; do not mix vectors from different models.
- Changing the model adds a profile and its own `vec0` table. A resumable background job fills it while search keeps using the active profile; new and changed notes are embedded for both, and a backfill never overwrites a newer vector. Switch the active profile when the job finishes, then drop the old table, which also reclaims space `vec0` does not reclaim after deletes. [Qdrant: embedding model migration](https://qdrant.tech/documentation/tutorials-operations/embedding-model-migration/)
- Stable `sqlite-vec` releases search by brute force only. Its author measured about 0.2 s for 100,000 vectors of 3,072 dimensions on an M1, so a personal vault of thousands to tens of thousands of chunks needs no approximate index. [sqlite-vec stable release](https://alexgarcia.xyz/blog/2024/sqlite-vec-stable-release/index.html)
- Filter inside the vector query with `vec0` metadata columns (at most 16; equality, comparison, and `IN` only), for example by source kind; apply richer filters in SQL on an over-fetched result. [vec0 metadata columns](https://alexgarcia.xyz/blog/2024/sqlite-vec-metadata-release/index.html)
- Use the extension's supported nearest-neighbor query syntax, including its `k` constraint where required. [sqlite-vec KNN documentation](https://alexgarcia.xyz/sqlite-vec/features/knn.html)
- Combine keyword and semantic candidate ranks with reciprocal rank fusion (k = 60) in one SQL query: rank each list with `row_number()`, join them with `FULL OUTER JOIN`, and sum `1/(k + rank)` per list, so a match found by both outranks one found by either alone. This suits a vault full of exact project and repository names. Deduplicate overlapping chunks and cap results per note. [Hybrid search with sqlite-vec](https://alexgarcia.xyz/blog/2024/sqlite-vec-hybrid-search/index.html)
- Return source excerpts and distinguish keyword from semantic matches.
- Start with notes only. Task, commit-message, and source-code embeddings are separate future scope.
- Model installation is an explicit user action. Show download/storage requirements, indexing progress, pause, cancellation, and retry.

### Ask my brain — v0.5

RAG uses hybrid retrieval to answer questions about saved notes. It adds synthesis after retrieval is useful and measured.

- Retrieve relevant chunks, verify their source hashes, and fit them into the generation model's context budget.
- Include source labels, note identity, heading, and location in the prompt.
- Stream output through a Tauri IPC channel, with request ID, cancellation, and terminal success/error state. Tauri recommends channels for streamed data. [Tauri Rust commands and channels](https://v2.tauri.app/develop/calling-rust/)
- Show citations that open the supporting note at its source location. If the file changed since generation, label the citation stale.
- If supporting material is missing or weak, say that the notes do not establish an answer; offer relevant excerpts.
- Treat retrieved text as reference material, not instructions. This feature has no filesystem-write or shell tools.
- Do not silently turn generated suggestions into tasks or modify notes.
- Keep conversations session-local initially; Save as note is explicit.
- AI requests go only to the configured local endpoint by default. A cloud provider would require a later explicit product decision and opt-in.

### Evolving the data — all releases

- Migrations stay append-only and run at startup, after a pre-migration snapshot. That is enough for one user on one machine.
- An app must never run on a database newer than it knows: `PRAGMA user_version` is a convention SQLite does not enforce, so the app checks it and refuses, pointing to the pre-migration snapshot. This ships in v0.1.x, before the first schema change. [SQLite file format](https://www.sqlite.org/fileformat.html)
- Rows use UUIDs, notes are identified by vault ID plus relative path, and unknown frontmatter keys are preserved, so multiple vaults or sync can be added later without rewriting identity. Absolute paths stay only where they are local by nature, such as a repository's folder; its remote URL is the identity that travels.
- Sync, if it comes, cannot migrate every device at once, because devices run different app versions. Record a format version on synced records and translate on read rather than migrating all data in one step. [Ink & Switch: Cambria](https://www.inkandswitch.com/cambria/)
- New AI models or chunkers add an embedding profile; they never alter existing vectors in place.
