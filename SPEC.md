# Brainiac — Specification

**Updated:** 3 October 2026  
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

**v0.1 — Git viewer and single/multi-repository tracker** shipped as 0.1.3. **v0.2 — knowledge, tasks, and code context** shipped as 0.2.0: one Markdown vault, tasks, Today, keyword search, and links between notes, tasks, and the repositories v0.1 tracks (sections 5–8). **v0.2.x — agent access** is the current release: agents such as Claude Code work with Brainiac's notes, tasks, and repository links through a local MCP server (section 9). **v0.3 — pull requests** comes next, because the Git features proved the most useful: reviewing and merging the pull requests of a workspace's repositories on GitHub and Bitbucket Cloud. Content imports, global capture, and AI follow it.

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
| External content imports | v0.4 | Paste, bookmarks, Markdown copies, articles, `.eml`, provenance and duplicate handling |
| Global capture window and Inbox | v0.4 | System shortcut, floating capture, Inbox triage of captured and imported items, shared backend state |
| Authenticated import adapters | v0.4.x | Selected Jira issues/mail messages; provider choice and video transcript acquisition validated separately |
| Semantic search | v0.5 | Optional local embeddings and hybrid retrieval |
| Grounded AI answers | v0.6 | Citation-backed local RAG |
| Other pull request providers, CI beyond pull request checks, other Git mutations, sync, plugins | Later | Separate features after the v0.3 providers are useful |

v0.1 requires a usable local Git binary. Detect it on startup and provide a clear setup message when absent; do not silently install developer tools. Core Git viewing works offline and requires no Markdown vault, Ollama instance, remote-service account, or elevated macOS permissions. Ahead/behind information reflects existing local refs and may be stale relative to the remote server until someone fetches: the user, their editor, or Brainiac's Fetch now and opt-in auto-fetch (section 4, Fetching). Fetching is the only operation that writes to a repository, and it touches remote-tracking refs and objects only.

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

The sidebar gains a section level above the repository tree: **Today**, **Tasks**, and **Notes**, then All repositories, Workspaces, and Pinned as in v0.1. The section that holds the notes is called Notes; "brain" names the whole app. There is no Inbox section before v0.4 (section 6).

- Brainiac reopens the section that was open when it quit. The first launch after upgrading to v0.2 opens Workspaces as before, and the sidebar shows one **Set up your vault** row until a vault is chosen.
- Today, Tasks, and Notes are never shown empty: before a vault is chosen they offer the vault setup, and Today and Tasks work without a vault.
- Settings (`Cmd+,`) chooses the vault, holds the note-ID setting and **Rebuild Index**, and offers **Export…** and **Restore from Export…**, which the File menu also has. From v0.2.x it also holds **Agent access** (section 9).
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
| `Cmd+1` … `Cmd+4` | Repository tabs: Changes, History, Branches & tags, Notes (v0.2) |
| `Cmd+N`, v0.2 | New note; in a repository's Notes tab, a new note linked to it |
| `Cmd+Shift+N`, v0.2 | New task |
| `Cmd+S`, v0.2 | Save the note now |
| `Cmd+Shift+E`, v0.2 | Switch the note editor between Live Preview and Source |
| `Option+Cmd+0`, v0.2 | Show or hide the context panel in Notes |
| `Space`, v0.2 | Mark the selected task done or not done |
| `J` / `K` (or arrow keys) | Next / previous row in the focused list, without clicking it first |
| `[` / `]` | Previous / next file in a commit or the changes list |
| `N` / `P` | Next / previous hunk in the shown diff |
| `/` | Focus the filter of the current list |
| `Cmd+,` | Settings |
| `Escape` | Dismiss the palette, a dialog, or a menu |

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
- Enumerate immediate child directories of the discovery folder; query Git to resolve each candidate's working-tree root and metadata paths. Register a child only when its resolved canonical Git root equals the candidate directory. A plain folder that inherits the enclosing root's Git context is not another repository.
- Handle `.git` directories and `.git` files, including linked worktrees and actual submodules. Preserve the detected relationship; do not assume every child is a submodule.
- Membership records each member's path, origin (discovered or manual), and repository when it is one; an explicitly selected non-Git folder has none. The root is identified by the workspace's `root_repository_id`, not by a per-member role. Repository records can still appear in other workspaces.
- Deduplicate by canonical checkout root, retaining distinct linked worktree paths. Do not combine separate repositories merely because their current branch names match.
- A missing discovery folder leaves the root and any manually added members usable and shows the discovery issue. Non-Git children are skipped with a preview explanation.
- For discovered workspaces, watch the discovery folder for added/removed child folders and expose **Rescan**. Surface additions for the user to track; never add them silently. Mark removed or inaccessible registered members missing; preserve registrations and future context links until explicitly removed or relocated.
- Rescan runs when a discovered workspace opens and on **Rescan**. When a missing member and exactly one untracked repository in the discovery folder are the same repository (Relocating a repository, below), Rescan suggests the move (**Update**) instead of listing that folder as new. A folder that matches several missing members, or a member that matches several folders, gets no suggestion and is listed as new. Rescan looks for up to 16 missing members among up to 64 untracked repositories, with one Git run per untracked repository plus one per match.
- Stay within the discovery folder. Symlinked external repositories require explicit selection. Deeper descendants require explicit addition; do not crawl arbitrary nested dependency trees.
- Brainiac owns its tracked-repository selection independently of editor configuration. Any workspace can gain manually added repositories or drop members, whichever way it was created.
- A discovered workspace stores the selected folder as `discovery_root` and the scanned folder relative to it as `discovery_path` (absent when the selected folder itself is scanned), so Rescan works whether or not the selected folder is a repository.

#### Relocating a repository

A registration keeps its identity when its folder moves. **Locate…** points it at the new folder and keeps its ID, workspace memberships, pin, place in the recent list, last tab, and activity feed with its read state, all of which removing and adding the folder again would lose.

- **Locate…** appears wherever a missing repository is shown: its overview row, the selected-repository panel, and its viewer. The viewer's More menu and the command palette offer it for the open repository even when its folder exists, for example to switch to a fresh clone. The folder picker opens in the old folder's parent.
- The chosen folder must be inside a Git working tree; a folder below the top of a working tree stands for that working tree.
- **Same repository** means the chosen working tree contains a commit Brainiac recorded for the registration: the last observed `HEAD`, or a recorded tip of a watched remote branch or tag. A fresh clone qualifies through the remote tips. The registration then moves without a question.
- The check never downloads: Git runs with `GIT_NO_LAZY_FETCH=1`, so a partial clone answers from the objects it has. A partial clone with Git older than 2.44, which ignores that variable, counts as unverified.
- Otherwise Brainiac asks first and says why: the chosen folder is below the top of the working tree, the histories share no recorded commit, or nothing was recorded to compare with. The confirmation covers the working-tree root it was shown; if the folder resolves to another root by then, Brainiac asks again. Confirming a repository whose history is unrelated or unverified starts its activity over: the old Git directory's feed is dropped when no other registration uses it, and the new one starts silently.
- A folder already registered as another repository is refused with `CONFLICT`; remove one of the two registrations first. Merging two registrations is not supported.
- What moves along:
  - Every membership points at the new folder, and the member's name becomes the new folder name. In a discovered workspace the member counts as discovered when the new folder is the discovery root or directly inside the discovery folder, and as manual otherwise. A workspace's root repository that ends up anywhere other than its `discovery_root` stops being the root and stays an ordinary member.
  - **The folder that moved** is the highest folder that is gone among the old folder and those of its parents whose names the new path repeats: relocating `code/web` to `src/web` while `code` is gone means `code` became `src`. Nothing moves along when the old folder still exists, as when switching to a second clone.
  - Inside the folder that moved: a workspace `discovery_root`, missing registrations, and missing non-Git members move to the same relative path in the new folder when it exists. A repository moves only when that path is its working-tree root and the same repository; the others, including any Git cannot check, stay missing.
  - When a main checkout's Git directory is gone from its old place and the history is the same, its registered linked worktrees are pointed at the new location. They work again once `git worktree repair` has run; Brainiac does not run it.
  - Activity (baseline, feed, read state) follows the registration to the new Git directory when the history is the same, no other registration still uses the old Git directory, and none already uses the new one. When the new one is in use, the old feed is dropped once nothing uses it. When registrations remain on the old Git directory, its feed stays with them.
- A status observation that started before a relocation and finishes after it is discarded.
- Afterwards the moved registrations are refreshed and watched at their new folders. Branches that moved while a repository was missing arrive as ordinary activity events.
- Relocating writes nothing to any repository.

#### Presentation and Git boundaries

In the sidebar and the overview, a workspace with a root shows the root first, followed by its other members; discovered members are grouped under the discovery folder's name. A manual workspace shows a flat list. The overview has no layout-specific scope switch: users narrow it with the name/path and state filters and by opening one repository. Aggregate counts retain per-repository attribution; histories and diffs are never implicitly merged into one Git history.

Root-repository status is exactly what Git reports for the root. Member status is collected independently, including when the root ignores the discovery folder. If the root tracks a member as a gitlink/submodule or reports a nested directory as untracked, display that parent entry as its own observation; it is not a substitute for the member's detailed status. Do not add member file counts into the root's own dirty-file count.

Route working-tree notifications to the most specific registered repository root. Refresh an ancestor when its own tracked state or detected Git relationship may have changed; avoid a full parent status job on every child keystroke. Keep a separate lightweight watcher for workspace membership discovery. Treat each linked worktree's available project folders independently; do not assume a root worktree automatically contains every child checkout.

### Relationship to editor workspace files

Editor workspace files such as VS Code's `.code-workspace` often describe a similar grouping: a list of folders, some commented out, alongside editor-only settings such as `files.exclude` and task definitions. Reading, importing, watching, rewriting, or synchronizing such files is not a v0.1 requirement.

A discovered workspace with a root renders in the sidebar like this; a manual one omits the root and the group:

```text
Product
  product            root
  services/
    billing
    search
```

v0.1 recreates such a grouping by discovery from a folder, by manual selection, or both. Brainiac's selection is persisted in its own SQLite configuration; an editor's active or commented folder entries do not determine membership.

Editor display settings such as hiding folders or `.git` do not change Brainiac's tracking model. Each selected repository remains independently inspectable. Task or launch definitions found in such files introduce no launcher requirement and are never executed.

A `.code-workspace` convenience importer can be considered later if useful. It is outside v0.1 acceptance criteria, with no required ongoing synchronization.

### Single-repository viewer

A registered repository opens directly; workspace membership is optional. Keep tabs for Changes, History, and Branches/Tags, and remember the last tab per repository. A fresh repository with no commits shows an empty history while still showing staged/untracked files. Missing/inaccessible paths show an error with **Locate…** (Relocating a repository) or Remove registration; removing registration does not touch the working directory.

#### Changes and diffs

- Group staged, unstaged, untracked, and conflicted files. Show paths, change kinds, rename source/destination, and added/removed line counts with a small change bar (`git diff --numstat` for the index and the working tree). Groups fold; long folder names are shortened in the middle so the file name always shows.
- For tracked text, display a unified diff with line numbers, additions/deletions, and context. Separate HEAD-to-index and index-to-working-tree comparisons; the same file can appear in both groups. The comparison switch shows each side's counts and offers **Both** (HEAD to working tree, one combined patch).
- Every patch view offers Unified/Split layouts, changed-word highlights inside a modified line pair (computed in the frontend), **Ignore whitespace** (`git diff -w`), hunk position with previous/next hunk jumps, and a hunk header that stays pinned while scrolling.
- Untracked files use a bounded, read-only text preview labeled untracked. Conflicted files show conflict status and current contents; conflict resolution is later scope.
- Binary files, submodule changes, Git LFS pointers, symlinks, and oversized patches receive explicit summaries rather than misleading text diffs. Do not download LFS objects or traverse submodules automatically.
- Initial display limits: 1 MiB or 10,000 patch lines per file, whichever comes first. Mark truncation and offer Open in editor; never silently omit remaining content.
- Load only the selected patch. Revalidate/invalidate displayed working-tree diffs when repository state changes and discard obsolete requests after selection switches.
- Keep patch rendering read-only, virtualize long output, and escape source text.

Git supplies comparisons of working tree, index, and commits; disable external diff/text-conversion helpers and paginate/bound output. [Git diff documentation](https://git-scm.com/docs/git-diff)

#### History and commit details

- Default history is the current HEAD's reachable commits, newest/topologically ordered. Load 100 records per page, anchored to the selected ref's resolved commit ID so new commits do not shift an in-progress traversal unexpectedly.
- Show commit hash, subject, author, authored/committed time, parent IDs, and branch/tag decorations. Display full message in commit details.
- Offer ref selection and commit-message/hash filtering; an `author:` token in the filter limits by author (`git log --author`). Queries remain scoped to the selected repository/ref; indicate loading and cancellation.
- Each row shows the author's initials and name; the day heading stays pinned while scrolling. The commit's file list can be hidden to give the patch the full width, shows a +/− bar per file, and a file stepper ("1 / 7", `[` and `]`) moves through files.
- Selecting a commit loads its changed-file list; selecting a file loads its patch. Compare a normal commit with its parent, a root commit with an empty tree, and a merge commit with its first parent by default. Label the chosen parent and permit selecting another parent.
- Copy commit hash and relative file path. Display removed files and rename history correctly; opening a historical path is separate from opening its current working-tree file.
- A graphical branch-lane visualization and blame are future additions. A useful history list and parent links are sufficient for v0.1.

Use Git's history/object commands behind structured Rust DTOs rather than parsing terminal-decorated output. [Git log documentation](https://git-scm.com/docs/git-log), [Git show documentation](https://git-scm.com/docs/git-show)

#### Branches and tags

List local branches, remote-tracking branches, and tags, marking the current branch and upstream where present. Selecting a ref changes the history view without checking out the branch. Resolve refs to object IDs in Rust before comparison/history queries. [Git ref enumeration](https://git-scm.com/docs/git-for-each-ref)

- Sort by recent activity (default) or name. Branches whose tip is older than three months fold into their own group; remote-tracking branches fold per remote; tags sort newest version first. Long names keep their start and end, shortened in the middle.
- Compare every branch with the repository's default branch: the remote's `HEAD` target (such as `origin/main`), else a local `main` or `master`. Show "ahead/behind main" next to the upstream comparison (`%(ahead-behind:<base>)` on Git 2.41+, `git rev-list --left-right --count` otherwise).
- The selected ref's side panel separates "commits not on main" from the shared history.

### Multi-repository dashboard

Show repository name/path, branch or detached HEAD, staged/unstaged/untracked/conflicted file counts, last refresh, and error/stale status. Count unique changed paths separately from staged/unstaged groups so a doubly modified file is not counted as two files. Provide workspace totals, filters for dirty/conflicted/stale repositories, sorting by name or latest observed commit time, and Refresh all. One repository failure must not block the others. Deduplicate the All repositories view while allowing one repository to belong to several workspaces. Ahead/behind appears only when an upstream exists and reflects local refs as of the last fetch, which the dashboard shows per repository.

### Refresh strategy

Watching `.git/HEAD`, refs, and index alone misses changes to unstaged working files. Observe both Git metadata and working-tree changes, excluding ignored/generated trees from expensive recursive observation.

- Debounce repository invalidation for approximately 500 ms.
- Resolve worktree-specific and shared Git metadata directories explicitly.
- Reconcile on launch, wake, activation, and manual refresh.
- Use a slow safety refresh, proposed 60 seconds while the dashboard is visible, for missed events. Back off on expensive repositories and label stale data.
- If status exceeds a proposed 5-second timeout, preserve the last snapshot with a warning and retry option.
- Batch/coalesce event bursts; never run a status command per keystroke or per watcher callback.

v0.1 actions are inspect status/diffs/history/refs, copy hashes/paths, refresh, fetch, reveal in Finder, and open in editor. Knowledge/task associations arrive in v0.2. Fetching (below) is the only operation that writes to a repository, and automatic fetching happens only for workspaces that opted in. Branch checkout, commit, stash, pull, and automatic dev-server startup remain future, explicit actions.

### Fetching

Fetching updates remote-tracking refs (`refs/remotes/...`), the objects they need, and tags that point into the fetched history. It never touches the working tree, the index, local branches, `HEAD`, or the stash, so it cannot change anything the user is working on. It is still a write to `.git`, so it happens in exactly two cases:

- **Fetch now:** an explicit action on a repository (header button, menu) or on a workspace (every member). It fetches the remote of the current branch's upstream, else `origin`, else the only remote, with the remote's configured refspecs, keeping only those whose destination is under `refs/remotes/<remote>/`, or that copy a tag to the same name without forcing (a mirror refspec such as `+refs/heads/*:refs/heads/*` is dropped, and so is anything that could overwrite the user's own tags). When none is left, the standard `+refs/heads/*:refs/remotes/<remote>/*` is used.
- **Auto-fetch:** a per-workspace setting, **off by default**. While Brainiac runs, each Git directory that an auto-fetching workspace contains fetches only that workspace's watched branch patterns (`+refs/heads/<pattern>:refs/remotes/<remote>/<pattern>`) from `origin`, else the only remote, else the current branch's upstream remote, so a branch that tracks a fork does not change what is watched. It runs at most every 15 minutes (a global setting) and backs off exponentially after failures up to 6 hours.

Both use the same hardened invocation with explicit, checked refspecs, so a fetch never prunes, runs hooks, recurses into submodules, or writes outside remote-tracking refs and tags, whatever the user's configuration says (`docs/architecture.md`, Git).

- **Credentials:** a background process cannot answer prompts. Git runs with `GIT_TERMINAL_PROMPT=0` and `GCM_INTERACTIVE=never`; when the user has not configured `core.sshCommand`, SSH runs with `BatchMode=yes`. Credential helpers such as the macOS keychain keep working. Authentication failures are reported as "needs sign-in" with the recovery step (fetch once from a terminal or editor), never as a prompt. Hardware keys and password-manager SSH agents may still ask for approval on each fetch; the auto-fetch setting says so.
- **Deleted branches:** when the remote no longer has a branch an explicit refspec names, the fetch is retried without it instead of failing, and auto-fetch leaves it out for 24 hours (Fetch now forgets this), so missing default branches cost no extra connections.
- **Other Git processes:** before fetching, look for the lock files of what a fetch writes (`packed-refs.lock`, `shallow.lock`, the reftable lock, and `*.lock` under the remote's refs and the tags; a commit in progress does not block). A fresh lock means "busy": nothing is recorded and auto-fetch retries on its next turn, waiting a full interval after three busy attempts in a row. A lock older than ten minutes, or dated more than a minute in the future, is a leftover that also blocks the user's own Git commands; it is reported as an error that names the file.
- **Concurrency:** at most two fetches run at once. A Git directory never has two fetches in flight: a second request (such as Fetch all over a checkout and its linked worktree) waits for the first and shares its result.
- **Freshness:** a repository's "last fetched" time is the later of Brainiac's own last fetch and the modification time of `FETCH_HEAD` (written by the user's own fetches). The activity feed warns when a watched repository has not been fetched for two days.
- After a fetch every checkout of the Git directory is refreshed as after a watcher event, which updates ahead/behind and the activity feed. A fetch that had nothing to fetch reports no fetch time.

### Workspace activity

The Activity tab of a workspace answers "what did the team merge or release since I last looked?". It reads local refs only; news arrives when a fetch (the user's, their editor's, or Brainiac's) moves remote-tracking refs.

- **Watched refs** are per workspace: branch names or patterns matched against remote-tracking branches without their remote prefix (`main`, `develop`, `release/*`), and tag patterns (`v*`). `*` matches any run of characters; branch patterns become fetch refspecs, so they may contain one `*` at most. Patterns must be valid ref names (`git check-ref-format`), and `?`, `[`, `]`, `:`, `^`, `~`, `\`, spaces, and a bare `@` are rejected. Defaults: branches `main`, `master`, `develop`; tags `v*`.
- **Tracking is per Git directory** (`common_git_dir`), so a checkout and its linked worktrees share one set of tips and one feed. One tracking pass runs per Git directory at a time; it follows each status observation and is skipped cheaply when neither the ref files (their modification times) nor the watched patterns changed in the last five minutes (after that a full pass runs anyway, for filesystems with coarse modification times).
- A pass compares the tips of the refs any containing workspace watches with the stored baseline. The baseline remembers which patterns it covered: refs that start matching later (the first observation, a new pattern, a second workspace) join it silently, and so do the branches of a remote that is new to it (an added or renamed remote). Changing a workspace's watched patterns takes a pass right away from the last status, and a repository joining a workspace is refreshed, so branches created afterwards still arrive as news. Each moved ref becomes one event, at most twenty per pass, newest first by tag or commit date (the rest join the baseline):
  - **advanced:** the old tip is an ancestor of the new one; record the commit and merge counts, up to five newest commits, and the authors.
  - **rewritten:** history was replaced (force-push), or the old tip no longer exists; record how many commits were replaced and added. Shown in red. Any other failure while deciding (a timeout) fails the pass, which is retried, rather than reporting a rewrite.
  - **created:** a watched branch appeared.
  - **tagged:** a new tag matching a tag pattern, with the number of commits since the previous tag.
- Details are best effort: when they cannot be read, the event is recorded without them and the tips still advance, so one bad ref cannot stop the feed.
- **Conflict risk:** paths changed by an advanced range (`git diff --name-only old new`, bounded) that the user also changes in the working tree are listed on the event.
- **Drift:** when the current branch is behind its upstream after the move, or behind the watched branch it was forked from (the watched branch with the fewest commits unique to `HEAD`), the event says by how much.
- **Each workspace sees what it watches:** the feed, the unread count, Mark all as seen, and notifications use the workspace's own patterns, and only events observed after the workspace started watching the matching pattern, even when another workspace watching the same repository has more or older ones. Seen state belongs to the event, so an event both workspaces show is seen in both.
- **Unread:** the tab shows a divider between unread and seen events, Mark seen per event and Mark all as seen, and the sidebar shows the workspace's unread count.
- **Team pulse:** commits, merges, and releases on the watched refs in the last seven days, and the most active authors, counted from local refs. It is a separate request so the feed never waits for it; each Git directory is read again only when its ref files changed (the tracking fingerprint), otherwise its last reading is reused and filtered to the current seven days.
- **Let me know:** optional per workspace, all off except the conflict-risk warning: a macOS notification when a watched branch moves (at most one per repository and workspace per hour), a morning digest at 09:00 local time when there are unread events, and the conflict-risk warning on events.
- Events older than 90 days are pruned. Removing the last checkout of a Git directory deletes its tips and events; a Git directory no workspace watches loses its baseline, so watching it again starts silently.

## 5. Notes — v0.2

### The vault

- One vault: a folder of Markdown notes the user chooses (**Choose Folder…**) or creates (**Create a New Vault…**). Brainiac edits the `.md` files where they are; nothing is moved, converted, or imported, and the notes stay ordinary files usable in any editor.
- Supported notes are UTF-8 `.md` files on a local filesystem, up to 5 MiB for editing. Other files are listed with a clear message and **Open Externally**; a `.md` file over 5 MiB or not in UTF-8 is found in search by its name, not its text.
- A vault may itself be a Git repository, and it can be registered and tracked like any other. The rule that Brainiac never writes to a repository covers Git's own state: it never stages, commits, checks out, or runs hooks there. Saving a note is an edit the user makes through Brainiac, like saving it in any editor. Trash, drafts, and revision history live in Brainiac's data folder, never in the vault, so saving never leaves extra files behind.
- Brainiac scans the vault at startup, on wake, and when the watcher reports changes, and reconciles what it finds. A vault that cannot be read (an unmounted disk) is reported as unavailable, never treated as every note deleted.
- Choosing another vault folder later keeps the notes of the previous one as missing, with their tasks and links; choosing that folder again brings them back.

### Notes view

- A vault tree with folders, pinned and recent notes, and a filter (`/`); the editor; and a context panel. The context panel lists the note's linked repositories (each with its live state and Open / Open in Editor), its tasks, its backlinks, its unresolved links (with **Create**), and suggestions: a registered repository the note mentions can be linked with one click or dismissed. Brainiac never links anything by itself.
- The context panel can be hidden (`Option+Cmd+0`); when hidden, the header shows how many repositories and tasks the note has. It hides by itself when the window is too narrow for the editor.
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
- **To sort:** a new task without a planned date or deadline is *to sort* until it gets one or is marked **Sorted**. v0.2 has no Inbox: everything is created inside the app, and an inbox earns its place only when items arrive faster than they are sorted, which starts with v0.4's capture and imports. Quick notes go to an ordinary `Inbox/` folder in the vault.
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
