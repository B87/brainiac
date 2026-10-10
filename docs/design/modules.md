# Design: modules and extensions

Notes on splitting Brainiac into a small core and modules that plug into it, and later letting people install extensions that others publish. The example that started it: notes as a self-contained module, kept in its own repository and installed by users who want it. Nothing here is on the roadmap or in scope for any release.

The request holds two ideas with very different costs. Keeping them apart is the main point of this file:

- **Modules:** Brainiac's own features (notes, tasks, databases, pull requests, agent runs) each live behind one interface to a small core, so adding or removing one does not touch the others. This is internal structure. It is useful now, it adds no dependency, and it is what anything later builds on.
- **Extensions:** code written by other people, downloaded from a catalog and run inside Brainiac. This is a security product with a public API that must stay compatible for years. It only makes sense once modules exist.

## Status: idea, not planned (written 7 Oct 2026, reviewed 10 Oct 2026)

Nothing is built. The Preparation section lists the few changes worth making before any decision, because they make the code simpler on their own.

The review on 10 October read the file against v0.6 (explaining changes, 52 commits after the first draft). Its findings are in What v0.6 showed, the import table, the Preparation list, and the reshaped module interface (An entity kind is the deep abstraction; Actions are contributed by the consumer; Start from `install`, not a manifest).

## Why

- **Adding a feature touches everything.** Databases (v0.4) and agent runs (v0.5) each had to edit:
  - the hand-written wiring in `src-tauri/src/lib.rs`, 876 lines with one block per feature;
  - `commands.rs`, which holds all 162 commands;
  - `models.rs`, 3,901 lines of every feature's DTOs;
  - the single migration sequence of `brainiac.db`;
  - the closed `View` union in `src/components/Sidebar.tsx`;
  - `App.tsx`, `Sidebar.tsx`, `CommandPalette.tsx`, and `SettingsPanes.tsx`, each of which imports every feature;
  - `secrets.rs`, which matches on each owner kind;
  - `ipc.ts`.

  That is change amplification, and it grows with every release. Explaining changes (v0.6) did it again in one release:

  | File | 7 Oct 2026 | 10 Oct 2026 |
  | --- | --- | --- |
  | `lib.rs` | 876 lines | 1,020 lines, 416 of them the setup block |
  | `commands.rs` | 162 commands | 187 commands |
  | `models.rs` | 3,901 lines | 4,740 lines |
  | Event names in `lib.rs` | | 12 constants, one or two per feature |
- **Not everyone wants everything.** Someone who uses Brainiac only as a Git dashboard with databases pays for notes' indexing, settings, and sidebar section.
- **Others could build what the maintainer will not.** Issue trackers (Jira, Linear), error monitoring (Sentry), cloud consoles, more database drivers, more forges: each is a feature some users need and the maintainer may never write.

## What the user gets (sketch)

- **Settings → Modules** lists the built-in modules (Notes, Tasks, Databases, Pull requests, Agent runs), each with a switch. A module that is off has no sidebar section, commands, settings, agent tools, or background work, and keeps its data until the user removes it.
- Later, **Extensions** in the same place:
  - **Browse** shows a catalog. **Install** shows what the extension asks to do (Permissions, below) before anything is downloaded.
  - **Update** and **Remove** work like any app's. Removing asks whether to keep the extension's data.
- An extension's views, commands, and search results look like Brainiac's own, with a small mark that says which extension they come from.

## Where the boundaries are today

Which backend modules import each feature (`crate::<module>` outside its own folder), on 10 October 2026:

| Feature | Imported by |
| --- | --- |
| `notes` | `backup`, `commands`, `explain`, `index`, `lib`, `mcp`, `tasks`, `vault` |
| `tasks` | `backup`, `commands`, `lib`, `mcp`, `notes` |
| `workspaces` | `agents`, `commands`, `explain`, `forge`, `lib`, `mcp`, `watcher` |
| `git` | `activity`, `agents`, `commands`, `db`, `explain`, `fetcher`, `forge`, `workspaces` |
| `credentials` | `agents`, `commands`, `databases`, `forge`, `secrets` |
| `sharing` | `agents`, `commands`, `explain` |
| `forge` | `commands`, `db`, `secrets`, `workspaces` |
| `databases` | `commands`, `secrets` |
| `agents` | `commands`, `explain`, `secrets` |
| `explain` | `commands` (until 10 Oct 2026 also `agents` and `forge`; Preparation) |

What it shows:

- **Notes is the most entangled feature, not a self-contained one.**
  - It owns part of the storage: `notes::Stores` (`notes.rs:48`) receives `brainiac.db` from `lib.rs` but opens `index.db` and `history.db` itself. Databases, agent runs, and explanations get their `history.db` handle from it (`stores.history` in `lib.rs`).
  - Tasks are built on it: `TaskService::new(notes)`, and `KnowledgeEvent` carries both note and task events.
  - Backup, search, agent access (`mcp.rs`), and Save as note (`explain/service.rs`) call `NoteService` directly.

  Externalizing notes first would mean solving every hard problem at once.
- **Notes and tasks are one module in practice.** Splitting them is a decision in its own right (Open questions).
- **Databases is the one leaf.** Only the command layer and Settings → Secrets import it. Agent runs was a leaf on 7 October and v0.6 made `agents` and `explain` import each other, and `forge` import `explain` (What v0.6 showed). The leaks were reversed on 10 October, so explain now sits on agent runs and forge with nothing importing it back; agent runs is again imported only by the command layer, Secrets, and explain. Databases is still the right first module.
- **Repositories and Git are the hub.** Almost every feature links to a repository. They belong to the core (below).
- **`db` and `sharing` are already core in all but name.** `db` is imported by thirteen modules. `sharing` (whether a repository's code may go to a provider) is one small service that two features depend on, and neither imports the other for it. It is the shape every shared concern should take.
- **The frontend has one seam, made by hand.** The `View` union is closed, and the sidebar, palette, and settings import each feature's components by name. v0.6 added a hook, `useExplainedPatch`, that a view calls to put an explanation beside its patch; four views call it, each rendering its own `DiffView`.

## What v0.6 showed

Explaining changes is the first feature that mostly extends other features' items instead of owning its own: an explanation is an agent run of a kind, a panel on a commit, a tab on a pull request, a pane in Settings, and a note once saved. The doc predicted the amplification and v0.6 confirmed it. It also produced three shapes the contribution points below had no row for, and a few leaks small enough to reverse now.

- **A feature-named flag on a shared row.** `AgentRunService` gained an `explain` boolean on its rows, `start_explain`, `discard_explain`, a filter that hides those runs from the Runs list, and readers for the run's file, cost, and opened files. The run service now carries one feature's policy by name, and a second owner of runs would add a second boolean. The accepted decision (an explanation is an agent run) stands; what should change is the interface. What the run service offers in general is a run someone else owns: a prompt, a file to read back, where the run is listed, and what happens to its container when the owner is done. Those are properties of a run request, with an opaque owner, not a feature name the service matches on.
- **Modules depend on modules.** Explain requires agent runs. The test below, "a module can be removed and every other keeps working," needs a `requires` relation: explain is off when agent runs are off, and its items show as unavailable. The file read as if every module were a peer of the core alone.
- **Decorating another module's item.** The explanation panel sits beside the patch of a commit, a branch comparison, a run's result, and a pull request. Each of those four views calls `useExplainedPatch` and renders its own `DiffView`. One patch component that takes a subject and asks the core for its decorations would let a fifth subject, or a second decorator, edit nothing. (10 October 2026: true for the branch comparison and a run's result, which now share one file list; not for commit details and pull requests, Preparation.) The same question returns for the backend: Explain is an action on anything with a commit range, and today each subject is wired by hand (`explain/subject.rs`).
- **Leaks, reversed on 10 October 2026 (Preparation):**
  - `forge/service.rs` returned `explain::service::PullRequestFacts`. Explain already reaches forge through a callback (`PullRequestReader`), which is the right seam; only the type lived on the wrong side. It is now `forge::PullRequestFacts`.
  - `agents/repository.rs` wrapped `explain::subject::read_subject` and `read_files`. The first reading of this review was that the readers belong in `agents` or `git`; the code says otherwise: `Subject` and its changed lines exist for anchoring notes, so they are explain's. What `agents` really lent explain was an isolated Git runner (the Git service, its empty home folder, a timeout). That is now `git::IsolatedGit`, which `RunArtifacts::isolated_git` hands out and explain's readers take, and the wrappers are gone.
  - `agents/image.rs` called `explain::store::concept_key`, but only in tests: the image ships the `known` script, and its tests checked that the script folds names as the ledger does. The script's contract (the file's columns and the fold) is explain's, which already writes the file (`explain/known.rs`), so the tests moved there and read the script from the image folder. The image still embeds the script, as it embeds every tool the container needs.
  - `explain/service.rs:38` imports `NoteService` for Save as note. The fix is not a callback but an inversion (Actions are contributed by the consumer, below).
- **Two things went right.** `sharing.rs` is the fix for shared knowledge: one small module both features depend on, with its own emitter, instead of one service holding the other. And explain added no `View` kind, because it is a panel beside existing views; the frontend's cost was the four hook calls and one Settings pane.
- **Repetition the core would remove.** Eleven type aliases of the shape `Arc<dyn Fn(Event) + Send + Sync>` (`KnowledgeEmitter`, `HealthEmitter`, `ExplanationEmitter`, `RunEmitter`, `SharingEmitter`, the workspace `Emitter`, `PullRequestEmitter`, and the notifiers), each wired in `lib.rs` to one of twelve event-name constants, and each subscribed to by name in the frontend. One typed channel in the core, with a module's event type as its parameter, replaces all of them.

## The core

The core is what every module needs and no module should own. It should stay small, because everything in it is permanent:

- **The shell:** window, navigation, and the frames of the sidebar, palette, settings, status bar, and side panel, which modules fill.
- **Storage:** opening database files, a migration list per module, snapshots, export and restore. Modules declare what they store; the core decides where and how.
- **Entities and links:** a way to name any item (`note:<id>`, `db.connection:<id>`, `repository:<id>`), open it, pin it, find it in the palette, and link two items.
- **Search:** one query fanned out to each module's search provider, with results merged.
- **Events:** each module publishes committed changes to its own items, and others subscribe through the core.
- **Credentials:** needs, bindings, and the secret lifecycle (the credentials review: credential state as rows owned by `credentials/`).
- **Agent access:** the MCP server and its rules. Modules register tools, and the core enforces that no tool deletes, touches Git, changes settings, or reads a secret.
- **Background work:** one scheduler instead of each feature spawning its own ticker in `lib.rs`.
- **Repositories and workspaces:** registration, Git inspection, fetching, and activity. The product starts from them, and every other module links to them.

## The module interface

### The interface should not mirror Brainiac's internals

The tempting plugin API hands modules the existing services: `RepositoryService`, `NoteService`, `CredentialService`, the `Db`. It is quick to build and very shallow:

- Every module depends on how those services are written today, so no service can be changed without breaking modules. For an external module, that means breaking someone else's code.
- Modules call each other directly, which is exactly how notes and tasks became one thing. The dependency graph above would be rebuilt inside the plugin system.

Instead, modules plug into a **few general contribution points** and talk to each other only through the core's entities, links, events, and search. The test: a module can be removed, and every other module keeps working, showing a link to its items as unavailable.

### Contribution points

The first draft listed ten contribution points, each a registry of its own:

| Contribution | What the module provides | What the core does with it |
| --- | --- | --- |
| **Entity types** | Kinds of items it owns (`note`, `task`, `db.connection`, `db.query`, `run`): title, icon, how to open one | Palette, pins, recent items, links, the side panel's "linked items" |
| **Views** | A sidebar section, main views, a tab on another module's entity (a repository's Notes tab), a settings pane | Routes and frames; the `View` union becomes open |
| **Commands** | Palette commands, menu items, default shortcuts | One palette and one keymap with conflicts reported |
| **Search provider** | Query → hits for its entity types | Merges and ranks |
| **Storage** | Its migration list, and whether each file is rebuildable | Opens files, snapshots and exports what is not rebuildable |
| **Events** | Change events for its own entities | Delivers them to the UI, search, agents, and other modules |
| **Credential needs** | "A password for each connection", with where it is sent | Sources, bindings, approval, Settings → Secrets |
| **Agent tools** | MCP tools over its entities | Registration, access rules, the agent access switch |
| **Background jobs** | A job and its interval | Scheduling, and stopping it when the module is off |
| **Backup participation** | Its non-rebuildable data | Export, restore, the approval of restored sources |

Ten thin interfaces over ten small things is the shallow shape: each registry is about as wide as what it does. The review of 10 October reshaped it into three ideas, below. The table stays as the list of what the core must end up doing.

### An entity kind is the deep abstraction

Six of the ten rows are facets of one declaration. A module that owns items of a kind says, once: how they are stored (a migration list and whether the file is rebuildable), how one is named and opened, how it is rendered, how it is searched, what its change event is, and what an agent may read of it. From that one declaration the core derives the palette, pins, recent items, links and the side panel, search, export and restore, the MCP read tools, and later sync (`shared-workspaces.md`). The module writes none of those.

What stays separate because it is not about items: commands, settings panes, credential needs, and background jobs. Four contribution points plus one deep one is a core a module author can hold in their head, and it is the strongest argument for doing modules at all.

### Actions are contributed by the consumer

"Talk only through entities, links, events, and search" left out the case v0.6 hit first: one module wants to do something with another's item. Save as note made explain import `NoteService`.

Invert it. A module contributes an **action** to every entity kind that exposes a **capability**: notes contributes "Save as note" to any item that can render itself as Markdown, and explain only implements that rendering. Explain then does not know notes exist, and pull requests, run summaries, and anything else that renders as Markdown get Save as note without anyone editing them. The same rule gives Explain itself: agent runs contributes "Explain" to anything that exposes a commit range, in place of the four hand-wired subjects. A capability is an interface an item implements; an action is a command whose target is any item with that interface. Both are declared by the module that needs them, so the dependency always points at the core.

Decorations follow the same rule: a module contributes a panel or tab to any entity kind with a given capability (an explanation beside anything that has a patch), and the core's patch component asks for the decorations of its subject. The order in which several modules decorate one item is open (Open questions).

### Start from `install`, not from a manifest

A manifest is a declaration format, and Rust already has one: the trait. Built-in modules need no file to parse. Each module exposes one function that takes the core's shared services (storage, the event channel, repositories, credentials, sharing, the registries) and registers what it contributes. `ExplanationService::new` takes ten arguments and `AgentRunService::new` eight, all threaded by hand through the 416-line setup block; with `install(&Core)` each becomes one call, and `lib.rs` becomes the list of installs. The frontend's counterpart is one registration per module, in place of the imports in `App.tsx`, `Sidebar.tsx`, `CommandPalette.tsx`, and `SettingsPanes.tsx`.

Honestly stated: adding a module then edits one line in `lib.rs` and one in the frontend, not "no core file." A manifest is needed only where code cannot be trusted to call the core: extensions (C, below). `requires` (What v0.6 showed) is part of the trait: a module names the modules it depends on, and the core turns it off with them.

### Links are the core's, not each module's

Today every link has its own table: `note_repository_links`, `db_connection_repositories`, `tasks.linked_note_id`, `tasks.linked_repository_id`. A repository's Notes tab knows about notes and connections by name.

A single links table owned by the core (from one entity to another, with a kind) means:

- a repository's side panel lists linked items from any module, with no change to the repository code;
- a module that is removed leaves links that show as unavailable, as note-to-repository links already do;
- a future extension (an issue tracker) can link issues to repositories and tasks without anyone else changing.

v0.6 added two more links of its own shape: an explanation to its subject (repository, commit or range, agent, depth) and a saved note to the explanation it came from, in the note's frontmatter. Both would be rows in the one table.

This is the most general-purpose piece of the design, and the one that makes modules truly independent.

### Notes as a module

What it would own: the vault and watcher, `index.db`, note revisions and drafts in `history.db`, the editor, the Notes view, the notes search provider, backlinks, the MCP note tools, and the vault's settings.

What it would use from the core: links (note to repository, task to note), search, pins, backup, the palette, and the scheduler.

What must move first: storage ownership (`notes::Stores` into the core), the task–note coupling (`TaskService` built on `NoteService`, the shared `KnowledgeEvent`), and the direct calls from `backup.rs` and `mcp.rs`. That is why notes is the last built-in to become a module, not the first.

## Design it twice: how module code runs

### A. Compiled in (built-in modules)

Each module is a Rust crate and a TypeScript package in the same Cargo and pnpm workspace, registered at build time. A module in another repository is a crate and a package pulled in as a dependency.

- Full speed and full power, with no new runtime dependency. Rust's types check the module interface.
- "Installing" means a different build, so there is no catalog. Turning a module off at run time still works.
- This is how notes can live in its own repository: as a dependency of the app, not as a download.

### B. Extensions as separate processes

An extension is a program Brainiac starts. It speaks a JSON-RPC protocol over a socket, as language servers, VS Code's extension host, and MCP servers do. Brainiac already has both halves of this pattern: `brainiac mcp` and the run controller.

- Written in any language, and a crash does not take Brainiac down.
- It runs with the user's permissions. Declared permissions are enforced only on what it asks Brainiac for, not on what it does itself: it can read `~/.ssh` or a repository's files directly. On macOS, sandboxing an arbitrary unsigned program is not practical.

### C. Extensions as WebAssembly components

An extension's backend is a WebAssembly component run by an embedded runtime (wasmtime), with its interface written in WIT. Its UI is a sandboxed iframe that talks to the app by messages.

- A real sandbox: an extension can do only what the host functions grant. It has no file system, network, or programs unless the user granted them, and then only through Brainiac.
- Brainiac's promises (it never writes to a repository; a secret never reaches the window, a log, or a third party) can hold for code Brainiac did not write.
- Costs:
  - a large dependency;
  - an API designed in WIT;
  - extensions limited to languages that compile to components (Rust, JavaScript, Go, Python, with varying maturity);
  - no native libraries, so a database driver needs the host to provide sockets.

### D. JavaScript in the main window

Extensions run as scripts in the WebView, with access to the app's IPC (the Obsidian model). It is the simplest option and has the largest pool of authors, but any extension can call any command and see everything the window sees. Every hard rule then depends on every extension's author. Rejected.

### Recommendation: two tiers, not one API

Using the same interface for built-in modules and third-party extensions is attractive, because the maintainer then uses the API they ask others to use. But it makes the extension API as wide as the widest built-in module. Notes needs:

- file watching;
- full-text indexing;
- an editor;
- writes to a folder the user chose;
- its own SQLite files.

An API wide enough for notes is wide enough to break every promise above, and it can never change.

So:

- **Modules (A):** the internal interface. It is a Rust trait and a TypeScript registry, as wide as built-ins need, and free to change in any release. All built-in features, including notes in its own repository, are modules.
- **Extensions (C):** a narrow, versioned, sandboxed API, built as *one module* (the extension host) that translates the extension API into contribution points. It is sized for integrations, not for rebuilding notes:
  - an entity type with links;
  - views in sandboxed frames;
  - palette commands;
  - a search provider;
  - network access to declared hosts;
  - credentials used through the host.

These are different layers with different abstractions. Changes to the internal interface stay inside the repository, and the extension API changes only with a new version.

B stays as a fallback for an extension that truly needs native code. It would need a stronger warning at install, since nothing confines it.

## Extensions: trust

### Permissions

An extension's manifest declares what it needs. Install shows the list, and the host refuses anything not on it:

- **Network:** the exact hosts it may reach.
- **Items:** which entity types it may read or write: its own always, others' (tasks, repositories) only if declared.
- **Repositories:** read access to metadata (name, remote, branches); never their files or working tree, and never writes.
- **Credentials:** the needs it declares, such as "an API token for `acme.atlassian.net`".
- **Agent tools:** whether it registers any.

An update that asks for more is not applied until the user allows it, as with a restored secret's source today.

### Secrets are used through the host, never read

An extension never receives a secret's value. It asks the host to send a request to a declared destination with a declared credential, and the host attaches the secret (from the user's chosen source, through `CredentialService`) and returns the response. The rule that a secret is sent only where it belongs is then enforced by the host, whatever the extension's code does.

This is the deepest single decision in the extension API: one call hides sources, caching, refusal, approval, and destinations.

### Distribution

- Packages are signed by their publisher, and the catalog records each version's hash. Brainiac installs only a version whose signature and hash match.
- Updates follow Brainiac's updater: signed and checked before they replace anything.
- An extension can be disabled remotely (a revocation list the app checks), for a version found to be malicious.
- Installing from a URL, outside the catalog, is possible with a warning, for developers and private extensions.

Running a catalog means review, takedowns, and a service to host. It could start as a curated list of GitHub releases in a repository (Open questions).

## Compatibility

- The extension API has a version, and a manifest states the range it supports. Brainiac refuses one it cannot serve and says which update is needed.
- A module's or extension's storage has its own migration list, and migrations are append-only like Brainiac's.
- Removing an extension removes its storage only after asking, and Export includes its data first.

## How this meets other designs

- **Shared workspaces** (`shared-workspaces.md`): entity types, core-owned links, and committed change events per module are what sync needs. A shared workspace then syncs whatever modules declare, instead of one sync per feature.
- **Credentials:** the needs-and-bindings model, and credential state owned by `credentials/`, are the credential contribution point.
- **Agent access:** today `mcp.rs` knows notes, tasks, and repositories by name. It becomes a registry, and its rules stay in the core.
- **Accepted decisions it would touch:** DTOs defined only in `models.rs` (they would be defined per module), the single migration list of `brainiac.db` (one per module), and the per-feature placement of storage decided centrally (explanations in `history.db`, the ledger and levels in `brainiac.db`; a module would declare what is rebuildable and the core would place it). Each needs a new decision added at the end of `docs/architecture.md`, never edits to the accepted ones.

## Preparation (now, no behavior change)

Each of these simplifies today's code on its own:

- **Storage belongs to the core.** `brainiac.db` is already opened in `lib.rs` and handed to `notes::Stores`; move the opening of `index.db` and `history.db` the same way, into `db`, so databases, agent runs, and explanations stop getting their `history.db` handle from notes.
- **Reverse the v0.6 leaks** (What v0.6 showed). Done on 10 October 2026, with no behavior change: `PullRequestFacts` moved to `forge`; `agents` hands explain a `git::IsolatedGit` instead of wrapping explain's readers; the `known` script's tests moved to `explain/known.rs`. `agents` and `forge` import nothing from `explain`. The lesson for the next leak: look at what the borrowed function needs before moving it, because the leaked thing may be the dependency it was given, not the function.
- **No feature-named flags on shared rows.** The next thing that owns a run says what it needs (a prompt, a file to read back, hidden from Runs, discarded after reading) as properties of the request, and the run service matches on those, not on `explain`. When that happens, fold the existing boolean into the same shape.
- **One event channel.** Done on 10 October 2026, with no behavior change. `events.rs` holds one generic `Emitter<E>` and one `Notifier`, which replace the eleven aliases; a module names its own events by implementing `FrontendEvent` beside the code that emits them, which replaces the twelve constants in `lib.rs`; `lib.rs` connects every emitter with `to_window` and every notifier with `notifier`, in one line each. A test compares the names the Rust sources send with the names `src/lib/ipc.ts` listens for, so the one name still written twice, in Rust and in TypeScript, cannot drift unnoticed. An event is lent to the emitter (`&E`), not given, because a host job is sent on every log line. Not done: a typed subscription between modules (the explain service still follows runs through a broadcast channel `lib.rs` feeds from the run emitter), which waits for a second module that needs one.
- **One patch component.** First written as `DiffView` plus the decoration hook in one place that takes a subject, so the four views that call `useExplainedPatch` call nothing. Reading the views on 10 October 2026 showed that only half of that holds. The branch comparison and a run's result had the same file list, about sixty lines each, and now share `ChangedFileList` (the one visible change: a branch comparison marks binary files, as a run's result did). Commit details (a folder tree that can be hidden) and a pull request (a filter, review marks, viewed and generated folds, and review threads in the same patch) keep lists of their own built from the same pieces, because one component for all four would need a slot for almost everything each view does, an interface as wide as the views. A neutral decorations layer over `useExplainedPatch` was also left out: with one decorator it would only pass the hook's values through. When a second decorator arrives, it composes where the pull request's review threads already do (`composeAnnotate`), and the hook's result becomes the place both are gathered; until then, a second decorator edits the four views.
- **New cross-feature reactions go through events or callbacks**, as `set_fetch_listener` already does for pull requests and `PullRequestReader` does for explain, not through one service holding another. A callback's types belong to the side that owns the concept.
- **Links by ID, tolerant of a missing target**, as note-to-repository links already are. A new link table should be shaped so it could become the core's general one.
- **New features as leaves.** A new feature imports the core (repositories, credentials, storage, sharing), and no other feature imports it except the command layer. v0.6 broke this in three places within a week of its start; the rule needs a check (`cargo` has none built in; a test over `grep` of `crate::` imports would do).
- **No plugin dependency** (wasmtime, a manifest format, a registry) until a release takes this design.

## Phases, if it happens

1. **Internal modules, compiled in, one registry at a time.** The first draft read as one step: define the core, then move features into it. Better is to add each piece of the core in the release that next needs it, where it replaces a `match`, a union, or a block of wiring that exists today, and to convert the existing features as that piece arrives. In order of payoff:
   1. the event channel (replaces the aliases and constants; no behavior change; done 10 October 2026);
   2. the entity registry, with links in one table (replaces the `View` union's open-by-id, the palette's imports, and the per-feature link tables);
   3. capabilities and actions (Save as note and Explain stop naming their targets);
   4. `install(&Core)` (replaces the setup block and the frontend's feature imports).

   Move databases first (the one leaf), then agent runs with explain behind it, pull requests, and finally notes and tasks. Done when adding a module edits one line of `lib.rs` and one registration in the frontend.
2. **Modules can be turned off** in Settings, with `requires` deciding what goes off together.
3. **Modules from other repositories**, still compiled in. Notes can live in its own repository here.
4. **The extension host** (C) with the narrow API, and a first extension written by the maintainer to prove it, such as issue-tracker links.
5. **A catalog:** signing, updates, revocation.

Phases 1–3 pay for themselves in a smaller `lib.rs`, `App.tsx`, and `commands.rs`, even if 4 and 5 never come. Each step of phase 1 pays for itself without the next.

## Not in this design

- Themes and UI customization.
- Extensions that replace core behavior (Git inspection, storage, the shell).
- Extensions that write to repositories, run Git, or read files outside what the user chose.
- Paid extensions and revenue sharing.

## Open questions

- Is notes and tasks one module ("Knowledge") or two? Tasks link to notes, and Today lists tasks; as two modules they would meet only through links and events.
- Who owns Today? It gathers tasks, pull requests, and activity, so it may be a core view that modules contribute sections to.
- In what order do several modules' decorations appear on one item, and who decides when they conflict (two panels beside one patch)?
- What is the general shape of a run owned by another module (prompt, file read back, listing, cleanup), and does anything other than explain want one before it is worth generalizing?
- Is a repository's code-sharing answer (`sharing.rs`) a core concern like credentials, or part of agent runs? Two modules depend on it today and a third provider of any kind would too, which argues for the core.
- Is a capability (renders as Markdown, has a commit range, has a patch) a Rust trait each item type implements, or a registry entry, and how does the frontend learn which items have which?
- Are repositories and the Git viewer core, or is the viewer (Changes, History, Branches) a module over core repositories?
- WebAssembly (C) or separate processes (B) for extensions: is the sandbox worth the narrower set of languages and the runtime's size?
- How extension UIs keep the app's look: shared design tokens and a small component kit for sandboxed frames.
- Who runs the catalog, and whether a curated list in a GitHub repository is enough at first.
- The project's license (not confirmed yet) decides what extension authors may do with the API and what the catalog can accept.
