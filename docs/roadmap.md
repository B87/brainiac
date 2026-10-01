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

- [ ] Add/open one repository directly; persist recent/pinned repositories and relocate missing paths.
- [ ] Create a workspace manually or by discovering independent repositories inside a chosen folder, with optional root grouping and rescan.
- [ ] Let the user choose which discovered repositories to track and manage membership independently of their IDE.
- [ ] Aggregate status table, unique changed-file counts, dirty/conflicted/stale filters, partial-error handling.
- [ ] Changes list with staged/unstaged diffs, untracked previews, conflict and binary/large-file states.
- [x] Paginated commit history, message/hash filtering, metadata, changed files, and per-file commit patches.
- [x] Read-only branch/tag lists and ref-scoped history selection.
- [ ] Debounced tracking, bounded subprocesses, manual refresh, wake/activation reconciliation.
- [ ] Repository/workspace command palette, copy hashes/paths, Open in editor, Reveal in Finder.
- [ ] Fetch now and opt-in per-workspace auto-fetch of watched branches, with lock detection, backoff, and non-interactive credentials.
- [ ] Workspace Activity tab: watched refs, advanced/rewritten/created/tagged events, unread state, conflict-risk and drift warnings, team pulse, fetch freshness, optional notifications and morning digest.
- [ ] Viewer refinements: line counts in Changes, Both comparison, ignore whitespace, split layout, word highlights, hunk navigation, author filter, comparison with the default branch, keyboard shortcuts, AA contrast.
- [ ] SQLite snapshot backup of settings/workspaces (export manifest is v0.2); no note vault, account, FTS index, or model required.

**Exit gate:** use the app for a week with a standalone repository, a manual workspace, and a discovered workspace whose folder is itself a repository. Identify dirty/conflicted repos, inspect staged and unstaged patches, navigate history and refs, observe external changes, see a teammate's merge in the Activity tab after a fetch, retain registrations after restart, and recover from one slow/missing repo while the rest remain usable. Confirm viewing does not alter working files, refs, or the index, and that fetching changes only remote-tracking refs and tags.

**Topology acceptance:** select a folder once and discover the repositories directly inside its discovery folder, both when the discovery folder is the selected folder and when it is a subfolder, and both when the selected folder is a repository and when it is not. Root and member changes remain separately attributed even when the root ignores the discovery folder. Adding/removing a child is detected, plain child folders do not become false repositories, and repeated discovery/manual selection does not duplicate registrations. Include independent nested repos, a linked worktree, and an actual submodule in fixtures.

**Setup acceptance:** configure the root plus the selected project repositories through folder discovery/selection, persist that selection across restart, and inspect all of them without reading or depending on the IDE workspace file.

### v0.2 — Knowledge, tasks, and code context

- [ ] Vault onboarding, scan/reconciliation, folder navigation, pinned/recent notes.
- [ ] Editor fidelity spike; note CRUD, auto-save, source fallback, conflicts, trash, draft/history recovery.
- [ ] Tasks with status, triage, planned dates, deadlines, linked note and linked repository.
- [ ] Inbox, Today, Tasks, Brain alongside the existing Workspaces/Git views.
- [ ] FTS5 note/task ingestion, ranked keyword search, safe snippets, rebuild status.
- [ ] Note/repository associations, standard Markdown links/backlinks, unresolved targets.
- [ ] Complete vault/database export and restore preserving tasks and associations.

**Exit gate:** use Brainiac for real notes and daily tasks alongside Git tracking; edits survive restart, knowledge search works offline, Today behaves correctly across midnight, and restore recovers files, tasks, and context links.

### v0.3 — Content imports and global capture

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
| Embedding model and dimensions | Local retrieval evaluation, model availability and license review | v0.4 schema/profile selection |
| SQLite-vector integration compatibility | Build and query a bundled release executable | v0.4 |
| Remote integrations and company data constraints | Choose the first provider and its allowed scopes | Integration release |
| Preferred mail source and capture gesture | Choose `.eml`, connected mailbox, or optional forwarding workflow | Email connector implementation |
| Jira deployment/authentication | Confirm Cloud versus Data Center and company-supported access | Jira connector implementation |
| Article extraction and transcript availability | Evaluate representative pages/videos; document fallback coverage | Rich import compatibility claims |
| Sync expectations and encryption | Separate design; define conflicts and authoritative stores | Any sync implementation |

These unknowns do not prevent implementing the core domain and persistence services. The first concrete development step is M0, followed by the smallest complete open repository → inspect status → view diff/history → track changes workflow.

## Designs for later releases

### v0.2 additions to views, storage, and contracts

#### Knowledge views — v0.2

Add Brain (vault tree/editor), Tasks, Inbox, and Today alongside the existing Workspaces view. Tasks link to notes and repositories. Inbox holds untriaged tasks and locally authored capture notes; source-import notes arrive in v0.3.

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
- Existing notes with no such key receive a persisted application ID without modifying the file. Adding an embedded ID is an explicit action.
- Title comes from frontmatter `title`, then the first H1, then the filename.
- App-driven renames preserve identity. External moves use embedded IDs where available; otherwise match a unique content hash within the reconciliation batch.
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

#### Data model of later releases

| Entity | Essential fields and constraints | Release |
| --- | --- | --- |
| `vaults` | `id`, `name`, `root_path`; one active vault initially | v0.2 |
| `notes` | `id`, `vault_id`, `relative_path`, `embedded_id?`, `title`, `content_hash`, `mtime`, `indexed_at`, `missing_at?`; unique live path per vault | v0.2 |
| `tasks` | `id`, `title`, `description`, `status`, `triaged_at?`, `planned_date?`, `due_date?`, `linked_note_id?`, `created_at`, `updated_at`, `completed_at?`, `version` | v0.2 |
| `note_revisions` | `id`, `note_id`, `content`, `content_hash`, `created_at`, `reason`; bounded local history | v0.2 |
| `search_documents` | FTS5 rows: entity type/ID plus title and searchable body | v0.2 |
| `note_sources` | `note_id`, source type, canonical URL/provider ID, source time, import time, content scope, source hash; derived from note provenance | v0.3 |
| `import_jobs` | `id`, input reference, adapter, state, staged payload reference, error, result note ID, idempotency key | v0.3 |
| `note_links` | `source_note_id`, `target_note_id?`, `raw_target`, source location; unresolved links retained | v0.2 |
| `note_repository_links` | `note_id`, `repository_id` | v0.2 |
| `embedding_profiles` | `id`, provider/model identity, dimensions, distance metric, chunker version | v0.4 |
| `note_chunks` | `id`, `note_id`, source hash, heading, byte/line range, text, chunk hash | v0.4 |
| vector tables | Chunk ID and embedding, partitioned by compatible embedding profile | v0.4 |

Task status is `todo`, `in_progress`, `done`, or `cancelled`, enforced by a database constraint. Completing sets `completed_at`; reopening clears it. Markdown checkboxes remain note content from v0.2 and do not automatically create or synchronize task records.
Keep missing-note tombstones to preserve task context. Add task `linked_repository_id` in the v0.2 migration.

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
6. Update note metadata and its FTS row in one database transaction.
7. Return the new version and emit a committed change event. If indexing fails after the file write, report **Saved; search update pending** and enqueue repair.

The filesystem and SQLite do not share a transaction. Recovery must reconcile a completed file write with an interrupted database update. Hash checks detect observed conflicts; they cannot provide a filesystem compare-and-swap against arbitrary external editors. Revision history and draft recovery reduce that residual race risk.

Auto-save after a proposed 750 ms of inactivity. `Cmd+S` flushes immediately. Display Saving, Saved, Save failed, or Conflict accurately; switching views retains an unflushed draft.

#### External modifications

- Watch directories so atomic replacement saves are observed. Debounce note events for roughly 300 ms, then read and hash.
- A hash equal to the accepted version is a no-op, including notifications from the app's own writes.
- For a clean active editor, reload the changed file and refresh the base version.
- For a dirty active editor, stop auto-save and offer **Reload disk**, **Save draft as copy**, or **Compare versions**. Preserve the draft before discarding it.
- If a file disappears while open, keep the draft and offer restore as a new file or relink.
- Startup, wake, watcher errors, and manual refresh trigger reconciliation. Notifications accelerate indexing; they are not the sole correctness mechanism.
- Watcher callbacks enqueue work instead of parsing or accessing SQLite directly. `notify` is the Rust watcher abstraction. [notify documentation](https://docs.rs/notify/latest/notify/)

#### Delete and recovery

App deletion moves the note into a recoverable vault-local trash location and removes it from active search. Keep its tombstone and associations. Restore resolves destination collisions explicitly. Never permanently delete the user's file as the default action.

External deletion marks a note missing and removes its search row. User-visible task context remains available as a missing note reference. Reconciliation must not treat an inaccessible vault as mass deletion.

#### Backup and restore

- Create a consistent SQLite snapshot before each migration and once per active day; retain seven daily snapshots by default.
- Keep draft checkpoints and a bounded revision history: proposed 30 days, at most 20 revisions per note, and a global 250 MiB budget, excluding unresolved conflicts and active drafts.
- Provide an export containing vault files, a consistent database snapshot, and a versioned manifest. Serialize application writes during snapshot/export and detect externally changed files; retry or report an incomplete export rather than claiming an atomic snapshot across independent editors.
- Export tasks as JSON with IDs, dates, statuses, and associations for portability.
- Restore validates the manifest and schema before replacement, then rescans files and rebuilds derived indexes.
- Local snapshots are recovery aids; a complete backup must include both vault files and application data, preferably on another device or backup system.

### Keyword search — v0.2

Index note titles, saved note body including code blocks, and task titles/descriptions. Search starts during vault ingestion and becomes complete when ingestion finishes. No embedding model is required.

Use a normal, content-bearing FTS5 table with entity IDs stored as unindexed columns. Maintain one row per live note/task, replacing it transactionally on updates. Keep the cache rebuildable independently of authoritative task records. FTS5 provides ranking, phrase, and prefix queries. [SQLite FTS5 documentation](https://www.sqlite.org/fts5.html)

Search behavior:

- Default input is literal user text; compile it into safe FTS expressions rather than passing arbitrary query syntax through.
- Support quoted phrases and final-token prefix matching; handle malformed quotes without exposing SQL errors.
- Boost title matches and include a title/path substring fallback for developer identifiers that tokenize poorly.
- Filter by Note or Task. Return a bounded first page of 50 results with stable pagination.
- Cancel or ignore obsolete requests when the query changes.
- Render snippets as escaped text plus highlight ranges, never trusted HTML.
- Clearly distinguish No matches, Indexing incomplete, and Search unavailable.
- Update task FTS rows in the task write transaction. Note indexing follows the successful file write and can be repaired.

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

- Keep chunk metadata in ordinary tables and vector data in compatible profile-specific tables.
- Schema dimensions and the query embedding must match the active profile. Model changes create a new index generation; do not mix vectors from different models.
- Use the extension's supported nearest-neighbor query syntax, including its `k` constraint where required. [sqlite-vec KNN documentation](https://alexgarcia.xyz/sqlite-vec/features/knn.html)
- Combine keyword and semantic candidate ranks with reciprocal rank fusion; deduplicate overlapping chunks and cap results per note.
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
