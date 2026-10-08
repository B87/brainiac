# Brainiac — Specification

**Updated:** 8 October 2026  
**Target:** macOS desktop application, Rust backend, Tauri v2 shell

This document says what Brainiac does: the product, its release plan, and the behavior of the current release. How it is built is in [`docs/architecture.md`](docs/architecture.md); milestones and the designs of later releases are in [`docs/roadmap.md`](docs/roadmap.md).

- A behavior change is written here first, or in the same commit as the code.
- When work on a release starts, its design moves from `docs/roadmap.md` into this document.

## 1. Product vision

Brainiac is a personal second brain and daily work manager for a programmer. It connects durable Markdown knowledge, imported outside content, actionable tasks, and the developer's local repositories in one keyboard-oriented desktop application. Local semantic search and grounded AI answers extend that foundation in later releases.

The central workflow is **capture → organize → act → retrieve**:

1. Capture a thought, task, email, issue, article, or video reference while working.
2. Give it context by linking notes, tasks, and repositories.
3. Pick the day's work and open the relevant context.
4. Recover past decisions through keyword search, then optional semantic search.

The first user is the developer building the app. Initial scope is one person, one Mac, one or more local repositories, and no required account. A note vault is introduced in v0.2. Personal and company material can be organized in separate folders and workspaces; multiple independent vaults are a later extension.

**First-release workflow: open repositories → inspect changes and history → track workspace state → open the relevant code in an editor.** The broader capture/knowledge workflow is introduced in subsequent releases.

### Product principles

- Notes remain useful outside Brainiac as ordinary Markdown files.
- Core workflows work offline; local AI is optional.
- Capture, navigation, task completion, and search work from the keyboard.
- Editing reliability and responsive daily use take priority over graph visualization and automation.
- Background work is bounded, observable, and cancellable.
- Integrations enrich context without becoming prerequisites for using the app.

### Long-term scope

The product can eventually include PR and CI status, calendar context, recurring tasks, and explicit development actions. It does not need to replace an IDE, Git client, calendar, or general-purpose project management platform in its first release.

## 2. Release boundaries

**v0.1 — Git viewer and single/multi-repository tracker** shipped as 0.1.3. **v0.2 — knowledge, tasks, and code context** shipped as 0.2.0: one Markdown vault, tasks, Today, keyword search, and links between notes, tasks, and the repositories v0.1 tracks (sections 5–8). **v0.2.x — agent access** lets agents such as Claude Code work with Brainiac's notes, tasks, and repository links through a local MCP server (section 9). **v0.3 — pull requests** shipped as 0.3.1: reviewing and merging the pull requests of a workspace's repositories on GitHub and Bitbucket Cloud (section 10). **v0.4 — databases** shipped as 0.4.0: SQLite and PostgreSQL connections, a query editor and result grid, saved queries, and a PostgreSQL server's health, next to the repositories they belong to (section 11), with phase 1 of secrets (section 12). **v0.5 — agent runs** shipped as 0.5.0: Claude Code or OpenCode (with Anthropic, OpenAI, or OpenRouter models) run in a container on this Mac's Docker engine or an approved Linux host, followed live, and reviewed before anything leaves the Mac, ending with a patch or a branch to fetch (section 13); pushing a branch, more agents, and runs from tasks follow within v0.5. **v0.6 — explaining changes** is the current release: a commit, a branch, or a run's result explained by an agent so the reader learns the software being built (section 14). Content imports, global capture, and AI follow.

| Capability | Release | Scope |
| --- | --- | --- |
| Add/open one local repository | v0.1 | Folder picker, recent and pinned repositories |
| Named multi-repository workspaces | v0.1 | Manual repository selection and discovery of repositories inside a chosen folder |
| Repository overview | v0.1 | Branch, dirty/conflict state, file counts, local upstream comparison, stale/error state |
| Changes and diff viewer | v0.1 | Staged/unstaged changes, untracked preview, unified text diffs |
| Commit history and details | v0.1 | Paginated history, commit metadata, changed files, per-file patches |
| Branches and tags | v0.1 | Read-only lists and history selection, comparison with the default branch |
| Fetching | v0.1 | Explicit Fetch now and opt-in, per-workspace auto-fetch of watched branches (off by default); updates remote-tracking refs only |
| Workspace activity | v0.1 | Feed of watched branches and tags that moved, unread state, conflict-risk and drift warnings, team pulse, optional macOS notifications |
| Tracking and refresh | v0.1 | Watchers, bounded jobs, manual refresh, wake/activation reconciliation |
| Command palette | v0.1 | Repository/workspace switching and viewer commands |
| Notes, task hub, Today | v0.2 | One Markdown vault, safe editing, dates, context associations |
| Keyword knowledge search | v0.2 | FTS5 across saved notes and tasks |
| Backlinks and note/repository associations | v0.2 | Connect knowledge to the existing Git workspace |
| Agent access | v0.2.x | Local MCP server for agents such as Claude Code: search, notes, tasks, repository links; off by default |
| Pull requests | v0.3 | GitHub and Bitbucket Cloud, per workspace and off by default: the workspace's and each repository's pull requests, overview, files changed, review, and merge |
| Pull request follow-ups | v0.3.x | A daily view of what waits on you, a task's pull request, creating a pull request, agent tools; chosen by use |
| Databases | v0.4 | SQLite and PostgreSQL connections, read only by default; query editor and result grid, saved queries, history, and a PostgreSQL connection's health |
| Secrets | v0.4.x | An account's token or a connection's password read from the Keychain, an environment variable, or a command such as `gh auth token` or `op read`, and Settings → Secrets (section 12) |
| Database and credential follow-ups | v0.4.x | SSH tunnels, editing rows in the grid, saved queries as files, and secrets from Google Secret Manager, Git's credential helper, and `.pgpass`; chosen by use |
| Agent runs | v0.5 | Claude Code or OpenCode run in a container on this Mac's Docker engine or an approved Linux host, with guided setup, a live conversation, follow-up prompts, and a review of the collected work saved as a patch (section 13) |
| Agent run follow-ups | v0.5 | Pushing the reviewed result to a new branch, Codex and Gemini CLI, and runs from tasks (`docs/design/agent-runs.md`) |
| Explaining changes | v0.6 | A commit, a branch, or a run's result explained with a guided tour, notes beside the lines, concepts, and questions, each claim citing its source; written by an agent run in a container and checked by Brainiac (section 14) |
| Explanation follow-ups | v0.6.x | Working-tree changes, pull requests, a question about one note, and a direct model call for small changes and local models |
| External content imports | v0.7 | Paste, bookmarks, Markdown copies, articles, `.eml`, provenance and duplicate handling |
| Global capture window and Inbox | v0.7 | System shortcut, floating capture, Inbox triage of captured and imported items, shared backend state |
| Authenticated import adapters | v0.7.x | Selected Jira issues/mail messages; provider choice and video transcript acquisition validated separately |
| Semantic search | v0.8 | Optional local embeddings and hybrid retrieval |
| Grounded AI answers | v0.9 | Citation-backed local RAG |
| Other pull request providers, CI beyond pull request checks, other Git mutations, sync, plugins | Later | Separate features after the v0.3 providers are useful |

v0.1 requires a usable local Git binary. Detect it on startup and provide a clear setup message when absent; do not silently install developer tools. Core Git viewing works offline and requires no Markdown vault, Ollama instance, remote-service account, or elevated macOS permissions. Ahead/behind information reflects existing local refs and may be stale relative to the remote server until someone fetches: the user, their editor, or Brainiac's Fetch now and opt-in auto-fetch (section 4, Fetching). Fetching is the only operation that writes to a repository, and it touches remote-tracking refs and objects only. From v0.3, pull requests are reviewed and merged on GitHub or Bitbucket, never in the local repository (section 10).

Brainiac keeps its own data in a local database and snapshots it before each upgrade of its format and once a day, keeping seven. It refuses to open data saved by a newer version of Brainiac, or a file that is not Brainiac's, and says where the snapshots are; it never runs on data it cannot read correctly.

## 3. User experience

### Main window — v0.1

Use a quiet macOS layout: system font, light/dark appearance, native menu bar, standard window controls, visible keyboard focus, and readable text without translucency. Start directly in the Git workspace; do not show empty Today, Tasks, or Notes views before v0.2.

```text
┌──────────────────┬───────────────────────────────────────────────────────────┐
│ ● ● ●            │ [repo ▾ ⌘K] Changes · History · Branches & tags   main → │
│                  │             origin/main  Fetch  ⟳  Finder  Open in editor   │
│ All repositories ├───────────────────────────────────────────────────────────┤
│ Workspaces       │ File / commit / ref list   │ Diff, commit details, or ref │
│   Work    7 new  │                            │ history preview              │
│     product ROOT │                            │                              │
│     services/    │                            │                              │
│       billing    │                            │                              │
│   Personal       │                            │                              │
│ Pinned / Recent  │                            │                              │
│ + Add …      ⌘O  │                            │                              │
├──────────────────┴───────────────────────────────────────────────────────────┤
│ Up to date / Stale / Error · path   J K  [ ]  N P  keys · Git version · Checked │
└──────────────────────────────────────────────────────────────────────────────┘
```

The sidebar switches scope and shares the window's title bar area with the traffic lights. The center shows either a dashboard (All repositories or one workspace) or a selected repository's viewer. A dashboard is a table of repositories with a side panel for the selected row: branch, upstream, last commit, a peek at its changed files, and Open repository, Open in editor, and Reveal in Finder. The repository viewer's header carries the repository switcher, the tabs, the current branch with its upstream comparison and when it was last fetched, Fetch now, Refresh, Reveal in Finder, and Open in editor; there is no separate inspector, so diffs keep the full width. A workspace dashboard has two tabs, Overview (the table) and Activity (section 4, Workspace activity); the sidebar shows a workspace's unread activity count next to its name. The status bar lists the keyboard shortcuts of the current view.

### Main window — v0.2

The sidebar gains a section level above the repository tree: **Today**, **Tasks**, and **Notes**, then All repositories, Workspaces, and Pinned as in v0.1. The section that holds the notes is called Notes; "brain" names the whole app. There is no Inbox section before v0.7 (section 6).

- Brainiac reopens the section that was open when it quit. The first launch after upgrading to v0.2 opens Workspaces as before, and the sidebar shows one **Set up your vault** row until a vault is chosen.
- Today, Tasks, and Notes are never shown empty: before a vault is chosen they offer the vault setup, and Today and Tasks work without a vault.
- Settings (`Cmd+,`) is a page of the main window, not a dialog: its own sidebar lists the sections in place of the app's sidebar, and **Back** returns to the view it was opened from. Brainiac never reopens Settings at launch; it reopens that view. The sections are:
  - **General**: the editor Open in Editor runs, chosen from presets. **VS Code** and **Cursor** open a file at its line, through the command-line tool inside the app in Applications. **Warp** opens the folder or file in a new tab of Warp (with its `warp://` link) and **ChatGPT** in the ChatGPT app (with macOS's `open`), both at the top of the file rather than at the line. **Custom** shows the program and its arguments for a repository and for a file at a line, separated by spaces, with `{path}`, `{path_url}` (the path encoded for use in a link), and `{line}` filled in. Settings that match no preset show as Custom. The program is run directly, never through a shell.
  - **Notes and Search**: the vault (**Reveal in Finder**, **Choose Folder…**, **Create a New Vault…**), the note-ID setting, and **Rebuild Index**.
  - **Repositories**: how often status is refreshed (every 10 seconds to once a day), how often auto-fetch runs for the workspaces that turned it on (every 5 minutes to once a week), how long a fetch may take (10 seconds to an hour), and the largest diff shown (1 to 1,024 MiB and 1,000 to 10,000,000 lines). A value is checked and saved when its field is left, or when Settings is left.
  - **Accounts** (from v0.3, section 10) and **Agent Access** (from v0.2.x, section 9).
  - **Backup**: **Export…** and **Restore from Export…**, which the File menu also has.

  Changes apply as they are made; there is no Done button. A link elsewhere, such as a Pull requests tab's **Add Account…**, opens Settings on its section.
- Repositories, notes, and tasks each have one icon, used in the sidebar, ⌘K, chips, and side panels. A repository is always shown the same way wherever it appears: name, branch, a status dot with its words (clean, *N* changed, conflicted, missing), ahead and behind when not zero, and how fresh the data is ("checked 1 min ago", or "fetched 2 h ago" in amber when stale).
- Edit times read as relative for the last seven days ("edited 2 hours ago") and as dates after that ("edited 14 Sep"). Due and planned dates are always dates ("Due Fri 3 Oct"); an overdue task says "Overdue" in words and with an icon, not by color alone.

### Git navigation and interactions — v0.1

| View | Behavior |
| --- | --- |
| All repositories | Registered repositories, deduplicated across workspaces, filtered by name/path, dirty, conflicted, or stale/error state |
| Workspace | Overview: its member repositories and aggregate counts; click a row to open the repository viewer. Activity: what moved on the watched branches |
| Changes | Staged, unstaged, untracked, and conflicted entries with line counts; selecting a tracked file opens its applicable diff |
| History | Paginated commit list for HEAD or a selected ref, with author on every row; selecting a commit opens metadata, changed files, and patches |
| Branches and tags | Read-only local/remote-tracking refs and tags, sorted by recent activity or name, compared with the default branch; select a ref to view its history without checking it out |
| Pinned / Recent | Fast access to repositories and workspaces |

- **Onboarding:** add a repository, create a workspace by picking repositories or by discovering the repositories inside a chosen folder, and choose which discovered repositories to track. Preview resolution errors and let valid entries proceed.
- **Single repo:** open a registered repository directly without creating a workspace first.
- **Multi repo:** see which repositories have changes or conflicts, filter the overview, and drill into one while retaining the selected workspace.
- **Inspect changes:** select staged/unstaged files and inspect added/deleted lines. A file changed in both places has distinct index and working-tree comparisons.
- **Inspect history:** select a commit, read its message, inspect its files/patches, and copy its hash. Preserve repository identity on every history result.
- **Open code:** open a repository or selected current file in the configured editor; reveal its folder in Finder.
- **Refresh:** update the selected repository or workspace without losing navigation/selection unnecessarily. While new data loads, keep showing the previous content with a thin progress line; skeleton rows appear only on a first load.
- **Fetch:** bring remote-tracking refs up to date with Fetch now, or let a workspace auto-fetch its watched branches (section 4, Fetching).
- **Keep up with the team:** see what was merged or released on the watched branches of a workspace since the last look (section 4, Workspace activity).
- **Accessible status:** state is never conveyed by color alone (a conflicted repository's dot carries "!", a missing one is a dashed ring, labels spell out counts), and text meets WCAG AA contrast (4.5:1) in both appearances.

Repository name/path filtering and commit-message/hash filtering belong to the Git viewer. They do not depend on the future FTS5 knowledge index. History filtering searches Git history lazily; label the selected ref scope and do not imply results span other repositories.

### Keyboard defaults

| Shortcut | Action |
| --- | --- |
| `Cmd+K` | Palette: repositories, workspaces, notes, and tasks (notes and tasks from v0.2) |
| `Cmd+O` | Add/open a local repository |
| `Cmd+R` | Refresh selected repository or workspace |
| `Cmd+1` … `Cmd+5` | Repository tabs: Changes, History, Branches & tags, Notes (v0.2), Pull requests (v0.3) |
| `Cmd+1` … `Cmd+3` | Workspace tabs: Overview, Activity, Pull requests (v0.3) |
| `Cmd+N`, v0.2 | New note; in a repository's Notes tab, a new note linked to it |
| `Cmd+Shift+N`, v0.2 | New task |
| `Cmd+S`, v0.2 | Save the note now |
| `Cmd+Shift+E`, v0.2 | Switch the note editor between Live Preview and Source |
| `Cmd+B` | Show or hide the sidebar |
| `Option+Cmd+B` | Show or hide the side panel |
| `Option+Cmd+0`, v0.2 | Show or hide the context panel in Notes (the same panel as `Option+Cmd+B`) |
| `Option+Cmd+N`, v0.5 | New run |
| `Space`, v0.2 | Mark the selected task done or not done |
| `J` / `K` (or arrow keys) | Next / previous row in the focused list, without clicking it first |
| `[` / `]` | Previous / next file in a commit or the changes list |
| `N` / `P` | Next / previous hunk in the shown diff |
| `E`, v0.6 | Explain the commit, branch, or run result shown |
| `/` | Focus the filter of the current list |
| `Cmd+Enter`, v0.4 | In a query tab: run the statement under the cursor, or the selection |
| `Shift+Cmd+Enter`, v0.4 | Run every statement in the query tab |
| `Cmd+.`, v0.4 | Cancel the running statement |
| `Cmd+E`, v0.4 | Explain the statement under the cursor |
| `Cmd+T`, v0.4 | New query tab |
| `Cmd+1` … `Cmd+9`, v0.4 | In Databases: Home, then the query tabs |
| `Cmd+S`, v0.4 | In a query tab: save the query |
| `Cmd+,` | Settings |
| `Escape` | Dismiss the palette, a dialog, or a menu |

`Cmd+B` shows or hides the sidebar; hidden, it takes no room at all, and View › Show or Hide Sidebar brings it back too. `Option+Cmd+B` shows or hides the side panel on the right. Brainiac remembers both. That panel is the selected repository's preview, a branch or tag's history, the repositories in today's work, activity and pull-request settings, a pull request's merge checklist, and a note's context panel.

Avoid overriding standard text-editing shortcuts. Essential actions have a menu or visible control. Error messages describe the failed action and offer a concrete recovery action.

## 4. Workspaces and repositories — v0.1

### Workspace model

A workspace is a named list of repositories that the user wants to see together. Brainiac imposes no folder layout: members can live anywhere on disk, and one repository can belong to several workspaces. There are two ways to build one, and both produce the same kind of workspace:

- **Manual:** pick repositories one by one. The workspace is a flat list.
- **Discovered:** pick a folder and let Brainiac find the repositories directly inside it. The chosen folder can itself be a Git repository, in which case it becomes the workspace's **root**, or a plain folder.

Discovery supports layouts such as these without requiring any of them:

```text
code/                           Plain folder; no root repository
  api/                          Independent Git repository
  web/                          Independent Git repository

product/                        Git repository; becomes the workspace root
  services/                     Discovery folder, chosen by the user
    billing/                    Independent Git repository
    search/                     Independent Git repository
```

When a root exists, it has its own status, history, branches, and diffs, and each member repository has independent Git state. Physical nesting does not imply a monorepo, a submodule relationship, coordinated branches, or atomic changes across repositories. Display actual Git relationships only when detected.

#### Discovery and membership

- **Add workspace from folder** selects a folder and a discovery folder relative to it. The discovery folder defaults to the selected folder itself; the user can point it at any subfolder. The preview lists the root (if the selected folder is a repository) and the discovered repositories, and the user chooses which to track.
- Only the discovery folder's immediate children are candidates, and a child counts as a repository only when it is the top of its own working tree: a plain folder inside the root's working tree is not another repository. Linked worktrees and actual submodules are recognised as what they are; not every child is assumed to be a submodule. Non-Git children are skipped with an explanation in the preview.
- Stay within the discovery folder. Symlinked external repositories require explicit selection. Deeper descendants require explicit addition; do not crawl arbitrary nested dependency trees.
- A repository is listed once however it was found, while distinct linked worktrees stay separate. Separate repositories are never combined because their current branch names match. A repository can belong to several workspaces.
- A missing discovery folder leaves the root and any manually added members usable and shows the discovery issue.
- For discovered workspaces, watch the discovery folder for added/removed child folders and expose **Rescan**. Surface additions for the user to track; never add them silently. Mark removed or inaccessible registered members missing; preserve registrations and future context links until explicitly removed or relocated.
- Rescan runs when a discovered workspace opens and on **Rescan**. When a missing member and exactly one untracked repository in the discovery folder are the same repository (Relocating a repository, below), Rescan suggests the move (**Update**) instead of listing that folder as new. A folder that matches several missing members, or a member that matches several folders, gets no suggestion and is listed as new. How many it looks for is bounded (`docs/architecture.md`, Workspaces and discovery).
- Brainiac owns its tracked-repository selection independently of editor configuration. Any workspace can gain manually added repositories or drop members, whichever way it was created.

#### Relocating a repository

A registration keeps its identity when its folder moves. **Locate…** points it at the new folder and keeps its ID, workspace memberships, pin, place in the recent list, last tab, and activity feed with its read state, all of which removing and adding the folder again would lose.

- **Locate…** appears wherever a missing repository is shown: its overview row, the selected-repository panel, and its viewer. The viewer's More menu and the command palette offer it for the open repository even when its folder exists, for example to switch to a fresh clone. The folder picker opens in the old folder's parent.
- The chosen folder must be inside a Git working tree; a folder below the top of a working tree stands for that working tree.
- **Same repository** means the chosen working tree contains a commit Brainiac recorded for the registration: the last observed `HEAD`, or a recorded tip of a watched remote branch or tag. A fresh clone qualifies through the remote tips. The registration then moves without a question. The check never downloads anything.
- Otherwise Brainiac asks first and says why: the chosen folder is below the top of the working tree, the histories share no recorded commit, or nothing was recorded to compare with. Confirming a repository whose history is unrelated or unverified starts its activity over, silently.
- A folder already registered as another repository is refused with `CONFLICT`; remove one of the two registrations first. Merging two registrations is not supported.
- What moves along:
  - Every membership points at the new folder, and the member's name becomes the new folder name. A workspace's root repository that ends up outside the workspace's selected folder stops being the root and stays an ordinary member.
  - When a parent folder was renamed or moved (relocating `code/web` to `src/web` while `code` is gone means `code` became `src`), the other missing repositories, missing members, and discovery folders inside it move to the same place under the new folder when they are there; a repository moves only when it is the same repository. Nothing else moves when the old folder still exists, as when switching to a second clone.
  - A moved main checkout's registered linked worktrees are pointed at the new location. They work again once `git worktree repair` has run; Brainiac does not run it.
  - The activity feed and its read state follow the registration when the history is the same.
- Afterwards the moved registrations are refreshed and watched at their new folders. Branches that moved while a repository was missing arrive as ordinary activity events.
- Relocating writes nothing to any repository.

The exact rules (which folder counts as moved, discovered or manual after a move, activity, and concurrent changes) are in `docs/architecture.md`, Relocation.

#### Presentation and Git boundaries

In the sidebar and the overview, a workspace with a root shows the root first, followed by its other members; discovered members are grouped under the discovery folder's name. A manual workspace shows a flat list. The overview has no layout-specific scope switch: users narrow it with the name/path and state filters and by opening one repository. Aggregate counts retain per-repository attribution; histories and diffs are never implicitly merged into one Git history.

Root-repository status is exactly what Git reports for the root. Member status is collected independently, including when the root ignores the discovery folder. If the root tracks a member as a gitlink/submodule or reports a nested directory as untracked, display that parent entry as its own observation; it is not a substitute for the member's detailed status. Do not add member file counts into the root's own dirty-file count.

A discovered workspace with a root renders in the sidebar like this; a manual one omits the root and the group:

```text
Product
  product            root
  services/
    billing
    search
```

### Relationship to editor workspace files

Editor workspace files such as VS Code's `.code-workspace` often describe a similar grouping: a list of folders, some commented out, alongside editor-only settings such as `files.exclude` and task definitions. Brainiac does not read, import, watch, rewrite, or synchronize them. The same grouping is recreated by discovery from a folder, by manual selection, or both, and kept in Brainiac's own configuration. An editor's active or commented folder entries, its display settings (hidden folders or `.git`), and its task or launch definitions change nothing in Brainiac, and tasks are never executed. A `.code-workspace` importer may come later, without ongoing synchronization.

### Single-repository viewer

A registered repository opens directly; workspace membership is optional. Keep tabs for Changes, History, and Branches/Tags, and remember the last tab per repository. A fresh repository with no commits shows an empty history while still showing staged/untracked files. Missing/inaccessible paths show an error with **Locate…** (Relocating a repository) or Remove registration; removing registration does not touch the working directory.

#### Changes and diffs

- Group staged, unstaged, untracked, and conflicted files. Show paths, change kinds, rename source/destination, and added/removed line counts with a small change bar. Groups fold; long folder names are shortened in the middle so the file name always shows.
- For tracked text, display a unified diff with line numbers, additions/deletions, and context. Separate HEAD-to-index and index-to-working-tree comparisons; the same file can appear in both groups. The comparison switch shows each side's counts and offers **Both** (HEAD to working tree, one combined patch).
- Every patch view offers Unified/Split layouts, changed-word highlights inside a modified line pair, **Ignore whitespace**, hunk position with previous/next hunk jumps, and a hunk header that stays pinned while scrolling.
- Untracked lists each file, including every file inside a new folder; a folder that holds its own repository is one entry. A group draws its first 1,000 entries and offers **Show more** for the rest.
- Untracked files use a bounded, read-only text preview labeled untracked. Conflicted files show conflict status and current contents; conflict resolution is later scope.
- Binary files, submodule changes, Git LFS pointers, symlinks, and oversized patches receive explicit summaries rather than misleading text diffs. Do not download LFS objects or traverse submodules automatically.
- Display limits: 1 MiB or 10,000 patch lines per file by default, whichever comes first, changed in Settings → Repositories. Mark truncation and offer Open in editor; never silently omit remaining content.
- A displayed working-tree diff updates when the repository changes; patches are read-only.

#### History and commit details

- Default history is the current HEAD's reachable commits, newest/topologically ordered. It loads 100 commits at a time, and new commits do not shift a list being scrolled.
- Show commit hash, subject, author, authored/committed time, parent IDs, and branch/tag decorations. Display full message in commit details.
- Offer ref selection and commit-message/hash filtering; an `author:` token in the filter limits by author. Queries remain scoped to the selected repository/ref; indicate loading and cancellation.
- Each row shows the author's initials and name; the day heading stays pinned while scrolling. The commit's file list can be hidden to give the patch the full width, shows a +/− bar per file, and a file stepper ("1 / 7", `[` and `]`) moves through files.
- Selecting a commit loads its changed-file list; selecting a file loads its patch. Compare a normal commit with its parent, a root commit with an empty tree, and a merge commit with its first parent by default. Label the chosen parent and permit selecting another parent.
- Copy commit hash and relative file path. Display removed files and rename history correctly; opening a historical path is separate from opening its current working-tree file.
- A graphical branch-lane visualization and blame are future additions. A useful history list and parent links are sufficient for v0.1.

#### Branches and tags

List local branches, remote-tracking branches, and tags, marking the current branch and upstream where present. Selecting a ref changes the history view without checking out the branch.

- Sort by recent activity (default) or name. Branches whose tip is older than three months fold into their own group; remote-tracking branches fold per remote; tags sort newest version first. Long names keep their start and end, shortened in the middle.
- Compare every branch with the repository's default branch: the remote's `HEAD` target (such as `origin/main`), else a local `main` or `master`. Show "ahead/behind main" next to the upstream comparison.
- The selected ref's side panel separates "commits not on main" from the shared history.

### Multi-repository dashboard

Show repository name/path, branch or detached HEAD, staged/unstaged/untracked/conflicted file counts, last refresh, and error/stale status. Count unique changed paths separately from staged/unstaged groups so a doubly modified file is not counted as two files. Provide workspace totals, filters for dirty/conflicted/stale repositories, sorting by name or latest observed commit time, and Refresh all. One repository failure must not block the others. Deduplicate the All repositories view while allowing one repository to belong to several workspaces. Ahead/behind appears only when an upstream exists and reflects local refs as of the last fetch, which the dashboard shows per repository.

### Refresh strategy

Brainiac notices changes to unstaged working files as well as to Git's own state (commits, branch switches, staging), with no manual refresh (`docs/architecture.md`, Refresh).

- A burst of changes causes one refresh after a short pause, never one per file saved or keystroke.
- Everything is reconciled on launch, wake, window activation, and manual refresh.
- A slow safety refresh, proposed every 60 seconds while the dashboard is visible, catches missed changes. Expensive repositories back off, and data that may be out of date is labelled stale.
- If status takes longer than a proposed 5 seconds, the last snapshot stays on screen with a warning and a retry option.

v0.1 actions are inspect status/diffs/history/refs, copy hashes/paths, refresh, fetch, reveal in Finder, and open in editor. Knowledge/task associations arrive in v0.2. Fetching (below) is the only operation that writes to a repository, and automatic fetching happens only for workspaces that opted in. Branch checkout, commit, stash, pull, and automatic dev-server startup remain future, explicit actions.

### Fetching

Fetching updates remote-tracking refs (`refs/remotes/...`), the objects they need, and tags that point into the fetched history. It never touches the working tree, the index, local branches, `HEAD`, or the stash, so it cannot change anything the user is working on. It is still a write to `.git`, so it happens in exactly two cases:

- **Fetch now:** an explicit action on a repository (header button, menu) or on a workspace (every member). It fetches the remote of the current branch's upstream, else `origin`, else the only remote, with the remote's configured refspecs, leaving out any that could write a local branch or overwrite the user's own tags.
- **Auto-fetch:** a per-workspace setting, **off by default**. While Brainiac runs, each repository of an auto-fetching workspace fetches only that workspace's watched branch patterns from `origin`, else the only remote, else the current branch's upstream remote, so a branch that tracks a fork does not change what is watched. It runs at most every 15 minutes (a global setting) and backs off exponentially after failures up to 6 hours.

Both use the same hardened invocation with explicit, checked refspecs, so a fetch never prunes, runs hooks, recurses into submodules, or writes outside remote-tracking refs and tags, whatever the user's configuration says (`docs/architecture.md`, Fetch invocation).

- **Credentials:** a background process cannot answer prompts, so a fetch never asks for a password or passphrase. Credential helpers such as the macOS keychain keep working. Authentication failures are reported as "needs sign-in" with the recovery step (fetch once from a terminal or editor), never as a prompt. Hardware keys and password-manager SSH agents may still ask for approval on each fetch; the auto-fetch setting says so.
- **Deleted branches:** a watched branch the remote no longer has does not make the fetch fail, and it costs no extra connections.
- **Other Git processes:** while another Git process is changing what a fetch writes, the fetch is skipped as busy, records nothing, and auto-fetch tries again later; a commit in progress does not block. A lock file left behind by another process, which also blocks the user's own Git commands, is reported as an error that names the file.
- **Concurrency:** fetches run a few at a time, and a repository never has two in flight: a second request (such as Fetch all over a checkout and its linked worktree) shares the first one's result.
- **Freshness:** a repository's "last fetched" time is the later of Brainiac's own last fetch and the modification time of `FETCH_HEAD` (written by the user's own fetches). The activity feed warns when a watched repository has not been fetched for two days.
- After a fetch every checkout of the Git directory is refreshed as after a watcher event, which updates ahead/behind and the activity feed. A fetch that had nothing to fetch reports no fetch time.

### Workspace activity

The Activity tab of a workspace answers "what did the team merge or release since I last looked?". It reads local refs only; news arrives when a fetch (the user's, their editor's, or Brainiac's) moves remote-tracking refs.

- **Watched refs** are per workspace: branch names or patterns matched against remote-tracking branches without their remote prefix (`main`, `develop`, `release/*`), and tag patterns (`v*`). `*` matches any run of characters, at most once in a branch pattern; patterns must be valid ref names. Defaults: branches `main`, `master`, `develop`; tags `v*`.
- A checkout and its linked worktrees share one feed (`docs/architecture.md`, Activity tracking).
- Refs that start being watched (the first observation, a new pattern, a second workspace, a newly added or renamed remote) join silently; only their later moves are news. Branches created after a pattern changes or a repository joins a workspace still arrive as news. Each moved ref becomes one event, at most twenty at a time, newest first by tag or commit date:
  - **advanced:** the old tip is an ancestor of the new one; record the commit and merge counts, up to five newest commits, and the authors.
  - **rewritten:** history was replaced (force-push), or the old tip no longer exists; record how many commits were replaced and added. Shown in red.
  - **created:** a watched branch appeared.
  - **tagged:** a new tag matching a tag pattern, with the number of commits since the previous tag.
- Details are best effort: when they cannot be read, the event is recorded without them, so one bad ref cannot stop the feed.
- **Conflict risk:** paths changed by an advanced range that the user also changes in the working tree are listed on the event.
- **Drift:** when the current branch is behind its upstream after the move, or behind the watched branch it was forked from, the event says by how much.
- **Each workspace sees what it watches:** the feed, the unread count, Mark all as seen, and notifications use the workspace's own patterns, and only events observed after the workspace started watching the matching pattern, even when another workspace watching the same repository has more or older ones. Seen state belongs to the event, so an event both workspaces show is seen in both.
- **Unread:** the tab shows a divider between unread and seen events, Mark seen per event and Mark all as seen, and the sidebar shows the workspace's unread count.
- **Team pulse:** commits, merges, and releases on the watched refs in the last seven days, and the most active authors, counted from local refs. It loads separately, so the feed never waits for it.
- **Let me know:** optional per workspace, all off except the conflict-risk warning: a macOS notification when a watched branch moves (at most one per repository and workspace per hour), a morning digest at 09:00 local time when there are unread events, and the conflict-risk warning on events.
- Events older than 90 days are pruned. Removing the last checkout of a repository deletes its feed; a repository no workspace watches forgets what it saw, so watching it again starts silently.

## 5. Notes — v0.2

### The vault

- One vault: a folder of Markdown notes the user chooses (**Choose Folder…**) or creates (**Create a New Vault…**). Brainiac edits the `.md` files where they are; nothing is moved, converted, or imported, and the notes stay ordinary files usable in any editor.
- Supported notes are UTF-8 `.md` files on a local filesystem, up to 5 MiB for editing. Other files are listed with a clear message and **Open Externally**; a `.md` file over 5 MiB or not in UTF-8 is found in search by its name, not its text.
- A vault may itself be a Git repository, and it can be registered and tracked like any other. The rule that Brainiac never writes to a repository covers Git's own state: it never stages, commits, checks out, or runs hooks there. Saving a note is an edit the user makes through Brainiac, like saving it in any editor. Trash, drafts, and revision history live in Brainiac's data folder, never in the vault, so saving never leaves extra files behind.
- Brainiac scans the vault at startup, on wake, and when the watcher reports changes, and reconciles what it finds. A vault that cannot be read (an unmounted disk) is reported as unavailable, never treated as every note deleted.
- Choosing another vault folder later keeps the notes of the previous one as missing, with their tasks and links; choosing that folder again brings them back.

### Notes view

- A vault tree with folders, pinned and recent notes, and a filter (`/`); the editor; and a context panel. The context panel lists the note's linked repositories (each with its live state and Open / Open in Editor), its tasks, its backlinks, its unresolved links (with **Create**), and suggestions: a registered repository the note mentions can be linked with one click or dismissed. Brainiac never links anything by itself.
- The context panel can be hidden (`Option+Cmd+B`, or `Option+Cmd+0`); when hidden, the header shows how many repositories and tasks the note has. It hides by itself when the window is too narrow for the editor. That narrow-window choice is not the one `Option+Cmd+B` remembers for a wide window.
- Checkboxes in a note (`- [ ]`) are square and remain note text. They never become tasks and never appear in Today; tasks are round (section 6).

### Editing

A note is edited as its Markdown text, so a save contains exactly what the user typed and nothing Brainiac reformatted. A rich editor that converts Markdown to a document and back rewrote or lost content in testing (HTML, wikilinks, footnotes, nested code fences, list numbering, `snake_case` escaped); Brainiac draws its formatting over the text instead, and drawing never changes it.

- **Live Preview**, the default, shows the note formatted. Headings show at their size; bold, italic, strikethrough, inline code, quotes, and fenced code are styled; Markdown markup (`#`, `**`, `` ` ``, link targets) is hidden except on the line being edited, where it reappears so it can be changed. Bullets, checkboxes, and horizontal rules are drawn as such, and links show only their text.
- Images stored in the vault show below their line in Live Preview. A web image shows as its link, so opening a note makes no network request.
- **Source** shows the same text with every mark visible, dimmed. `Cmd+Shift+E` or the toggle in the note header switches between them, keeping the cursor and scroll position; the choice applies to every note and is remembered.
- Links and wikilinks open with `Cmd`+click; a link to another note opens it in Brainiac, a web link opens in the browser.
- Clicking a checkbox toggles `[ ]` and `[x]` in the text.
- Enter continues a list, task list, or quote on the next line and never renumbers the items below; Up and Down move one line at a time, including past a line drawn as an image.
- Copying copies the Markdown, including markup Live Preview hides, and undo steps through text changes in both modes.
- Frontmatter shows as a dimmed block at the top of the note and is edited as text, so unknown keys are preserved.
- Tables, HTML, and syntax Brainiac does not render (callouts, footnotes, math) show as plain text in both modes and are saved as written.
- Opening a note without editing never rewrites it, in either mode. A save changes only what was edited: line endings, a missing final newline, and the rest of the note stay as they were. A fixture suite checks this for headings, lists, checkboxes, tables, fenced code, links, images, frontmatter, HTML, unknown syntax, CRLF line endings, and a missing final newline.
- Notes save by themselves after 750 ms without typing; `Cmd+S` saves at once. The header shows Saving, Saved, Save failed, or Changed on disk accurately, and switching views keeps an unsaved draft.

### Note identity

- New notes get a UUID in the frontmatter key `brainiac_id`.
- An existing note without one gets an ID stored by Brainiac, without changing the file. Brainiac writes `brainiac_id` into the note the first time it gets a task or a repository link (a setting, on by default), because such a note carries context a rename must not lose. Adding it otherwise is an explicit action. Opening or indexing a note never writes it.
- The ID is a convention Brainiac cannot enforce: copying a note copies it, and an edit can remove it. Two notes with the same ID are shown as a conflict; they are never merged silently.
- A note is identified by its vault and its path within the vault, never by an absolute path, so the vault can move or be restored elsewhere.
- A note moved or renamed outside Brainiac is recognised by its `brainiac_id`, otherwise by the same path, otherwise by content identical to exactly one note that went missing in the same scan or the same burst of changes, such as a `git pull`. Anything ambiguous becomes a missing note and a new note, which the user can relink; identical content alone does not prove identity.
- The title is the frontmatter `title`, else the first heading, else the file name. Lists and the folder tree show titles; the file's path shows on hover and in the note's header.
- A note whose file name matches its title keeps them matching: when the title changes, the file is renamed after the title once the cursor leaves the title's line or the note, never while typing it. A number Brainiac added to tell notes apart (`Untitled 2.md`) still counts as matching, and a renamed note takes the next free number the same way. A note that other notes link to is not renamed by itself, because their links name the file; neither is a file whose name was already different from its title, such as `2026-10-02.md` titled Standup. While the two differ, the header offers **Rename File to Match Title…**, which opens Rename with the new name filled in and the links it would update listed.
- Arbitrary frontmatter keys, code fences, and relative links are preserved.

### Links between notes

- Brainiac reads both standard Markdown links to other notes and `[[wikilinks]]`, and writes standard Markdown links.
- Backlinks list every note that links to the open note. A link whose target does not exist is kept as unresolved and resolves when a note with that name appears.
- Renaming a note in Brainiac offers to update the links to it in other notes, listing the files it would change. The option is off by default because it edits other notes.

### Saving and changes from outside

- A save never overwrites a version of the note Brainiac has not seen. If the file changed on disk since it was opened, the save stops, the draft is kept, and the editor offers **Compare…**, **Reload from Disk**, and **Save Draft as Copy**.
- A note changed outside Brainiac (another editor, a `git pull` in the vault, an agent) reloads by itself when it has no unsaved edits. Its previous text is kept in revision history first, so any outside edit can be undone like Brainiac's own.
- If an open note's file disappears, the draft, its tasks, and its links are kept, and the editor offers **Restore as New File** and **Relink to a File…**.
- If a save succeeds but search could not be updated, the header says **Saved · search update pending**, and the update is retried in the background.

### Delete and recovery

- Deleting a note moves it to Brainiac's trash, in its data folder, and removes it from search. Its tasks and links are kept. Restoring it asks before overwriting a note at the same path. Brainiac never permanently deletes a note as the default action.
- A note deleted outside Brainiac is shown as missing; its tasks keep a reference to it, and its last text is kept in revision history so **Restore as New File** can bring it back.
- Revision history keeps up to 20 versions per note for 30 days, within 250 MiB in total. Autosaves while typing count as one version until 10 minutes pass; each change from outside Brainiac is its own version; unresolved conflicts and unsaved drafts are never pruned.

### Notes and repositories

- A note can link to any number of registered repositories, and only to repositories (not workspaces). Links are made explicitly: **Link…** in the context panel, accepting a suggestion, or creating the note from a repository.
- The repository viewer gains a **Notes** tab (`Cmd+4`): the notes linked to the repository, its open tasks, and suggested notes that mention it, with a read-only preview, **Open Note**, **Unlink**, and **New Note for** *repository*. With nothing linked it explains how to link or create one.
- Wherever a linked repository appears, it shows its live state from the status Brainiac already keeps; nothing extra runs in Git.
- Removing a repository keeps its links, shown as a removed repository with **Add Again** and **Unlink**. Adding a repository with the same remote later offers to reconnect them. A linked repository whose folder moved offers **Locate…** (section 4, Relocating a repository).

## 6. Tasks and Today — v0.2

- A task has a title, a short plain-text description, a status (to do, in progress, done, cancelled), an optional planned date, an optional deadline, and links to at most one note and one repository. Work spanning several repositories links a note that covers them. Longer material belongs in the linked note.
- Planning a task and giving it a deadline are separate actions. Dates are calendar days in the Mac's time zone, so a task due today stays due today when travelling or when the clocks change.
- Completing a task records when; reopening it clears that. A task is drawn with a round check, never a square checkbox.
- **To sort:** a new task without a planned date or deadline is *to sort* until it gets one or is marked **Sorted**. v0.2 has no Inbox: everything is created inside the app, and an inbox earns its place only when items arrive faster than they are sorted, which starts with v0.7's capture and imports. Quick notes go to an ordinary `Inbox/` folder in the vault.
- **Today** lists open tasks that are overdue, due today, or planned for today or an earlier day (unfinished work carries over), then those completed today. Tasks to sort appear as one folded **To sort · N** line above them; expanding it lists them. Each task shows its linked note and repository, the repository with its live state. A side panel lists the repositories in today's work with their state and **Fetch** and **Open**. "Today" follows the Mac's date, including across midnight and after waking.
- **Tasks** lists all tasks, filtered by status and by **To sort**.
- Two edits of the same task, from two places, never overwrite each other silently: the later one is refused with a conflict and shows the current task.

## 7. Search — v0.2

`Cmd+K` searches repositories, workspaces, notes, and tasks together, grouped by kind, with repositories first. Notes and tasks are searched by keyword; no AI model is needed.

- Searches note titles, note text including code blocks, and task titles and descriptions. Search becomes complete when the vault scan finishes; until then it says **Indexing notes: N of M. Results may be incomplete.**
- Input is literal text: it is never read as search syntax, and a malformed quote or stray symbol never shows an error. Each word also matches words that start with it, so `migrat` finds *migration* and `async run` finds *async runtime*; a quoted phrase matches its words in order. Case and accents are ignored, so `cafe` finds *cafè*.
- Code identifiers are found whole or by their parts: `fetch_with_backoff` and `backoff` both find `fetch_with_backoff`, and `tokio::spawn`, `src/main.rs`, and `v0.2` match as written.
- Title matches rank higher, and titles and paths also match any part of a word, so `Backoff` finds a note named `fetchWithBackoff.md`. In note text, a word inside a camelCase identifier or inside unspaced Chinese or Japanese text is found only from its start.
- Scope buttons filter to Repositories, Notes, or Tasks. Each group shows its first results and **Show all N**. Snippets highlight the matched words.
- The palette says **No matches** (offering a looser search when a quoted phrase found nothing), **Indexing incomplete**, or **Search unavailable** (with **Rebuild Index**; repository names still match) and never confuses them.
- Results update as the query changes; an older query's results never replace a newer one's.

## 8. Backup and restore — v0.2

- Brainiac snapshots its own data before each format upgrade and once a day, keeping seven (section 2). Revision history is snapshotted separately and less often. The search index is never backed up; it is rebuilt from the vault.
- **Export** writes the vault's notes, a consistent copy of Brainiac's data, a manifest, and tasks as JSON with their dates, statuses, and links. It goes into a new folder outside the vault, where it would otherwise be read as a second copy of every note. A note changed during the export is retried or reported; the export never claims to be complete when it is not.
- **Restore** checks that the files are Brainiac's and from a version it can read before replacing anything. The export's notes are copied into an empty folder, or an existing vault folder is used as it is. Brainiac then restarts into the restored data, snapshotting the data it replaces first. It matches notes by `brainiac_id`, then by path and content; matches repositories by their remote URL (stored and exported without a password or token in it), offering **Locate…** for the rest, and keeps the repositories already registered on this Mac; and rebuilds search.
- Snapshots are recovery aids on the same Mac. A complete backup is an export, or the vault plus Brainiac's data folder, kept on another device or backup system.

## 9. Agent access — v0.2.x

Agents such as Claude Code can already read and edit the vault's Markdown files, and Brainiac treats them like any other editor (section 5, Saving and changes from outside). Agent access lets them work with what lives only in Brainiac: tasks, Today, search, and links between notes and repositories. It is a local [MCP](https://modelcontextprotocol.io) server inside the running app.

- **Settings → Agent access** is **Off** (the default), **Read only**, or **Read and write**. Changing it applies at once to agents already connected: their tools appear, change, or disappear without reconnecting. While it is off, a connected agent sees no tools and is told where to turn it on. Settings shows how many agents are connected.
- Settings shows the command that adds Brainiac to Claude Code for every project, `claude mcp add --scope user brainiac -- <the app>/Contents/MacOS/brainiac mcp`, with the app's real location, and copies it; other MCP clients run the same program with the argument `mcp`. If agents cannot connect, Settings says why. Brainiac's Claude Code plugin adds the server and a skill in one step (below).
- An agent that starts while Brainiac is closed opens it in the background and waits a few seconds for it. Nothing listens on the network: only programs running under the same macOS account can connect.
- **Read only** lets an agent search notes and tasks; read a note with its version, tasks, and linked repositories; list a folder's notes, the recent notes, the tasks (filtered as in the Tasks view), and Today; list the registered repositories with their live state; find the registered repository that contains a folder, such as the one the agent is working in; and list a repository's notes and tasks.
- **Read and write** adds: create a note with its text; edit a note; create a task, change it, or complete it; and link a note to a repository or unlink it.
- An agent never deletes or trashes a note, deletes a task, renames or moves a note, exports or restores, changes settings, fetches, or does anything else in a repository, and it cannot open files or run commands through Brainiac.
- Agents follow the same rules as the app. An edit based on a version of a note or task that has changed since the agent read it is refused with a conflict and never overwrites; the agent reads it again. The app updates live, and a note open with unsaved edits gets the conflict handling of section 5.
- Every note an agent saves keeps its previous text in revision history, marked **Changed by an agent**, as its own version even when saves follow each other, so any agent edit can be undone like the app's own. A note an agent creates gets a `brainiac_id` like any new note.
- Notes may hold text from elsewhere, and later imported articles and emails. Brainiac tells agents to treat note and task text as the user's data, never as instructions.
- **Claude Code plugin:** `/plugin marketplace add B87/brainiac`, then `/plugin install brainiac@brainiac`, adds the server and a skill that teaches Claude Code how to use Brainiac: planning the day from Today, turning work in a repository into tasks linked to it, and writing up decisions as notes linked to the repositories they concern. The plugin expects the app in Applications; for an app elsewhere, use the command from Settings instead. Use one or the other, not both, or Claude Code connects twice.

## 10. Pull requests — v0.3

Review and merge the pull requests of a workspace's repositories, on GitHub and Bitbucket Cloud, without leaving Brainiac. Both providers look and behave the same; the provider is a small label, and what one of them cannot do is simply not offered. Brainiac's own advantage is the local checkout: diffs come from local Git when the commits are on the Mac, and a fetch that moves a pull request's branch refreshes it.

### Boundaries

- **Off by default, per workspace.** Nothing is requested from GitHub or Bitbucket for a workspace that has not turned pull requests on, and every other feature works without an account or a network.
- **Nothing is written to a local repository.** Brainiac never checks out, pushes, or deletes a branch for a pull request. Commenting, reviewing, resolving threads, and merging are writes to GitHub or Bitbucket only, each an explicit, visible action; merging asks for confirmation.
- **Providers:** GitHub (github.com) and Bitbucket Cloud (bitbucket.org). GitLab, Bitbucket Data Center, and GitHub Enterprise Server are later.
- Agents get no pull request tools in v0.3.

### Accounts

- **Settings → Accounts** lists one account per provider: its login, the kind of token, when it expires, where the token comes from, and what it allows, with **Change Token…** and **Remove**.
- GitHub takes a fine-grained personal access token with pull requests read and write, contents read, and checks and commit statuses read; merging and resolving threads also need contents read and write, which also lets the token push, so Accounts says so and leaves the choice to the user. Bitbucket Cloud takes an Atlassian API token, with the account's email, scoped to `read:user:bitbucket` (to know which pull requests are yours and wait on you), `read:repository:bitbucket`, `read:pullrequest:bitbucket`, and `write:pullrequest:bitbucket`; Bitbucket's app passwords no longer work.
- Adding an account checks the token with one request. For Bitbucket that request also returns the token's scopes, so a missing one is named at once, and a token that can read but not write is offered **Save as Read-Only**. GitHub shows a fine-grained token's expiry but not its permissions, so a missing permission is found the first time GitHub refuses an action: that action is then turned off and the message names the permission to add.
- An Atlassian API token is about 190 characters, longer than Terminal's hidden password prompt keeps (`security … -w` with nothing after it cuts the input short). Paste it into Accounts, or add it from Terminal from the clipboard with `security add-generic-password -U -s brainiac -a bitbucket -w "$(pbpaste)"`.
- A token is kept in the macOS Keychain, in the item with service `brainiac` and account `github` or `bitbucket`, or read from a command or an environment variable (**Token from**; section 12), and sent only to the service it belongs to, over HTTPS. Tokens never appear in logs, exports, or the database.
- **Test** on the account form checks the form's token with the same request, without saving anything.
- A token already in that item, such as one added with `security add-generic-password -s brainiac -a github -w`, is found when Accounts opens without reading it, and **Use This Token** checks it like a pasted one; Bitbucket still asks for the account's email. The first time Brainiac reads an item it did not create, macOS asks to allow it.
- A token is stored only once its check passes. **Remove** deletes the account and Brainiac's Keychain item; a command or a variable it read the token from is not changed.

### Which pull requests a repository has

- A repository's pull requests come from the hosting service its `origin` points to: `git@github.com:acme/api.git` and `https://bitbucket.org/acme-team/api` both name a repository there. A repository whose pull requests live elsewhere, such as the upstream of a fork, can be pointed at it with **Change…**; Brainiac keeps that choice when `origin` changes.
- A workspace turns pull requests on in its side panel; a repository on GitHub uses the GitHub account and one on Bitbucket the Bitbucket account. A repository belongs to the workspaces it is in; its pull requests are tracked when any of them has pull requests on.

### Workspace → Pull requests

The third tab of a workspace (`Cmd+3`, after Overview and Activity).

- Every open pull request of the workspace's repositories, grouped by repository, in a table with column headings: title and number, author, reviewers, checks, unresolved threads, size, and age. Reviewers show their state (requested, approved, changes requested, commented) with an icon as well as color. Rows waiting on your review are marked.
- Filters: all open, needs your review, yours, others, drafts. Sorted by what has waited longest.
- A side panel like Activity's holds the workspace's switch, its accounts, and how much of the hour's request allowance each has used.
- **States:** a first load shows skeleton rows; a workspace with pull requests off explains what turning them on does and offers it; a workspace without an account for one of its providers offers to add one; when a provider is unreachable or out of requests, the tab keeps showing what it had, says how old it is and when Brainiac retries; a repository that fails (removed, no access) shows its error in its group while the others keep working.
- The sidebar shows each workspace's count of pull requests waiting on your review. Opening the workspace from the sidebar shows this tab while there are some, unless unread activity opens Activity first.

### Repository → Pull requests

The fifth tab of a repository (`Cmd+5`, after Notes).

- Only that repository's pull requests, in the workspace tab's table without grouping. Filters: open, needs your review, yours, drafts, and merged and closed in the last 30 days, which are loaded when the filter is chosen and not kept up to date.
- The checked-out branch's pull request comes first, with its state and the commits the checkout has that the pull request does not (not yet pushed). It is found through the branch's upstream only, so a branch without one shows nothing there.
- A side panel says where the pull requests come from (`origin`, or the repository chosen with **Change…**) and which workspace tracks them.
- **States:** a repository no workspace tracks names the workspace to turn on, with **Turn On for** *workspace*; a repository whose `origin` is on neither provider offers **Choose Repository…**.

### Pull request

- The header carries back, the repository and number, the provider, the tabs **Overview**, **Files Changed**, and **Checks**, and one primary action: **Review Changes**.
- **Overview:** title, state, source and target branches, the head commit, and "N new commits since your review" when there are (GitHub records the commit a review was given on; Bitbucket does not, so there it is not shown; the count comes from local Git once the commits are on the Mac); the description; how many draft comments wait, with **Finish Review**; the conversation, with resolved threads folded and threads on changed code marked, each with **Reply** and, on a line, **Resolve** or **Reopen**; a comment box. A side panel lists what the pull request needs before it can be merged, as a checklist with **Merge**, which stays disabled until the list is complete; its reviewers; the local checkout, saying whether the pull request's branch is checked out; and the notes linked to the repository.
- **Files Changed:** a file tree with change kinds and comment counts and a filter; clicking a file's row marks it viewed, and a change to the file clears the mark (the marks stay on this Mac). Show all changes, only those since your last review, only those since the commit your drafts were written on when new commits arrived, or pick commits; unified or split; ignore whitespace. Threads and your drafts sit under their lines. Generated files, and files viewed and unchanged since, are folded. The diff says where it came from: local Git when both commits are on the Mac, which is also the only way to see the changes since your review or to ignore whitespace, otherwise the provider.
- **Checks:** each check's name, state, and a link to its log on the provider.
- Pull request text is Markdown written by other people. It is sanitized like notes: raw HTML in it is shown as text apart from a few harmless tags, links open in the browser and only to web and mail addresses, and images are shown as links, so opening a pull request loads nothing from it.

### Reviewing

- **On a line:** a line number selects the line, and Shift-click extends the selection to a range; the `+` that appears on a line, or `C` on the selection, opens a comment box under it. The comment is a draft: shown under its line and in **Finish Review**, where it can be edited or removed, kept on this Mac across restarts (with Brainiac's data, so it is backed up), and sent with the review. A suggested change is a comment.
- **On the pull request:** the comment box at the end of the conversation posts at once. **Reply** in a thread posts at once; on GitHub a reply to a comment on the pull request or to a review is another comment on the pull request, since GitHub threads only comments on lines. **Resolve** and **Reopen** are offered on threads on lines.
- **Review Changes**, the header's primary action, carries the count of drafts and opens **Finish Review**: the summary, the verdict (comment, approve, or request changes), the drafts to send, and the commit being reviewed, which is the head on screen. Approving needs no words; a comment or a request for changes needs a summary or a draft.
- The review goes on that commit and only on it. Brainiac reads the branch's tip again just before sending; if it moved since the dialog opened, or a draft was written on an earlier commit, nothing is sent: approving is turned off, the drafts are kept, and the dialog offers **Show New Changes** (Files Changed, "Since your drafts": what changed since the commit the drafts were written on, from local Git) and **Continue on** the current commit, which moves the drafts to it. A draft on a line those commits changed may then be refused by the provider; the message says so, and the draft can be edited.
- On Bitbucket a review is sent as several requests: each line comment, then the summary, then the approval or the request for changes. A review cut off midway (a lost connection, Brainiac quitting) is shown in Finish Review with what was sent, and **Send the Rest** posts only what is left, never a comment twice.
- A write whose answer never came may have landed. Brainiac reads the conversation again and looks for it before saying it was not posted; what it finds there counts as sent.
- A refusal for a permission the token lacks names the permission, and that action stays off for the account until the token is replaced in Settings → Accounts.

### Merging

- The checklist in the Overview's side panel is complete when the pull request has no conflicts, no failed or running checks, no reviewer asking for changes, and no unresolved thread. Approvals are shown but not required, since Brainiac cannot know which the repository demands. **Merge** stays disabled until the list is complete and the provider allows merging (not a draft, not conflicting, the account can push to the repository, the token can merge).
- **Merge** opens a confirmation with the checklist, the merge methods the repository allows (a merge commit, squash, rebase, and on Bitbucket fast-forward; a method one provider lacks is not offered there), the commit message, and whether to delete the source branch on the provider (the repository's default: GitHub's setting to delete it on merge, which GitHub then does itself, or Bitbucket's close-source-branch choice on the pull request; a branch in another repository, such as a fork's, is left alone). The button names the commit being merged.
- The commit message is prefilled as the provider would write it: a merge commit says "Merge pull request #N from branch" with the title below; a squash is titled with the title and number and carries the description; a rebase or fast-forward keeps the commits' own messages and takes none. On Bitbucket the message is one text.
- If new commits arrived since the dialog opened, nothing is merged: the dialog shows the new head, and the new changes are offered in Files Changed. GitHub refuses the merge itself when the commit changed. Bitbucket cannot be told which commit to merge and merges the branch as it is at that moment, so Brainiac checks the branch just before merging; a push in the second between that check and the merge is still merged.
- A merge whose answer never came may have landed: Brainiac reads the pull request again and reports it merged when it is, otherwise says so and lets the user try again. A merge that went through but whose branch could not be deleted is reported as merged, with the branch left.
- Merging is done by GitHub or Bitbucket and cannot be undone from Brainiac. The local repository changes only when it is next fetched.

### Staying up to date

- The pull request on screen is refreshed when opened and every 60 seconds while visible; workspace lists every 5 minutes while Brainiac is open. A fetch that moves a pull request's branch refreshes it at once.
- Bitbucket allows each user about 1,000 requests an hour and GitHub 5,000. A workspace of 30 repositories costs about 400 an hour on Bitbucket and far fewer on GitHub, where asking whether anything changed is free. Brainiac spends them in order (the pull request on screen, then lists, then everything else), asks only for what changed, and shows each account's use. When a provider is out of requests or unreachable, Brainiac shows what it had with its age, says when it will try again, and the Git views keep working.
- Pull requests, files, and conversations are cached on the Mac so the tabs open at once. The cache is never backed up or exported, can be deleted, and forgets a pull request 14 days after it closes. Review drafts are the user's own text and are backed up with Brainiac's data.
- An edit refused because the pull request changed since it was shown (new commits, or someone else's edit) shows the current pull request and keeps what the user wrote, like a note save conflict.

## 11. Databases — v0.4

A database client next to the repositories, notes, and tasks it relates to: saved connections to SQLite files and PostgreSQL servers, an editor that runs SQL, a fast result grid, saved queries for the ones used every week, and a live view of a PostgreSQL server's health. It covers what a programmer reaches for daily, not everything a full database tool does (`docs/design/databases.md`, Not in this design). The design behind it, and the options compared, are in that file.

### Boundaries

- **Read only unless allowed.** A connection is read only unless its access is set to Read and write, and even then each tab on a Production connection starts read only. On a read-only connection a statement cannot change data (Safety, below).
- **Passwords are kept only in the Keychain**, asked for once per run of Brainiac, or read from an environment variable or a command (section 12). They never appear in Brainiac's files, logs, backups, or exports.
- **Results stay in memory** while their tab is open. Copy and Export are the only ways rows leave the app; the history keeps statements, never rows.
- Brainiac's own databases (`brainiac.sqlite3`, `index.sqlite3`, `history.sqlite3`, `forge.sqlite3`) can be opened, always read only.
- Agents get no database tools, and nothing writes SQL for the user.

### Connections

- **Databases** is a sidebar section after Notes (also View › Databases and `Cmd+K`). It opens on **Home**: the connections as cards (kind, where it points, access, and a SQLite file's size), each with **New Query** and, for PostgreSQL, **Health**; the saved queries by folder, each with **Run**, **Open**, **Copy as Markdown**, and **Delete…**; and one field that filters both. Home is always the first tab.
- **New Connection…** asks for a kind and its fields:
  - **SQLite:** the database file, chosen with the macOS file dialog. Brainiac never creates a file.
  - **PostgreSQL:** host, port, database, user, password, and TLS: **Verify** (the default: the Mac's trust store plus an optional CA file, for providers whose certificates the Mac does not trust), **Require without verifying**, or **Off**. Pasting a `postgres://` or `postgresql://` URL fills the fields and moves its password into the password field.
  - **Name**, **Environment** (Local, Development, Staging, Production), **Access** (Read only, or Read and write; a new Production connection starts read only), the **time limit** of a statement (30 seconds by default), and for PostgreSQL **Runs on** (Health, below).
  - **Password:** kept in the Keychain (item `brainiac/db:<connection id>`), asked for once each time Brainiac runs, read from an environment variable or a command (section 12), or none. Editing a connection without typing a password keeps the saved one; **Refresh Password** forgets the one kept for this run.
- **Test Connection** connects once with the form's fields, reading the form's password source afresh, and reports the server's version, or the reason in words: a password the server refused, a database that does not exist, nothing listening at the host and port, a host not found, a certificate this Mac does not trust or that is for another host name, or a server that does not offer TLS.
- A connection's environment is its color everywhere it appears, and its name in words beside it: a strip along the tab's editor, a dot on its tabs, a label on Home and in the switcher. Production is red.
- A connection can be linked to repositories. A repository's Notes tab lists its linked connections, each with **New Query**, and **Link a connection…** adds one.
- **Delete Connection…** asks first, removes its Keychain item, and leaves the database itself untouched; saved queries that ran on it keep their SQL and lose their connection. A Keychain item that cannot be deleted leaves the removal in Settings → Secrets, with **Retry**.

### The query view

- **Tabs.** Each tab is one editor with its own connection and its own database session, so a setting or a transaction in one tab never reaches another. A tab shows its saved query's name or "Untitled n", a dot in its connection's environment color, and a dot while its text differs from its saved query (or, untitled, is not empty). Tabs and their text are kept across restarts. `Cmd+T` opens a tab; `Cmd+1` is Home and `Cmd+2`…`Cmd+9` the tabs after it. Closing a tab with unsaved text that is not a saved query asks first.
- **Connection.** The toolbar's connection button names the tab's connection, its environment, and "Read only" when the tab cannot write, and opens a filterable list to switch the tab to another connection, or add one. Switching keeps the text and closes the tab's session.
- **Editor.** SQL in the connection's dialect, with completion of keywords, tables (qualified, and bare for the default schema), and columns from the schema.
- **Running.** `Cmd+Enter` runs the statement under the cursor, or each statement in the selection; with the cursor after a statement's `;`, that statement. `Shift+Cmd+Enter` (**Run All**) runs every statement in turn and stops at the first failure; each statement that ran gets its own result. `Cmd+.` (**Cancel**) cancels the statement on the server and stops Run All. A statement that runs longer than the connection's time limit is stopped by the server.
- **Errors** are shown under the editor with the database's own message, detail, hint, and code, the line and column when the database reports where, and **Go to Error**, which puts the cursor there; the place is also underlined in the editor until the text changes.
- **Explain** (`Cmd+E`, or the Explain menu) shows the plan of the statement under the cursor as an indented tree with costs and estimated rows, without running it. **Explain Analyze** runs it to measure it, adding actual rows, time, and loops; in a tab that can write it asks first. SQLite shows its query plan, and its Explain Analyze times the statement.
- **Side panel** (shown or hidden from the toolbar) holds three things for the tab's connection only: **Schema** (schemas, tables, views, and materialized views with PostgreSQL's row estimate; expanding one lists its columns with type, nullability, and default, its indexes, and its foreign keys, and **Select Rows** opens `select * from <table> limit 100` in a new tab; **Refresh Schema** reads it again; the schema is read when a tab on screen first uses the connection, once however many tabs ask), **Saved** (the saved queries that run on this connection), and **History**.

### Results

- **Grid.** Only the rows on screen are drawn, so a 10,000-row result scrolls like a short one. Headers show each column's name and type; columns resize by dragging their edge; clicking a header sorts the rows on screen (ascending, descending, then as returned), with NULLs last. Numbers are right-aligned, `NULL` is drawn apart from the text "NULL", and long values are cut with an ellipsis. The arrow keys move the selected cell.
- **Inspector.** Hidden until its button shows it, and then kept shown: the selected cell's whole value beside the grid: JSON pretty-printed, long text wrapped, binary as its size and hex. A value is cut at 64 KB for the window, and binary at its first 4 KB; the inspector says so, and Export writes the whole value.
- **Values** are shown as the database prints them: exact `numeric`, intervals as `1 year 2 mons 3 days 04:05:06`, arrays as `{1,2,NULL}`. Integers beyond 2⁵³ stay exact. `timestamptz` is shown in the Mac's time zone with its offset. A few rarely selected PostgreSQL types (geometry such as `polygon`, `tsvector`, the `reg*` types) are shown as their type with "cast to ::text".
- **How much is fetched.** A statement returns at most 1,000 rows, and says "first 1,000 rows · more available". **Fetch All** runs it again for up to 100,000, and stops sooner when the rows reach 128 MB, counting each value as shown; it is offered only when the statement ran read only, so fetching more never repeats a write.
- **Copy** the rows as tab-separated text (pastes into a spreadsheet), CSV, JSON, a Markdown table (pastes into a note), or SQL `INSERT` statements; `Cmd+C` in the grid, or the inspector's **Copy**, copies the selected value. Copy takes values as the result holds them, so a value cut for the window is copied cut, and the notice says how many were; Export writes them whole.
- **Export…** writes every row to a CSV or JSON file chosen in the save dialog by running the statement again read only, so it is offered only for statements that ran read only. The file appears only once it is complete.
- A statement without rows shows its command tag ("UPDATE 42") and whether it committed or waits in the open transaction.

### Saved queries

- **Save Query** (`Cmd+S`) names the tab's text: a name, a folder (`Billing`, `Support/Weekly`), an optional description, and the connection it runs on. Saving a tab opened from a saved query updates it; **Save as New** makes another. If the saved query changed elsewhere since the tab opened it, the save is refused and the tab keeps its text.
- **Parameters.** `:name` in a statement is a parameter; inside strings, quoted names, comments, and `::type` casts, `:` is not. Running a statement with parameters shows a form above the editor with a field per name, prefilled with the last values, and a **NULL** switch per field. Values are sent as values, never pasted into the SQL: PostgreSQL reads each with the type it expects there and says so when it cannot ("abc" for an integer); SQLite gets a number when the value is one, and text otherwise. A saved query keeps the values it last ran with.
- Saved queries are listed on Home, in the side panel's **Saved** for their connection, and in `Cmd+K`, which finds them by name, folder, description, or SQL and runs one at once on its connection, asking for its parameters.
- **Copy as Markdown** puts the query's name, connection, description, and a fenced `sql` block on the clipboard, for a note.
- Saved queries are the user's own text: kept in Brainiac's data and backed up with it.

### History

- Every statement that runs is recorded: its SQL, the connection, when, how long it took, and the rows returned or changed or the error, never the rows themselves. The side panel's **History** lists the tab's connection's last runs, newest first, and searches their SQL; opening one puts its SQL in a new tab. **Clear History of** *connection* asks first.
- History keeps 90 days or 10,000 runs per connection, whichever is fewer, and is turned off in Settings → Databases.

### Safety

- **Read only means read only.** On a read-only connection, or in a tab running read only, a statement cannot change data: every PostgreSQL statement runs alone in its own read-only transaction, so it cannot also turn the transaction to read and write, and a SQLite file is opened read only by SQLite itself; `ATTACH` and `VACUUM INTO`, which could write another file, are refused. A statement that tries to write fails with "This connection is read only"; `nextval` is refused the same way, and so are PostgreSQL's `COPY` and `LOAD`, which can write files or run programs on the server. Side effects outside the data are not covered: advisory locks, `dblink`, and the server functions a powerful role may call (`pg_terminate_backend`, `lo_export`, `pg_reload_conf`). A read-only database role remains the real guarantee.
- **Modes.** A tab on a read-and-write connection runs **Read only**, **Auto-commit** (each statement commits), or **Manual**. New tabs start in Auto-commit, except on Production, where they start read only and turning on writes asks first and applies to that tab only. In Auto-commit a statement still runs read only first, so the result knows whether running it again (Fetch All, Export) is safe; a statement the database refuses as a write then runs for real, unless Cancel came first. In Auto-commit and Manual, a PostgreSQL statement waits at most 5 seconds for a lock another session holds and then fails, so a schema change does not queue every later query on its table behind it. A typed `BEGIN`, `COMMIT`, or `ROLLBACK` runs only in Manual: elsewhere each statement is its own transaction, so it is refused with a pointer to Manual rather than seeming to hold back the statements after it. Editing a connection makes a tab's mode fit it: a mode the connection no longer allows, or writes on a connection that became Production, goes back to the connection's default.
- **Manual transactions.** The first statement opens a transaction, and a bar above the editor says "Transaction open · 3 statements · since 14:02" with **Commit** and **Roll Back**; a typed `BEGIN`, `COMMIT`, or `ROLLBACK` does the same. A `COMMIT` the server refuses (a deferred constraint, a serialization failure) still ends the transaction, and the error says nothing in it was committed. After a failed statement PostgreSQL refuses more until Roll Back, and the bar says "Transaction failed · Roll Back to continue". After 5 idle minutes (2 on Production) the bar turns amber, since an open transaction holds locks others may wait on; the server ends a transaction left idle for 15 minutes (5 on Production). While a transaction is open the tab stays in Manual, its connection and mode cannot change while a statement runs, and an edit to its connection applies once the transaction ends. Closing the tab, switching its connection, or quitting (`Cmd+Q` or closing the window, which quits) with a transaction open asks first, and the answer rolls it back. Quitting from the Dock or when logging out does not ask: the server rolls the transaction back.
- **Sessions.** A tab's session opens on its first run, closes after 10 idle minutes (never with a transaction open), and at most 8 are open at once, the least recently used idle one closing first. A tab whose session was closed or lost runs on a new one and says "reconnected", since settings such as `SET search_path` are gone. A read-only statement that hits a lost connection runs again once on a new session; any other says the connection was lost and that the statement may or may not have been applied, and an open transaction was rolled back by the server. A PostgreSQL server is asked to notice within about two minutes that Brainiac went away (the Mac slept or changed network) and to close a session left idle for 15 minutes; quitting ends Brainiac's sessions cleanly.
- A SQLite file inside a repository is the user's file, like a note in a vault that is a Git repository: Brainiac writes to it only through a read-and-write connection, on a statement the user runs, and touches nothing else in the repository.

### Health

What a PostgreSQL server is doing now and over the last hour, as a tab opened from a connection's card on Home. It is a live view, not monitoring: it samples every 10 seconds only while the tab is visible and Brainiac's window is not minimized or hidden, on a session of its own named "Brainiac health" with a 2-second time limit, keeps the hour in memory only, and sends no alerts. A SQLite connection has no Health.

- **From PostgreSQL itself**, with nothing to set up:
  - **Connections** in use against `max_connections`, split into active, idle, and idle in transaction, marked **Near the limit** (an icon and words, and a banner) from 85%.
  - **Cache hit ratio** and **transactions per second**, with rollbacks and deadlocks, each with a one-hour line.
  - **Longest open transaction**, and the temporary files written in the last hour (queries that ran out of `work_mem`).
  - **Sessions**: pid, user, application, state, how long, what it waits for, and its statement; idle-in-transaction and waiting sessions first. Health's own session is not listed.
  - **Waiting on locks**: each waiting session and the sessions blocking it.
  - **Most time spent**: the statements that took the most total time, from `pg_stat_statements`; a row opens its statement with `explain` in a new tab. Without the extension, the panel says how to enable it.
  - **Largest tables**: size with indexes and TOAST as of the last vacuum or analyze (read without waiting for a table someone holds locked), rows, the share of dead rows, and the last autovacuum.
- Seeing other users' statements needs the `pg_monitor` role; without it they are shown as hidden and Health names the role to grant.
- **From where the server runs**, because PostgreSQL does not report its machine's memory, CPU, or disk. The connection's **Runs on** names one place:
  - **Google Cloud SQL** (project and instance): memory, CPU, and disk from Cloud Monitoring, read once a minute and about a minute behind, with the Google credentials `gcloud auth application-default login` saves on the Mac. Brainiac reads that file when needed and never copies it; the account needs the Monitoring Viewer role.
  - **Docker container** on this Mac: memory against the container's limit (without reclaimable page cache, as `docker stats` shows it), CPU, and disk I/O, from Docker's API; **List Containers** offers the running ones. Brainiac sends Docker only read requests.
  - **Not set**: Health shows the PostgreSQL panels and offers to set it. Coolify is listed as needing SSH tunnels, which come later.
- Each value says where it came from: "Postgres", "Docker", or "Cloud Monitoring · 1 min behind".
- Health reads the connection's password only to connect. When it cannot be read or the server refuses it, Health shows why and tries again after a minute, so a locked password manager is not asked every 10 seconds.
- **Cancel Query** and **End Session** are offered on other sessions when the connection's access is Read and write; each asks first, naming the session's user, application, and statement.

## 12. Secrets — v0.4.x

Where an account's token and a connection's password come from. Brainiac writes secrets only to its own items in the macOS Keychain; every other source is read, never changed. The design, the options compared, and the sources still to come are in `docs/design/secrets.md`.

### Sources

| Source | For | What Brainiac saves |
| --- | --- | --- |
| **Keychain** (the default, and every secret saved before) | Accounts and PostgreSQL connections | Nothing: the item is `brainiac/github`, `brainiac/bitbucket`, or `brainiac/db:<connection id>` |
| **Ask each run** | PostgreSQL connections | Nothing: typed once per run of Brainiac and kept in memory |
| **Environment variable** | Accounts and PostgreSQL connections | The variable's name |
| **Command** | Accounts and PostgreSQL connections | The program's full path and each argument |
| **No password** | Connections (SQLite always) | Nothing |

- What is saved says where to look, never the secret, and it is exported and backed up. The command picker says so: an argument holds a reference such as `op://Work/db/password`, never a password or token.
- **Environment variables** are Brainiac's own, as they were at launch. An app opened from Finder or the Dock does not get a shell's variables, so they suit `pnpm tauri dev` and scripts; the picker says so and suggests a command.

### Commands

- The program is chosen by name or path. **Find…** looks a name up in Brainiac's `PATH`, then `/opt/homebrew/bin` and `/usr/local/bin`, and the full path found is shown and saved; it is never looked up again. A program that is gone asks to be chosen again.
- Each argument is its own field, and the picker shows the exact array the program runs with, such as `["/opt/homebrew/bin/gh","auth","token","--hostname","github.com"]`. There is no shell: no quoting, variables, `~`, or pipes.
- The program runs without a terminal and with nothing on its input, in a folder of Brainiac's own, with Brainiac's environment and `GIT_TERMINAL_PROMPT=0`. A password manager may show its unlock window. It runs with the user's permissions; Brainiac does not sandbox it.
- It must print only the secret. Exactly one final line break is removed; everything else, spaces included, is kept. More than 64 KiB on its output or its messages stops it, as do 60 seconds, together with anything it started.
- Its output and messages are never shown, logged, or saved, even when it fails. An error names the program's file name and its exit status, and suggests running it in Terminal to see whether it waits for a sign-in.

### When a secret is read

- When it is first needed in a run of Brainiac, and then kept in memory until Brainiac quits. Uses that need it at the same time share one read, so the Keychain asks at most once.
- **Refresh** (Settings → Secrets, and **Refresh Password** on a connection) forgets it, so it is read, or asked for, again when next used. A server that refuses it forgets exactly that secret: a newer one read meanwhile is kept. An open database session keeps the credentials it connected with.
- An account's token read afresh is checked with the provider before it is used, once per read. It must belong to the saved account's user: a command whose tool switched accounts is refused with both names, and Brainiac never switches the account by itself. The check updates the token's kind, expiry, and scopes, but never makes a read-only account able to write.
- **Test** reads the form's source afresh and never uses or changes what is kept for the run. Its result is for the fields it ran with and disappears when they change. A test of exactly what is saved is shown in Settings → Secrets until the source changes or Brainiac quits.

### Saving and removing

- A secret typed for the Keychain is written there only after the account or connection is marked as being saved, and the mark is cleared once both are saved. A save cut off in between, by a failure or a crash, leaves the account or connection unusable, with a message, until it is saved again with the secret typed again or another source. Brainiac never finishes such a save by itself. A new connection whose password cannot be kept is not added at all, so trying again does not leave a second one.
- Moving from the Keychain to another source uses the new source at once and then deletes the old item. An item that cannot be deleted is shown in Settings → Secrets with **Retry**, and saving says so; it is never used instead of the new source, nor taken up again by choosing the Keychain without typing the secret.
- **Remove** (an account) and **Delete Connection…** mark it as being removed, delete Brainiac's Keychain item whatever the source is, and then remove it. When the item cannot be deleted, the removal stays in Settings → Secrets with **Retry**.

### Settings → Secrets

- The store in use, and each account and connection with a secret: where it comes from, where Brainiac sends it ("api.github.com as octo", "db.example.com:5432/app as app"), and its state: **not read until allowed** (restored), a save that did not finish, a cleanup or removal to retry, a password to be asked for, or the last test and when it ran ("Not tested" otherwise).
- **Allow…**, **Retry**, and **Refresh**. Where a connection's password is sent includes its TLS mode and CA file. A source is changed where it is entered: in Accounts, or in Edit Connection….
- Opening it reads no secret, runs no program, and does not unlock the Keychain.

### Restore

- Every restored account and connection keeps where its secret comes from, and Brainiac reads none of them until the user allows each one: neither to use it nor to test it. **Allow…** shows where it reads, where it sends the secret, and, for a command, the exact program and arguments, and then **Allow This Source**. An item with the same name in this Mac's Keychain is not trusted without it. Saving the account or connection also allows what it shows.
- A save, cleanup, or removal that had not finished when the export was made is shown as such. Neither restoring nor saving a restored account or connection deletes an item in this Mac's Keychain: only **Retry** does.
- A snapshot from the `backups` folder is this Mac's own data. Copied back by hand, it keeps the sources it allowed.
- Upgrading keeps every existing Keychain item and what each connection did before, and reads nothing.

### Boundaries

- A secret never reaches the window, a log, Brainiac's databases, a snapshot, or an export. A secret the user types travels once to Brainiac's backend, and the form clears it.
- A secret is sent only where it belongs: an account's token to its provider's API, a connection's password to its own server.
- Agents can neither read a secret nor change where one comes from.
- Later: Google Secret Manager, Git's credential helper, and `~/.pgpass` (v0.4.x, chosen by use), and the Secret Service if Brainiac ever builds for Linux.

## 13. Agent runs — v0.5

A coding agent run in a container, started from a repository, followed live, steered with follow-up prompts, and reviewed in the diff viewer before its work leaves Brainiac. This section is Claude Code or OpenCode on this Mac's Docker engine or on an approved Linux host, ending in a patch. Pushing a branch, Codex and Gemini CLI, and runs from tasks follow within v0.5; their design, the options compared, the spike records, and the UX canvas are in `docs/design/agent-runs.md`.

### Boundaries

- **The user's repository is only read.** Its checkout is never mounted into a container, and its working tree, index, refs, configuration, and hooks are never changed. A run gets one commit and its history, copied into Brainiac's own repository in its data folder (`docs/architecture.md`, Agent runs — v0.5). Fetching stays the only write to a user's repository (section 2).
- **Code and prompts go to the model provider during the run.** Reviewing decides what is published, not what is sent. New run says so before it starts.
- **The agent can read the credential it is given, and so can the repository's own code and whoever controls the Docker engine.** On a remote host, that includes whoever administers the host. Scripts the agent runs (tests, builds, installs) and the repository's Claude Code hooks see it in their environment, a hook even before the first prompt, so a run is for a repository you would trust with the token or key. OpenCode's commands see its key the same way. Network access is unrestricted: the agent can reach any site, including services on the user's network.
- **Nothing is pushed in phase 1.** The result leaves Brainiac only as a patch the user copies or saves, or as a command the user runs in their own repository to fetch it as a new branch; Brainiac does not run it.
- **Two tested agents:** Claude Code through its ACP adapter, and OpenCode, which speaks ACP itself, at versions pinned in an image Brainiac builds. A request from the agent for a file or terminal on the Mac is refused. Brainiac's MCP server is not offered inside a run. The repository's own Claude Code setup applies, as on the Mac: its CLAUDE.md, and its `.claude` settings (hooks, permission rules, environment), except that the token or key always goes to Anthropic over verified TLS. For OpenCode, the repository's AGENTS.md applies but its own OpenCode configuration (`opencode.json`, `.opencode/`) is not read, because a setting there can send the key to another server; the image fixes each provider's address and turns off OpenCode's own models, sharing, and updates.
- **Agents (section 9) cannot start runs or change Settings → Agents.**

### Settings → Agents

- **Run hosts:** where runs execute, as a list in Settings → Agents: This Mac first, then each approved Linux host (Remote hosts, below). A row shows the host's engine or SSH address, its image and test, and one state: Ready, Not set up, Waiting for confirmation, Upgrade available, a job in progress with its step (Installing, Upgrading, Building the image, Testing, "step 2 of 6"), or the job that failed. A row opens the host's page (Agents › Run hosts › the host). **Add host…** adds a remote host (Remote hosts, Adding a host).
- **This Mac's page:** this Mac's Docker engine, chosen by its socket (OrbStack or Docker Desktop; others when tested), with its state and whether the run controller is running, then this Mac's image and each agent's test. It says that runs pause while the Mac sleeps, and that a run past its time limit is stopped on wake. An engine on which Brainiac cannot enforce the workspace size, or that fails the test, is not offered.
- **Agents:** a list of Claude Code, OpenCode · Anthropic, OpenCode · OpenAI, and OpenCode · OpenRouter, each with Ready or what it still lacks. Each is set up on its own page (Agents › OpenCode · OpenRouter): how it is paid, the agreement, and its new runs' defaults. Its token or key comes through the credentials layer (section 12): its source is saved, never the secret, and exactly one reaches a run.
- **Claude Code**, paid with one of:
  - **Claude plan:** a token the user creates with `claude setup-token` in Terminal and pastes. A token Terminal wrapped over two lines is joined, with the spaces and invisible characters the wrap and the copy add; two tokens, or anything that still does not look like one, are refused. Brainiac never signs in to claude.ai, never reads Claude Code's own Keychain item or `~/.claude`, and never copies login files into a container. It records when the token was saved and warns from eleven months on. Runs use the plan's usage limits, shared with the user's other Claude use, and show "Uses your Claude plan" instead of a cost. This option is off by default and ships in a release only once Anthropic's answer on its terms is recorded (`docs/design/agent-runs.md`, Subscription token).
  - **API key:** an Anthropic API key.
- **OpenCode**, paid with the provider's API key: Anthropic's, OpenAI's, or OpenRouter's, which reaches many providers' models with one key. ChatGPT plans and OpenCode's own models are not offered.
- **Sends code to:** the provider and plan, with a checkbox the user ticks to agree that runs send the repository's history up to the start commit, prompts, and anything the agent reads.
- **Image:** a readable Dockerfile that Brainiac builds, with the base image, Claude Code, its ACP adapter, OpenCode, and the collector each at a pinned version (**View Dockerfile**). Each host builds it for itself, from its page, which shows its digest and build date with **Build image** or **Rebuild**; one image serves every agent.
- **New runs**, for each agent: default permissions (Ask before actions, or Act without asking), time limit, model, and CPU, memory, and workspace size; at first Ask before actions, 1 hour, 4 CPUs, 8 GB of memory, and a 20 GB workspace. For Claude Code the **model** is an alias it knows (opus, sonnet, haiku, opusplan) or a full model name, with no spaces; empty leaves the choice to Claude Code and the repository's own settings. OpenCode needs one: the provider's model name, such as `gpt-6.1-sol`, or on OpenRouter `anthropic/claude-sonnet-5-5`. The test runs with it, so a name the plan or key cannot use fails there.
- **Before the first run**, an agent's page lists what it still lacks (the token or key, the agreement, OpenCode's model), and a host's page what the host lacks (an engine, the install, the image), in the order to do it. Choosing another engine forgets the image built on the old one.
- **Test**, for each agent on each host's page: starts a short run, sends a prompt, cancels, and collects, and shows what passed and when. A wrong token or key can take a few minutes to be refused: Claude Code retries it first. A run of an agent cannot start on a host until its test has passed there for the current credential, image, and engine; a changed token or key needs a new test, because a wrong token can come back looking like an ordinary reply.

### Remote hosts

- **An approved host** is a Linux machine with systemd and Docker, reached by SSH with a key the user already has (an agent or a chosen identity file). Brainiac stores the user, host, port, the identity file's path, and the host key fingerprint. It does not store the private key, and it does not store the agent's token or key on the host's row. The address and the approved key belong to the machine, which later features can reach too; running agents is one role on it. Confirming the key again keeps what was installed. **Remove** forgets the machine and its key, unless the host still holds a run's work: then both are kept and wait for the key to be confirmed again.
- **The host key** is shown before anything is installed. An unknown key, or a key that changed, is refused until the user approves that fingerprint. Brainiac never accepts a key on its own.
- **Adding a host** is a dialog in steps: **Connect** (a name, the SSH user, host, port, and an optional identity file), **Confirm key** (the fingerprint, the command that prints it on the host for comparison, and every action Install runs there with `sudo`), then **Install**, **Build image**, and **Test**. **Trust this key and install** saves the host and runs those three one after another as one job, testing each agent that is set up; Test is not run, and says why, while no agent is. The dialog can be closed at any time after that; the job goes on, on the host's page.
- **Install** puts Brainiac's run controller on the host as a service that keeps running after SSH closes, under its own user in the `docker` group. The SSH user must be able to run the install with `sudo` without a password prompt; the dialog lists those actions first. The uploaded program is checked against the copy Brainiac built. Before any agent credential is sent, Brainiac checks that this is the controller it installed and that the host's Docker engine can attach the workspace's loop devices.
- **A host's page** shows its state, its SSH address and approved key, its setup in order (host key confirmed, run controller installed with its build and protocol, image built, each agent's test passed), each with its action (**Show key**, **Reinstall…**, **Build image** or **Rebuild**, **Test**), its live runs, **Emergency stop**, and **Remove…**. A restored host offers **Confirm host key…** instead.
- **Host jobs:** Install, Upgrade, Build image, and Test on a remote host run in the background, one at a time per host, as a job with numbered steps. Each step shows whether it is waiting, running, done, failed, or not run, and how long it took; the job shows when it started and its time so far; the running step shows its progress (the number of crates compiled while the controller builds) and the last lines of its output, with **Show the whole log** and **Copy the log**. Leaving Settings or closing the window does not stop a job. While one runs, the host's row, its page, New run, and the status bar say so ("build-01 upgrading · 7:41"), and New run does not offer the host. When it ends Brainiac says so in the window, and as a macOS notification when Brainiac is not the active app. A failed job says at which step and what the host answered, and offers **Try again** and **Copy the log**. The last job's steps and log stay on the host's page until the next job, also after Brainiac quits; a job that was running when Brainiac quit is shown as interrupted.
- **Install and Upgrade** are the same six steps: reach the host (SSH, `sudo` without a password, its processor); build the run controller for the host's platform on this Mac, in Docker; copy it to the host and check its SHA-256; wait until no run is live there; install the program and its service and start or restart it; check that it answers with the installation and protocol Brainiac expects. A controller an earlier setup left running on the host (Brainiac's data was deleted, or another Mac installed it) is replaced the same way: its runs are waited for, then it is restarted. The first build after a Brainiac update compiles the controller, under emulation when the host has another processor, and takes many minutes; Brainiac keeps the build's cache, so a later build recompiles only what changed, and the next install with the same Brainiac reuses the program. **Cancel** works until the install step begins and removes a copy already sent; nothing on the host has changed by then. A failure before that step leaves the host on its previous controller, and **Try again** reuses the program already built.
- **Upgrade** is explicit. A host whose controller is not the one this Brainiac builds shows **Upgrade available**, with both builds. Upgrade builds the new controller at once and then waits while a run is live on the host, because restarting the controller would end it; new runs and tests on that host wait from the moment Upgrade is chosen until the job ends, and Cancel stays possible while it waits. **Remove** is refused while a run is live, and does not delete the controller's files on the host while a run's container, volume, or cleanup is still there.
- **Build image** and **Test** on that host are the same steps as on this Mac, after the controller is in place, with a Test for each agent. Test is the first time the agent's token or key is sent. Settings says the host's administrator can see the repository and the credential.
- **Emergency stop**, shown in Settings, is a command run on the host. It stops the host's run containers and does not delete them. It works without the Mac app. On this Mac, stopping the engine is still the emergency stop.
- A host restored from a backup stays off until the user confirms it. Until then Brainiac does not deploy to it or start a run on it.
- One host that cannot be reached does not stop runs on another host, or on this Mac.

### New run

- **New run…** from Runs (`Option+Cmd+N`), the command palette, or a repository. The repository must be a complete local Git repository: a shallow or partial clone, missing objects, Git LFS pointers, or submodules in the chosen tree are refused with a remedy, before any container starts or credential is read. When more than one approved host can take a run, the dialog asks which one; otherwise it uses this Mac. A host with a job in progress is shown with its step and a link to its progress, and is not offered until the job ends.
- **Start from** a branch or a commit, resolved once to a commit. The dialog shows that commit, says that uncommitted changes are not part of the run, and says what the container gets: this commit and its history (with the count); other branches, stashes, hooks, remotes, and Git settings stay on the Mac.
- **Agent:** Claude Code or OpenCode with one of its providers, among those whose test passed on the chosen host; the others say what they lack. Its new runs' defaults fill the rest.
- **Prompt**, **Time limit** (30 minutes to 8 hours) with the time it ends, which counts waiting for the user and idle time too, **Model** (the agent's setting, changeable for this run), and resource limits with **Change…**.
- **Permissions:** **Ask before actions** waits for the user before the agent runs a command or edits a file; **Act without asking** lets it do anything inside its container, and never push, get new credentials, or change where it runs.
- **Before you start** names the provider and plan or key, the unrestricted network, and who can read the token or key: the agent, the repository's code (and its Claude Code settings, for Claude Code), and whoever controls the engine (and, on a remote host, whoever administers that host). **Start run** says nothing is pushed until the result is reviewed.

### The run

- **Runs** is a sidebar section after Databases (also View › Runs and `Cmd+K`), with a badge counting runs that need the user: a permission request, a session ready for the next prompt or stopped by the plan limit, or work waiting for a decision (uncollected, left-out files not yet added or accepted, a failed collection). Its list groups runs as Needs you, Ready to review, Active, and Ended, shows all of them or only those that need the user, are active, or ended, and is filtered by repository and host. Each row shows the run's title, repository and host, what it is doing, what it produced, and when (asked, idle or running for, the time it ends, or when it ended).
- A run's header shows its title, repository, start commit, agent, model, and engine, and permissions, then separate badges for what the agent is doing and what the run produced, the time it ends and the time left, "Uses your Claude plan" (or the cost when the provider reports one, "Unavailable" when not), **Cancel run…**, and **Finish and collect**. A remote run names its host.
- **Starting:** **Start run** opens the run at once. What can refuse a run (Settings, the repository, the host) still answers in New run; the rest goes on from the run's page, which shows it as steps with the time since Start run: copy the start commit and its history, send it to a remote host, hand the run to the run controller, then start the container and the agent. Leaving the page does not stop it. A failure on the way ends the run as Failed with the reason, with nothing left on the engine. **Cancel run…** works while it starts: before the controller has the run, nothing is started and it ends Cancelled; after, the cancel goes to the controller.
- **Conversation:** the prompts, the agent's messages and plan, and its tool activity, with each finished turn folded to a summary ("14 steps · read 6 files, ran 3 commands"). The agent's messages are Markdown, drawn like pull request text: sanitized, with raw HTML shown as text, links only to the web and mail, and images as links. A side panel lists the start, where it runs, the provider and credential source, the network, the model (the one the agent reported once its session opened, with the one asked for when they differ; a plan or key that cannot use the asked-for model gets the agent's default), limits with workspace use, the image, and the files the agent says it changed, marked as reported by the agent. An edit shows the change the agent reports with it, its text before and after (or only the part it replaces) as removed and added lines, cut short past 16 KB a side; it is what the agent says it did, not its working tree. **Changes so far** is available while the run is live, and **Changes** once the work is collected.
- **Next prompt** is enabled only while the run is idle and connected; **Send** starts the next turn on the same session.
- **Changes so far** reads the agent's working tree while the run is live: at the end of each turn, and on **Refresh**. It is read by the same rules as a collection, without stopping the agent, and shown against the start commit in the diff viewer, with when it was read and how many new files a collection would leave out. It is provisional: the agent keeps working, and a file it was writing may be caught halfway. It cannot be exported; **Finish and collect** makes the result to review, and Changes so far goes with it. A read that takes over 5 minutes is stopped; one that fails says why and keeps the one before, and a turn that ends during a read is read right after it. A read never delays the run's stop or its collection. A run whose controller is older than Brainiac says so: on a remote host, to upgrade it once no run is live there; on this Mac, that runs started after it exits can.
- **Permission requests** (in Ask before actions) show the command, or the edit as removed and added lines, that it runs inside the container, when it was asked, and that the run still ends at its time limit; **Allow once** or **Reject**. While Brainiac cannot reach the run, the request is shown and cannot be answered: nothing is allowed on the user's behalf. In Act without asking, Brainiac chooses allow once. A request that arrives after Cancel has no effect.
- **States**, each in words:

  | Fact | States |
  | --- | --- |
  | What the agent is doing | Preparing · Working, turn n · Waiting for you, permission · Ready for your prompt · Plan limit reached · Workspace full · Stopping · Finished, Cancelled, or Expired · Failed or Interrupted |
  | What the run produced | Collecting · Ready to review, n files · No changes · Collection failed |
  | Whether Brainiac can see it (no badge when connected) | Last reported: … · Cancel requested · Stop not confirmed |
  | Cleanup | Cleanup pending |

- **Plan limit reached:** the plan's usage limit stopped the turn. The session stays open and its work is kept, the time limit keeps running, and the user can send again later or finish and collect. Brainiac does not claim to know when the limit resets.
- **Workspace full:** the agent's writes fail for lack of space. The run is not stopped.
- **A rejected token or key** fails the run with its work kept for collection. A run never asks for a new credential: the user updates it in Settings and starts another run.
- **A run whose image is gone from the engine** fails before anything of it is made there; a cleanup of unused images on the engine's machine can remove the image between runs. It has no work to collect: the user builds the image in Settings → Agents and starts a new run.

### Leaving and coming back

- Quitting Brainiac does not stop a run. Reopening reconnects to the same session, shows the updates missed while it was closed once each, and allows the next prompt without sending the credential again. Losing SSH does not stop a remote run either.
- Logging out or restarting the Mac interrupts a run on this Mac: it is stopped and its work kept, as for any interruption. A remote run keeps going; the host is not this Mac.
- A run on this Mac does not work while the Mac sleeps. On wake, a run past its time limit is stopped and shown as "Expired while this Mac slept"; it may have worked for a few seconds after wake. A remote run keeps working while the Mac sleeps. Its time limit is the host's, and closing the lid does not extend it. A prompt or permission answer that was not acknowledged is not sent again.
- When Brainiac cannot reach the run (the engine or the host is not answering), the run shows its last report and when it was made, retries, and never claims the run stopped. **Cancel** meanwhile is shown as requested and is sent when Brainiac reconnects. Another host's runs are unaffected.

### Ending and collecting

- **Finish and collect** (only while idle), **Cancel**, the time limit, a failure, or an interruption all stop every process in the container and confirm it stopped before anything is collected.
- **Collecting** snapshots the agent's working tree, including edits it never committed: staged and unstaged changes, new files, binary files, symbolic links (as links), executable bits, and deletions. It is compared with the start commit as one new commit on top of it; the agent's own commits are not kept as history.
- **Left out:** new files that the start commit's ignore rules or Brainiac's fixed rules exclude (generated output, caches, Git's own data, home folders). Review lists them with the reason; **Choose files to add…** collects again with the chosen regular files, and **Keep this snapshot** accepts the list. Until one of them, or a patch export or Copy branch command that confirms the same list, the stopped container is kept.
- **Interrupted** (the engine, the container, or the run controller stopped unexpectedly): the container is stopped and kept with its files; only Discard removes them. The conversation cannot continue; **Collect work** collects it for review. Updates after the last one recorded may be missing.
- **Collection failed:** nothing is removed from the engine; the stopped container and its files are kept, with the reason ("over the 200 MB limit for one file", or that the run's workspace is not on the engine), **Retry collection** and **Discard work…**. Exporting the files of a failed collection follows in 0.5.x.

### Review

- **Changes** shows the collected snapshot against the start commit in the diff viewer, with the files, additions, and deletions, and says it includes edits the agent did not commit.
- **Copy patch** and **Save patch…** export that comparison, binary changes included within the artifact limits.
- **Copy branch command** copies a Git command for the user to run in their own repository: `git fetch` from Brainiac's repository in its data folder into a new branch `agent/<a few words of the title>-<the first 8 characters of the run's ID>`, holding the snapshot commit on top of the start commit. Brainiac does not run it, so the user's repository stays only read. Git refuses to move a branch of that name that holds other work, and only moves one that is behind the snapshot. The command works until the run is deleted.
- What the agent said it changed is never taken as the result; the collected snapshot is.

### Deleting and keeping

- **Delete run…** removes the conversation, the review, and the stopped container with its files, and names any work that would be lost (left-out files never added or accepted). It is refused while Brainiac cannot confirm the run stopped.
- Ended runs are removed 30 days after they end, with their prompts, conversation, and results. Work waiting for a decision (uncollected, left-out files not yet accepted, a failed collection) stays until the user decides.
- **Cleanup pending:** a container, volume, or file that could not be removed stays listed with **Retry cleanup**; it never blocks reviewing a result.
- Runs, their conversations, and their results are not part of a vault export. Settings → Agents are exported as where each agent's credential comes from, never the credential, and a restored agent must be confirmed before Brainiac reads its credential or starts a run of it. This Mac's engine and image are not restored: the user chooses the engine and builds the image again.

### Credentials and what reaches the container

- The credential is read on the Mac when a run starts and handed to the agent in memory. It never goes into the container's settings, its environment as Docker shows it, the image, labels, arguments, logs, the conversation as saved, or Brainiac's files.
- Known injected values are removed from the conversation before it is saved or shown. That is best effort for values Brainiac knows; the agent can write a secret into its files, and a review cannot certify that a result holds none.

## 14. Explaining changes — v0.6

Brainiac explains a change to the person reading it, so they learn the software being built, not only what moved. An agent writes the explanation in a run of its own kind (section 13): it reads the change, the code around it, and the project's own docs in a container, and returns one file that Brainiac checks and shows beside the diff. The reasons, the spike, the UX canvas, and the open questions are in `docs/design/code-explanations.md`.

### Boundaries

- **Never automatic.** An explanation starts only from **Explain**, for one subject: a commit (History), a branch compared with the default branch (Branches & tags, where a selected branch now also shows **Changes against main**: one patch from where it left the default branch to its tip), or a collected run's result (a run's Changes). Working-tree changes and pull requests follow in 0.6.x.
- **The repository is only read**, as for a run: the subject's commit and its history are copied into Brainiac's own repository and the container (section 13, Boundaries). Whatever else the agent changes is discarded; only its explanation comes back. An explain run is not listed in Runs and has no review. Its container and workspace are removed as soon as its explanation is checked, or when it fails; its conversation stays with the explanation (How it was written, below).
- **Code goes to the provider of the chosen agent profile.** The first Explain in a repository asks once whether its code may be sent for explanations, naming the provider; the answer is listed, and can be changed, in Settings → Explanations. A repository answered No is never explained, and a profile of another provider asks again.
- **A run's risks apply.** The agent acts without asking inside its container, the repository's code (and its Claude Code settings) can read the token or key, and the network is unrestricted (section 13, Boundaries). The question above says so.
- **Every claim cites its source**, and Brainiac shows only what it verified (Checks, below). It can verify that a note sits on the change and that its quotes exist, not that the note is right.
- **Agents (section 9) cannot start explanations or change Settings → Explanations.**

### Explain

- **Explain** is a button in the patch's toolbar, and `E`. A short dialog names the agent profile and host that will read the code (Settings → Explanations' at first), the depth (**Brief**, **Teach me**, or **Deep**), whether to add questions, and the time limit; the first time in a repository it also asks the question above. It needs an agent whose test passed on the host (section 13, Settings → Agents); otherwise it says what is missing, with a link to Settings → Agents.
- An explanation of the same subject, agent, and depth opens at once; **Explain again** replaces it.
- **While it works**, the panel shows its steps (copying the subject, starting the agent, reading, checking), the files the agent opens, the time since Explain and the time limit, and **Cancel**. Leaving the view does not stop it, and the subject shows "Explaining…" until it ends. One explanation runs at a time per subject.
- **A failure** (the time limit, a refused token or key, no valid file after the second turn) says why, with **Try again** and **How it was written**. Nothing partial is shown.

### The explanation

- **A panel beside the patch**, not a separate view: the summary (why the change exists, not only what moved), the sources the agent read, the agent, model, depth, and date, the cost when the provider reports one ("Uses your Claude plan" on a plan), and three tabs:
  - **Tour:** the changed files in reading order, each with its role. The file list switches between **Reading order** and **Path**; in reading order, `[` and `]` follow the tour.
  - **Concepts:** the ideas the change relies on (a language feature, a library, a system tool, a project pattern), each with its explanation and where it appears. **Got it** adds one to the ledger, and later explanations leave a known concept out unless the change uses it in a new way.
  - **Check yourself:** two or three questions, each with **Reveal answer**, when they were asked for.
- **How it was written** opens the explain run's conversation in the run view, read only: Brainiac's prompt, the files the agent read and the commands it ran, its replies, the follow-up turn when there was one, and the time and cost. It goes with its explanation: Explain again, Try again, or Delete all.
- **Notes in the patch:** a note sits after the lines it explains, with links to its sources (lines of a file at the subject's commit, or a doc's section). A concept's name in the code is underlined and opens the concept.
- **Code and docs disagree:** shown with both quotes and where they are, only when both were found in the files. **Add a task** creates a task linked to the repository; **Not a problem** hides it.
- **Out of date:** a branch's explanation stays with the branch when it moves. Each note keeps a hash of the lines it explains: a note whose lines are unchanged in the new comparison stays, and one whose lines changed or went is marked out of date, with **Re-explain**. Commits and run results do not change, so their notes never go out of date.
- **Save as note** writes the explanation into the vault as a Markdown note, `Explanations/<repository>/<short commit or branch> — <subject>.md`, with the repository, subject, date, agent, and model in its frontmatter, linked to the repository (section 5, Notes and repositories). It is a copy that does not change with the explanation.

### Checks

Brainiac checks the agent's file before anything is shown:

- It matches the schema, and its tour lists every changed file.
- Each note lies within changed lines on the new side of the change.
- Each quote is found verbatim in its file at the subject's commit: at the cited lines, or else elsewhere in the file, where it is anchored instead. A claim whose quote is not found is dropped.
- A code–docs disagreement needs both its quotes found.

A file that does not match the schema, or has notes outside the change, gets one more turn in the same session, listing what was wrong. If it still fails, the explanation fails with those reasons.

### Settings → Explanations

- **Agent:** the profile and host, at first those of the last run. **Model by depth:** for Claude Code, sonnet for Brief and Teach me and opus for Deep at first; for OpenCode, the profile's model. **Time limit by depth:** at first 10 minutes for Brief and Teach me and 20 for Deep. Each can be changed.
- **Your level** for each language (new, comfortable, or expert), and the default depth.
- **Concepts you know:** the ledger, with **Remove**, and **Merge** for one idea that explanations named in two ways.
- **Repositories:** each repository's answer, with **Change**.
- **Stored explanations:** how many and their size, with **Delete all**. Explanations are kept until deleted, each with its run's conversation.
