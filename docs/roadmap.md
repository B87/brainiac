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
- [x] Relocate a repository whose folder moved, keeping its identity; Rescan suggests renamed members.
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

- [x] Vault onboarding, scan/reconciliation, folder navigation, pinned/recent notes.
- [x] Editor fidelity spike (S1): TipTap's Markdown round trip rewrote or lost content, so notes are edited as styled Markdown text in CodeMirror 6.
- [x] Live Preview spike (S1b): passed in WebKit; the editor is `src/lib/editor/` and the `NoteEditor` component, ready to wire into the Notes view.
- [x] Note editor with Live Preview and Source mode; note CRUD, auto-save, conflicts, trash, draft/history recovery.
- [x] Tasks with status, planned dates, deadlines, linked note and linked repository; tasks not yet sorted are marked as such.
- [x] Today, Tasks, Notes alongside the existing Workspaces/Git views; tasks to sort appear as a folded line in Today and a filter in Tasks.
- [x] Search tokenizer spike (S2): `unicode61` for note and task text, `trigram` only for titles and paths; every query word matches as a prefix.
- [x] Vault scale spike (S3): 10,000 generated notes index in about 4 s and rescan in 35 ms; snippets are cut in Rust and one worker writes the index.
- [x] Vault watcher spike (S4): one recursive watcher saw every editor save, rename, and `git pull` in a 10,000-note vault; changes are processed in batches so renames survive a pull.
- [x] FTS5 note/task ingestion, ranked keyword search, safe snippets, rebuild status.
- [x] Note/repository associations, Markdown links and wikilinks, backlinks, unresolved targets.
- [x] Storage split into `brainiac.db`, `index.db`, and `history.db`; migration `0002` tested against a 0.1.3 database (built by the test from `0001_init.sql` with v0.1 data, so no binary fixture is checked in).
- [x] Complete vault/database export and restore preserving tasks and associations.
- [x] One write path for every caller: each write goes through a domain service that emits the committed change event; task writes carry the expected version; a note changed outside Brainiac keeps its previous text as a revision.

**Exit gate:** use Brainiac for real notes and daily tasks alongside Git tracking; edits survive restart, knowledge search works offline, Today behaves correctly across midnight, and restore recovers files, tasks, and context links.

### v0.2.x — Agent access

- [x] Spike S5: `rmcp` over a Unix socket, reached by Claude Code through a stdio helper (`docs/architecture.md`, Decisions).
- [x] Local MCP server inside the running app, reached through `brainiac mcp`, the app's own executable as a stdio helper that opens the app when it is closed; off by default, read only or read and write.
- [x] Tools for search, notes, tasks, and repository links that call the same domain services as the UI; agent saves kept as `agent` revisions.
- [x] Settings → Agent access, with the connected agents and the command to add Brainiac to Claude Code.
- [x] Claude Code plugin in this repository: the server and a skill for using Brainiac.
- [ ] Exit gate checked by hand with Claude Code against a packaged build.

**Exit gate:** an agent such as Claude Code finds notes, creates and completes a task, and links a note to a repository while the app is open; the UI updates live, a concurrent UI edit produces a conflict rather than an overwrite, and the agent's note edits can be undone from revision history.

### v0.3 — Pull requests for GitHub and Bitbucket Cloud

The Git viewer and workspace tracking proved the most useful part of the app, so pull requests come before imports, capture, and AI. Design: `SPEC.md` section 10 and `architecture.md`, Pull requests — v0.3; started 3 October 2026 on the `v0.3-pull-requests` branch.

- [x] Spike S6: Bitbucket Cloud's draft pull requests, pending comments, multi-line inline anchors, merge behavior (conditional on a commit, asynchronous completion), and conditional requests, against a throwaway repository on each provider; measure the request cost of a workspace of about 30 repositories (`docs/architecture.md`, Decisions).
- [x] Accounts: a GitHub token and a Bitbucket Cloud API token, kept in the Keychain, checked with one request, with their scopes and the hour's request use shown.
- [x] Forge mapping: each repository's pull requests come from its `origin`, with an override for an upstream or a fork; a per-workspace switch, off by default, turns tracking on and picks the account.
- [x] Provider-neutral `PullRequestService` over two adapters (GitHub REST plus GraphQL for review threads; Bitbucket Cloud REST 2.0), with a discardable cache, a per-account request budget, and `pr_changed` events. Reads (lists, one pull request, files, checks) are done; the conversation, diffs, and writes come with their views.
- [x] Read: the workspace's Pull requests tab with filters, each repository's Pull requests tab with the checked-out branch's pull request first, the pull request overview (description, conversation, checks, what it needs to merge), files changed with viewed marks and "since your review", and diffs from local Git when both commits are on the Mac.
- [x] Review: comment, reply, resolve, local review drafts, and submitting a review or an approval tied to the commit that was reviewed; a head that moved stops the submission and keeps the drafts.
- [x] Merge, confirmed, with only the methods the repository allows, refused when the head moved.
- [x] A count of reviews waiting on you on each workspace in the sidebar.

**Exit gate:** for a week, review real pull requests on both providers from Brainiac: find what waits on you from the workspace tab, review only what changed since your last review, comment and approve without opening the browser, and merge one pull request. A push during a review stops the submission without losing drafts; a Bitbucket submission cut off midway resumes without duplicate comments; losing the network or exhausting the request budget leaves the Git views working; and nothing is written to a local repository.

### v0.4 — Databases

A database client is open most of the working day next to the repositories it belongs to, so it comes before imports and capture. Design: `SPEC.md` section 11 and `architecture.md`, Databases — v0.4, with the options compared in [`design/databases.md`](design/databases.md); started 4 October 2026 on the `v0.4-databases` branch.

- [x] Spike: decode every built-in PostgreSQL type with `tokio-postgres`, `numeric` included; a portal's row limit with `RETURNING`; the read-only wrapper against `SET TRANSACTION READ WRITE`, `CALL` with a `COMMIT`, and `COPY`; rustls against hosted providers' certificates; cancel latency; a corpus for the statement splitter.
- [x] Read-only client: SQLite and PostgreSQL connections with passwords in the Keychain, Home, the query view with one session per tab, the connection switcher and the side panel's schema, completion, Run and Run All, cancel, timeouts, the result grid with the inspector, Copy. Every connection is read only in this step.
- [x] Saved queries and history: names, folders, `:name` parameters, `Cmd+K`, search, history in the side panel, tabs that survive a restart, Copy as Markdown.
- [x] Writes: read-and-write access, Auto-commit and Manual transactions, the Production rules, Explain and Explain Analyze, Export, connections linked to repositories.
- [x] Health: the PostgreSQL metrics first, then Docker containers, then Cloud SQL through Cloud Monitoring; Coolify waits for SSH tunnels.

**Status, 5 October 2026:** released as 0.4.0, before its exit gate was checked by hand; problems found in that week of use become 0.4.x fixes. As of 4 October, every step above was committed on the `v0.4-databases` branch. The spike's checks are tests against a throwaway PostgreSQL server (`architecture.md`, Decisions); the services are tested the same way and against SQLite files the tests create; Docker and Cloud Monitoring against local stand-ins; the views in WebKit over the fake backend. Left: the exit gate by hand over a week of real use, which is also the first time Brainiac meets a hosted provider's TLS (RDS, Cloud SQL, Supabase, Neon), a real Docker socket, and real Cloud Monitoring credentials. Not built from the design: dropping a SQLite file on the window, copying a selection of rows (Copy takes every row, or one value), and exporting only the rows loaded.

**Exit gate:** for a week, use Brainiac instead of the usual database GUI against a local SQLite file, a local PostgreSQL in Docker, and a hosted production PostgreSQL. Run the weekly saved queries from `Cmd+K` with their parameters; cancel a slow query; copy results into a note; a write on a read-only connection fails and changes nothing; a write on production needs read and write turned on for that tab and leaves a transaction to commit or roll back by hand; Health shows connections, the slowest queries, and the machine's memory for the Docker and Cloud SQL databases; and a password never reaches the window, a log, or Brainiac's files.

**v0.4.x follow-ups, chosen by use:** SSH tunnels (which also bring Coolify's metrics), editing rows in the grid, saved queries as files, and secrets phase 2 ([`design/secrets.md`](design/secrets.md)): Google Secret Manager, Git's credential helper, and `.pgpass`.

**Secrets, phase 1 (4 October 2026):** committed on the `v0.4-databases` branch with the database work, and moved into [`SPEC.md`](../SPEC.md) section 12 and [`architecture.md`](architecture.md), Secrets — v0.4.x: the credentials layer with the Keychain, Ask, None, environment variables, and commands; the save, cleanup, and removal markers; approval of restored sources; and Settings → Secrets. v0.5 reads its agents' keys through this layer. Tested with an in-memory Keychain, scripts as commands, a throwaway PostgreSQL server, and WebKit over the fake backend. Left: its exit gate by hand (a week on an unsigned `pnpm tauri dev` build with `gh auth token` and a database password from a command), the spike of `op read` and `bw get` from the GUI app, and finding programs on an Intel Mac.

### v0.5 — Agent runs

A coding agent's command-line tool run in a container, on the Mac or a remote host, started from a repository or a task and reviewed in the diff viewer before anything is pushed. Design: [`design/agent-runs.md`](design/agent-runs.md). It needs Health's Docker client (v0.4) and the credentials layer (v0.4.x). Remote hosts use system SSH, not database tunnels; the Mac-sleep check below is still required before that item is done. Create pull request waits for v0.3.x. Remote sessions surviving Mac sleep must be designed and demonstrated in an early remote prototype before building the production runtime, even though local runs may ship first.

- [x] Blocking architecture spike: one pinned Claude ACP adapter over non-TTY stdio with no Mac filesystem/terminal, owned by an independently running controller on an approved remote host. Prove actual Mac sleep/wake, app quit/reopen, SSH loss, same-session follow-up, offline permissions/deadline/collection, journal replay, lost acknowledgements without duplicate turns, controller-crash termination, and authenticated emergency stop. Also prove arbitrary-commit export on Git 2.30, stopped-volume collection/cleanup, controller-memory/stdin credential delivery with raw logs disabled, enforced disk budgets, and local sleep/resume expiry reconciliation under its disclosed limitation.
- [x] Agent setup in Settings: a local engine/controller using the proven session protocol, Claude Code with an API-key source or the user's Claude subscription token from `claude setup-token` (the token only after Anthropic's answer on its terms, `design/agent-runs.md`, Subscription token), an image pinned by digest, explicit provider/host/network disclosure, and a test run.
- [x] Runs from a complete local repository: immutable input bundle, bounded filtered controller journal, same-session reconnect after app quit, permission choices, cancel/follow-up/Finish, interrupted-runtime recovery, snapshot of the final tree including uncommitted edits, verified import and review, Save patch, Delete run with retryable collection/cleanup. Local execution does not progress while its host is suspended.
- [ ] Landing and more agents: create-only HTTPS Push of the exact reviewed snapshot with uncertain-response reconciliation, Create pull request after its v0.3.x dependency, tested Codex/Gemini adapters, enforced network allowlist before adding read-only registry tokens.
- [ ] Remote hosts after production SSH integration: approved-host Settings, verified controller deployment/upgrade/removal, and per-host isolation. Repeat the early proof with the packaged product: a turn continues while the Mac sleeps, session remains available idle or waiting for permission, reconnect retrieves missed events and permits follow-up, deadlines and partial collection run offline, and uncertain requests are never repeated. Continuation is a required gate for shipping remote hosts.
- [ ] Runs from tasks: New run from a task, the run shown on the task, and runs waiting on the user in Today.

**Status, 5 October 2026, end of the day:** the blocking spike passed on a remote Linux host and on Docker Desktop and OrbStack on the Mac (`architecture.md`, Decisions, 5 Oct 2026). Phase 1 is built and committed on the `v0.5-agent-runs` branch (six commits on top of Settings → Agents; not pushed, not merged, not released): its behavior is [`SPEC.md`](../SPEC.md) section 13 and its design [`architecture.md`](architecture.md), Agent runs — v0.5. An independent review compared the implementation with the spike and found it faithful; its six pre-release findings are fixed and tested (a Discard that leaked the workspace file, a Test that could pass a refused key, kept runs stranded by a rebuilt image, undeletable rows, folders in Choose files to add…, and answered Start errors shown as interruptions).

- **Built:** Settings → Agents (engine, token or key through the credentials layer, agreement, image pinned by digest, the controller's state, and Test); New run from a local repository; the run controller, `brainiac runner`, with its guard (the container on the chosen engine with the start fetched into a workspace of exactly the run's size, the ACP session, permissions in both modes, follow-ups never sent twice, Cancel and Finish, the deadline that counts sleep, the filtered journal, stops confirmed by the engine that keep the work, interrupted runs after a controller restart); the collector and the verified import of the snapshot; `AgentRunService` with `agent_runs`, the journal mirrored to the Mac, reconnect at launch, Cancel kept until the controller answers, collection on its own, Changes in the diff viewer with Copy patch and Save patch…, the left-out files with Choose files to add… and Keep this snapshot, Collect work for an interrupted run, Retry collection, Discard work…, Delete run…, Retry cleanup, and 30-day retention; and the Runs section with its badge, the list, and the run view. Tested through the controller's socket with a stand-in engine and the real collector script (28 tests), and on OrbStack with the real image (a refused key, the loop workspace, collection, discard, and a discard after the volume went). The full `pnpm check` gate passes.
- **Decided:** Brainiac starts the controller when it needs one (not a launchd agent); the controller uses `bollard`; a run's workspace is an ext4 filesystem of its size on a loop device attached by a privileged helper (`architecture.md`, Decisions, 5 Oct 2026).
- **Next, before 0.5.0, in this order:**
  1. A real run with a working key on OrbStack: `pnpm tauri:dev`, Settings → Agents from nothing, Test, New run on a small repository, a follow-up prompt, a permission in Ask mode, Finish, review, Save patch…. Every automated run so far used a key Anthropic refuses, so a true model reply has only been exercised through the fake agent.
  2. The same on Docker Desktop: whether its VM attaches loop devices for the workspace is still unconfirmed.
  3. The deadline with the lid closed: start a run with a short limit, sleep the Mac past it, and check the run ended "expired while asleep".
  4. **Export files…** for a failed collection (`SPEC.md` lists it; not built), the Workspace full state (nothing detects a full filesystem yet), workspace use in the side panel (only the limit is shown), and New run from a repository's page (only Runs, the palette, and the menu open the dialog).
  5. The collector's 200 MB limit against a real large file, and the review's remaining low findings: the controller started several times at once when many runs reconnect (each start truncates `runner.log`); journal duplicates after a crash between the mirror's append and its cursor; the helper container left when a launch is cancelled during it; the memory floor of 2 GB where the spike needed 3 GB; Collect work shown for runs the service is about to collect itself. Test gaps: nothing exercises the Docker layer except the opt-in real-engine test; binary files, symlinks, executable bits, and a deleted `.gitignore` in the collector; the protocol against oversized or partial lines; the controller's SIGTERM, idle exit, and journal-full paths; retention.
  6. The exit gate below, by hand, for a week; then `pnpm release 0.5.0`.
- **Open:** Anthropic's answer on the subscription token (the API key does not wait for it; the plan option is compiled in only for debug builds until then); a launchd agent so runs survive logging out; keeping the key out of the agent's processes for repositories the user does not trust; file paths in the left-out list and the diff are not redacted, so an agent naming a file after the key would put it in `history.db` (within the stated best-effort limit; to note in `SPEC.md`).
- **Also pending, outside v0.5:** 0.4.0's draft release is built but not published (`pnpm release:publish v0.4.0`); the v0.4 and secrets exit gates by hand; v0.2.x's exit gate against a packaged build.

Prove the remote controller architecture before production runtime implementation. Then local runs with one agent may ship first, landing next, followed by production remote hosts and tasks. Local and remote use the same controller/session contract; client disconnect or app restart does not interrupt a healthy controller. Mac sleep suspends local execution, while remote execution remains independent of the Mac. The remote proof is repeated as a phase-3 product release gate.

**Exit gate:** for a week, hand real tasks to agents from Brainiac instead of a terminal. Set up Claude Code and Codex from nothing; save uncommitted/binary results from a repository with no remote; cancel, steer, reconnect, and recover failed runtimes; retry collection/cleanup without losing work; publish the exact reviewed snapshot to a new branch and reconcile a lost response, with local refs/index/config/working copy unchanged. Before remote support ships, close Brainiac and sleep the Mac during a remote turn, prove host-side progress, reconnect to the same session with complete journal replay, and send another prompt; also verify offline permission waiting, expiry/collection, lost acknowledgements, controller-crash termination, and emergency stop. Test allowlist bypasses, restored approvals, resource cleanup, and known injected credentials absent from configuration, journals, logs, and IPC. Exact-value filtering cannot certify arbitrary agent output or artifacts as secret-free.

### v0.6 — Content imports and global capture

- [ ] Inbox view for captured and imported items, with triage: file or move, link to a task or repository, create a task.
- [ ] Text/selection capture, URL bookmarks, Markdown copies, source provenance, resumable import jobs.
- [ ] Public article extraction with bookmark/paste fallback.
- [ ] `.eml` import, safe body conversion, message identity, original retention and export/restore.
- [ ] Duplicate/update preview, personal-annotation preservation, source filters, content-scope labels.
- [ ] Video reference and supplied timestamped transcript support; validate richer transcript acquisition separately.
- [ ] Note templates: a templates folder in the vault, New Note from Template and Insert Template with a preview, Obsidian's date syntax, rendered in Rust with bounded work.
- [ ] Kindle highlights from `My Clippings.txt`: one note per book from editable templates, books in any device language, re-import that adds only new highlights after a preview and keeps the user's edits.
- [ ] Configurable global shortcut, floating capture, menu bar entry, background lifetime preference.
- [ ] Shared backend state, snapshot recovery, capture draft retention, macOS focus tests.

**Exit gate:** import representative articles/emails/issue excerpts/video references, find their captured content offline, identify metadata-only items, preserve annotations, retry failed imports without duplicates, and capture from another app without losing drafts or main-window updates. Create notes from an Obsidian template copied unchanged; import a `My Clippings.txt` from a Kindle in another language, then import again after new and extended highlights, with the book note renamed, moved, and annotated in between, and nothing duplicated or overwritten.

**v0.6.x follow-up:** add one authenticated source adapter chosen by daily use, Jira or the email provider. Validate its authentication/deployment and selected-item import before expanding providers. Native share/browser entry points and a forwarding service remain separate follow-ups.

### v0.7 — Semantic retrieval

- [ ] Validate sqlite-vec/Rust/bundled-SQLite release integration.
- [ ] Choose and document a local embedding profile using actual notes.
- [ ] Chunk/hash/version pipeline, bounded indexing, progress, cancellation.
- [ ] Vector retrieval, hybrid ranking, model-change reindexing.

**Exit gate:** a labeled set of roughly 30 real queries demonstrates useful retrieval beyond keyword search; deleted/stale chunks are excluded and Ollama failure leaves core search functional.

### v0.8 — Grounded AI answers

- [ ] Context assembly, generation model configuration, streamed channel output.
- [ ] Openable citations, stale-source labeling, insufficient-evidence behavior.
- [ ] Cancellation/timeouts and explicit Save as note.

**Exit gate:** representative questions produce answers supported by the displayed notes, empty-evidence cases are handled honestly, and no generated output writes files or executes commands automatically.

### Later — Life management and integrations

Expand the v0.6 import adapters and consider pull requests on GitLab and Bitbucket Data Center; CI beyond the checks a pull request shows; the In flight view of each change from local branch to merged (Pull request follow-ups, below); Linear references; local calendar context; recurring tasks and reminders; daily/weekly review notes built on v0.6 templates; Kindle highlights from Amazon's cloud notebook or Readwise (`design/templates-and-kindle.md`, Where highlights come from); optional graph navigation; explicit branch/dev-server actions; multiple vaults; sync; automation or plugin APIs; a Linux build, whose secrets would go to the Secret Service (`design/secrets.md`, phase 3).

Remote services require their own authentication, rate-limit, cache, error, and privacy requirements. Polling with backoff is sufficient for an initial desktop integration; webhooks need an explicitly designed delivery mechanism. Do not make these dependencies of the core app.

## Open questions

| Unknown | Resolution method | Needed before |
| --- | --- | --- |
| Syntax in vaults brought from other tools | No existing vault to sample: a fixture vault covers CommonMark, GFM, wikilinks, and common Obsidian syntax (callouts, footnotes, embeds), which is kept as text; document what Brainiac does not style | v0.2 compatibility claim |
| macOS minimum and Intel requirement | Verify target hardware, dependencies, and packaged builds | Release distribution |
| Real-world workspace size and Git cost | Benchmark representative workspace/repositories | v0.1 watcher tuning |
| SSH tunnels for databases from the start | Check whether the databases used at work are reachable without a bastion; a tunnel opened in Terminal or the Cloud SQL Auth Proxy is the fallback | v0.4 read-only client |
| Hosted PostgreSQL providers to support | Choose among Cloud SQL, RDS, Supabase, and Neon; the spike verified TLS only against a local CA, so the exit gate is the first test against a hosted provider | v0.4 exit gate |
| Health on Cloud SQL | Whether `pg_monitor` can be granted and `pg_stat_statements` is on; Google credentials from `gcloud` only, or also a service account key (`design/databases.md`, Open questions) | v0.4 Health |
| Commands as secret sources from a GUI app | An app opened from Finder gets a minimal `PATH` and no terminal: check where `gh`, `op`, and `gcloud` are found, and how long a password manager's unlock window takes (`design/secrets.md`, Open questions) | v0.4.x secrets |
| Sending an agent's work to a cloud model | v0.8's AI answers go only to a local endpoint by default; agent runs send code to the provider of the agent the user set up, with their own key or Claude subscription token. Record it as a decision with an explicit opt-in in setup | v0.5 setup |
| ACP adapters and remote session ownership | The early v0.5 spike: a pinned Claude adapter over controller-owned stdio with no Mac filesystem/terminal; prove remote progress through Mac sleep, same-session reconnect/follow-up, offline deadline/permissions, and crash termination. Add Codex when supported (`design/agent-runs.md`, Open questions) | Before production runtime implementation; repeat for remote release |
| Quick-capture focus behavior | macOS/Spaces/fullscreen prototype | v0.6 |
| Reading `My Clippings.txt` from current Kindles | Check whether recent models mount as a USB drive or use MTP; collect files in several device languages, with notes and export-limited books (`design/templates-and-kindle.md`, Open questions) | v0.6 Kindle device detection and language table |
| Vault size and note sizes | Generated vaults of 1,000 and 10,000 notes measured (S3; `docs/architecture.md`, Decisions); check against the real vault after a month of use, with a cold file cache | Confirming v0.2 batch sizes and revision budget |
| Snapshot size and retention | Measure `brainiac.db` and `history.db` on a real vault after a month | v0.2 backup defaults |
| Restore onto another Mac | Fixture: vault and snapshot restored where paths and repository locations differ | v0.2 restore |
| Embedding model and dimensions | Local retrieval evaluation, model availability and license review | v0.7 schema/profile selection |
| Chunking notes, code, and imports | Labelled set of about 30 real queries; compare heading-aware splits, size caps, and overlap by recall@k | v0.7 chunker version 1 |
| SQLite-vector integration compatibility | Build and query a bundled release executable | v0.7 |
| `sqlite-vec` approximate indexes | Track whether ANN indexes reach a stable release; brute force is enough at personal scale | Only if a vault outgrows brute force |
| Remote integrations and company data constraints | First providers chosen: GitHub and Bitbucket Cloud (v0.3). Confirm the scopes each employer allows and whether caching pull request content locally is acceptable | v0.3 accounts |
| Where the daily view of pull requests lives | Use v0.3's workspace tab for a few weeks, then choose a pull requests inbox in the sidebar or a group in Today (Pull request follow-ups, below) | v0.3.x |
| Preferred mail source and capture gesture | Choose `.eml`, connected mailbox, or optional forwarding workflow | Email connector implementation |
| Jira deployment/authentication | Confirm Cloud versus Data Center and company-supported access | Jira connector implementation |
| Article extraction and transcript availability | Evaluate representative pages/videos; document fallback coverage | Rich import compatibility claims |
| Sync expectations and encryption | Separate design; define conflicts and authoritative stores | Any sync implementation |

These unknowns do not prevent implementing the core domain and persistence services. The first concrete development step is M0, followed by the smallest complete open repository → inspect status → view diff/history → track changes workflow.

## Designs for later releases

v0.2's design (views, notes, tasks, search, storage, and backups) moved to [`SPEC.md`](../SPEC.md) sections 5–8 and [`architecture.md`](architecture.md) when the release started, v0.2.x's agent access to `SPEC.md` section 9 and `architecture.md`, Agent access, v0.3's pull requests to `SPEC.md` section 10 and `architecture.md`, Pull requests — v0.3, v0.4's databases to `SPEC.md` section 11 and `architecture.md`, Databases — v0.4, v0.4.x secrets phase 1 to `SPEC.md` section 12 and `architecture.md`, Secrets — v0.4.x, and v0.5's agent runs, phase 1, to `SPEC.md` section 13 and `architecture.md`, Agent runs — v0.5.

### Secrets — v0.4.x

Phase 1 shipped in 0.4.0. Its behavior is [`SPEC.md`](../SPEC.md) section 12 and its design is [`architecture.md`](architecture.md), Secrets — v0.4.x. Phases 2 and 3 remain in [`design/secrets.md`](design/secrets.md).

### Agent runs — v0.5

Phase 1 (local runs and review) is in [`SPEC.md`](../SPEC.md) section 13 and [`architecture.md`](architecture.md), Agent runs — v0.5. Phases 2 to 4 (landing and more agents, remote hosts, tasks), the background, the spike records, and the open questions remain in [`design/agent-runs.md`](design/agent-runs.md).

### Additions of v0.6 onward to storage and contracts

#### Data model of later releases

| Entity | File | Essential fields and constraints | Release |
| --- | --- | --- | --- |
| `note_sources` | core | `note_id`, source type, canonical URL/provider ID, source time, import time, content scope, source hash; derived from note provenance | v0.6 |
| `import_jobs` | core | `id`, input reference, adapter, state, staged payload reference, error, result note ID, idempotency key | v0.6 |
| `import_items` | core | adapter, item ID (such as a Kindle highlight's), `note_id`, first seen, hash of the block as rendered, disposition (`added`, `superseded`, `dismissed`); the ledger that stops a deleted item from returning on re-import | v0.6 |
| `embedding_profiles` | core | `id`, provider, model name and digest, dimensions, distance metric, normalization, chunker version, state | v0.7 |
| `note_chunks` | index | `id`, `note_id`, profile, source hash, heading, byte/line range, text, chunk hash | v0.7 |
| vector tables | index | One `vec0` table per embedding profile: chunk ID, embedding, filter columns | v0.7 |

#### Commands of later releases

| Command | Important input/output |
| --- | --- |
| `prepare_import` / `commit_import` / `list_import_jobs` | Source input or job ID; preview/provenance, duplicate choice, resulting note, retry state |
| `configure_shortcut` | v0.6: validated binding and registration result |
| `list_templates` / `render_template` / `create_note_from_template` | v0.6: template kind, path, title, and folder; rendered text with cursor offset and undefined names, or an error with line and column; the new note |
| `semantic_search` / `ask_brain` / `cancel_job` | v0.7+: profile/request IDs, results or response channel |

#### Shortcuts of later releases

| Shortcut | Action |
| --- | --- |
| `Cmd+Shift+Space`, v0.6 | Configurable global quick capture |

### Pull request follow-ups — v0.3.x and later

v0.3's design moved to `SPEC.md` section 10 and `architecture.md`, Pull requests — v0.3. These were designed with it and left out:

- **The daily view of what waits on you, across workspaces (v0.3.x).** Two designs were compared: a Pull requests inbox in the sidebar, with Today unchanged; or a "Pull requests waiting on you" group in Today, folded like To sort, with a review you submitted listed under Completed today and `Cmd+K` finding any pull request. Choose after using v0.3's workspace tab; either is a filter over data v0.3 already has.
- **A task's pull request (v0.3.x):** an explicit link from a task to a pull request, shown with its state in Today and Tasks.
- **Issue keys (v0.3.x):** recognize issue-tracker keys such as `ABC-123` in branch names, commit subjects, pull request titles, and task titles, from local Git and v0.3's pull request cache, with no tracker account or API. A task whose title carries a key shows the branches and pull requests with that key and their state, and a key opens the issue in the tracker. Settings lists the project keys to recognize, so `UTF-8` or `SHA-256` never match, plus the tracker's base URL. This replaces syncing tasks with Jira for now (`design/task-connections.md`, Status).
- **Create a pull request from a pushed branch (v0.3.x):** title from a linked task, description from the commits, reviewers suggested from local history; Brainiac does not push.
- **Agent tools (v0.3.x):** read pull requests, diffs, and conversations, and add review drafts the user opens and sends; agents never submit, approve, merge, or close. Pull request text is written by other people, so agents are told to treat it as data.
- **In flight (Later):** each of your changes from local branch to merged (local only, pushed without a pull request, in review, needs changes, ready, merged), with the reviews you owe beside them. It needs every local branch's commits ahead of the default branch and matching branches to pull requests across forks and reused names, which is where most of its cost lies. Store each pull request's head repository, branch, and commit from v0.3 so it needs no migration.

### Capture and imports — v0.6

Import is a core way to create knowledge, available from the same capture interface as writing a note. Sources feed a common Rust ingestion pipeline and become ordinary Markdown notes in the Inbox. Importers do not create separate knowledge silos or require AI to produce a useful result.

#### Capture interface

Provide an **Add to brain** action in the main window and command palette. Accept pasted text, a URL, or a selected/dropped supported file. Show the detected source and allow the user to correct the title, add a personal comment, and choose a destination. Default to `Inbox/`.

After saving, the item is immediately readable and searchable using the content actually captured. Show a small source badge, capture date, and **Open original** action. The inspector exposes provenance without making the note editor feel like an import-management tool. Inbox triage offers file/move, link to a task or repository, and create a task explicitly.

Later, the same capture action can be reached from global quick capture, a browser extension, or a macOS share extension. Those entry points submit to the same backend pipeline. Native share-extension packaging and browser handoff need their own technical spike; they are not assumed to come automatically with Tauri.

#### Source behaviors

| Source | Captured note | Initial path | Richer integration |
| --- | --- | --- | --- |
| Pasted text/selection | Supplied text, optional source URL, and personal comment | v0.6 paste | Browser extension supplies selected text and page metadata |
| Markdown file | Independent copy in the vault, preserving original source reference | v0.6 picker/drop | Batch import with preview |
| Web page | Article title, source URL, author/date when available, extracted readable body | v0.6 URL bookmark; v0.6 extraction | Browser capture for authenticated or client-rendered pages |
| Email | Subject, sender/recipients/date, selected message body, message identity | Paste in v0.6; `.eml` in v0.6 | Selected-message import through an authorized mailbox connector |
| Jira issue | Issue key/title, description, status/assignee snapshot, source link | Paste text or bookmark in v0.6 | Explicit fetch through configured Jira deployment/account |
| YouTube video | Video link/title and user's notes; timestamped transcript when supplied/available | Bookmark and pasted transcript in v0.6 | Metadata and supported transcript acquisition |
| Kindle highlights | One note per book: highlights and the user's Kindle notes in book order, rendered from templates | `My Clippings.txt` picker/drop in v0.6 | Cloud notebook or Readwise, only as a separately reviewed opt-in |

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
- Retain raw originals such as `.eml` in a vault-local `attachments/imports/<import-id>/` folder in v0.6. Save relative references in frontmatter and include them in export/restore. Unsupported attachments stay as originals, with no implied indexing.
- Imported Markdown copies get a new Brainiac ID; preserve any original ID as source metadata so copying does not create identity collisions. Do not modify the source file. Detect dependencies such as relative images and warn when they cannot be copied/resolved by the supported importer.
- Source caches are reconstructible from provenance. Pending acquisition credentials and job state stay outside note files.

#### Duplicates and updates

Deduplicate by provider account plus stable item ID where available: Jira issue ID, email Message-ID/provider message ID, video ID, or conservatively normalized URL. Use content hashes as a second signal, not the sole identity for unrelated items.

For a repeated unchanged capture, offer **Open existing** or **Save another copy**. If the source changed, offer an explicit new snapshot or a previewed update of the imported section. Never silently overwrite a user's annotations or independently edited note. URL normalization must not strip query parameters that identify different content.

Snapshot import is the default. Live synchronization, scheduled bulk acquisition, and remote mutations are separate later features with their own contracts.

#### Extraction limits and search behavior

- Apply bounded download sizes, redirect counts, timeouts, and cancellation. Use source-specific parsers and strip active HTML, scripts, and tracking pixels.
- Generic URL import fetches public `http`/`https` pages; it does not borrow browser cookies, bypass login walls, or silently execute page JavaScript. If extraction fails, offer bookmark-only or paste selection.
- Authenticated/provider imports use explicit configured accounts, with credentials through v0.4.x's credentials layer (`design/secrets.md`). Configure company Jira hosts deliberately rather than forwarding credentials to arbitrary pasted hosts or redirects.
- Keep external page resources from triggering incidental downloads. Fetch attachments only through an explicit supported import action.
- Index the resulting Markdown using the existing FTS/chunking pipeline. Add source-type filters and display original-source provenance on results.
- RAG uses only captured bodies/transcripts and distinguishes personal comments from source excerpts. It cannot answer from the uncaptured contents of a saved URL.
- Import verification covers duplicate retries, source identity collisions, partial failures, MIME/HTML conversion, unavailable transcripts, user edits during refresh, and backup/restore of original files.

#### Import rollout

1. **v0.6 core:** text/selection capture, URL bookmarks, Markdown copies, provenance, Inbox triage, duplicate/retry handling.
2. **v0.6 source adapters:** public-page extraction and `.eml` parsing/original retention, with explicit bookmark/paste fallback; Kindle highlights from `My Clippings.txt`, after note templates.
3. **v0.6.x:** one authenticated connector, selected by daily use—Jira or the email provider—and validated video metadata/transcript improvements.
4. **Later:** browser/share entry points, optional forwarding service, batches, and explicit synchronization.

All external content-import behavior starts in v0.6. Selecting local repositories and discovering them inside a folder in v0.1 is workspace configuration, separate from importing knowledge content.

**Acceptance scenario:** capture an email, an issue excerpt, an article, and a video reference into the Inbox; add commentary, create a task from one item, find captured material offline, open each original source, retry an interrupted import without duplicates, and identify which items contain only metadata.

### Note templates and Kindle highlights — v0.6

The full design, with the research behind it, is in [`design/templates-and-kindle.md`](design/templates-and-kindle.md).

- **Templates first.** Templates are ordinary notes in a templates folder of the vault, chosen in Settings. **New Note from Template…** creates a note from one and **Insert Template…** inserts one at the cursor, both after a preview. Obsidian's `{{title}}`, `{{date}}`, `{{time}}`, and `{{date:YYYY-MM-DD}}` work unchanged, so templates can be copied from an Obsidian vault. The new note never inherits the template's `brainiac_id`, and links inside templates are not indexed as the vault's.
- **Rendering in Rust with MiniJinja.** Its Jinja syntax is close to the Nunjucks of Obsidian's Kindle plugin. Its `fuel` feature bounds the work of a render; there is no template loader, no function that reaches files or the network, and output stops at 5 MiB. Missing values render empty and are listed in the preview.
- **Kindle highlights from `My Clippings.txt` only.** It is the one source that covers sideloaded books and personal documents, and it needs no account. Amazon's cloud notebook covers only purchased books, needs the user's Amazon session, and is excluded by Amazon's Conditions of Use for extraction tools, so it stays out of v0.6.
- **Parsing.** The file has no published format and follows the device's language: parse kind, location, page, and date through a keyword table per language, and fall back to the entry's structure. Drop exact duplicates, show extended or edited highlights as revisions, and label highlights cut short by publishers' export limits as truncated.
- **One note per book from two templates,** one for the note and one for each highlight, with variable names taken from Obsidian's Kindle plugin. The note template renders once and is the user's afterwards; Brainiac owns only `brainiac_source` and the highlight blocks it adds.
- **Re-import adds, never rewrites.** The book's note is found by its `brainiac_id` through a book key (normalized title and author), not its path. Each highlight carries an Obsidian block ID, `^kh-…`, from the book, its location, and its text. New highlights go next to their neighbours in book order after a preview; a revised highlight replaces its block only if the user has not edited it. `import_items` in `brainiac.db` remembers what was imported, so a highlight the user deleted does not return.

### Global capture and window state — v0.6

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

### Semantic search — v0.7

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

### Ask my brain — v0.8

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
