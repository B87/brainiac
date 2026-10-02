# Brainiac — Specification

**Updated:** 2 October 2026  
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

**v0.1 — Git viewer and single/multi-repository tracker.** This is the user's current priority. It must work as a complete local Git inspection tool before notes, tasks, content imports, or AI are introduced.

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
| External content imports | v0.3 | Paste, bookmarks, Markdown copies, articles, `.eml`, provenance and duplicate handling |
| Global capture window and Inbox | v0.3 | System shortcut, floating capture, Inbox triage of captured and imported items, shared backend state |
| Authenticated import adapters | v0.3.x | Selected Jira issues/mail messages; provider choice and video transcript acquisition validated separately |
| Semantic search | v0.4 | Optional local embeddings and hybrid retrieval |
| Grounded AI answers | v0.5 | Citation-backed local RAG |
| Remote PR/CI tracking, other Git mutations, sync, plugins | Later | Separate features after local viewing is useful |

v0.1 requires a usable local Git binary. Detect it on startup and provide a clear setup message when absent; do not silently install developer tools. Core Git viewing works offline and requires no Markdown vault, Ollama instance, remote-service account, or elevated macOS permissions. Ahead/behind information reflects existing local refs and may be stale relative to the remote server until someone fetches: the user, their editor, or Brainiac's Fetch now and opt-in auto-fetch (section 4, Fetching). Fetching is the only operation that writes to a repository, and it touches remote-tracking refs and objects only.

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
| `Cmd+K` | Repository/workspace palette |
| `Cmd+O` | Add/open a local repository |
| `Cmd+R` | Refresh selected repository or workspace |
| `Cmd+1` … `Cmd+3` | Repository tabs: Changes, History, Branches & tags |
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
- Stay within the discovery folder. Symlinked external repositories require explicit selection. Deeper descendants require explicit addition; do not crawl arbitrary nested dependency trees.
- Brainiac owns its tracked-repository selection independently of editor configuration. Any workspace can gain manually added repositories or drop members, whichever way it was created.
- A discovered workspace stores the selected folder as `discovery_root` and the scanned folder relative to it as `discovery_path` (absent when the selected folder itself is scanned), so Rescan works whether or not the selected folder is a repository.

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

A registered repository opens directly; workspace membership is optional. Keep tabs for Changes, History, and Branches/Tags, and remember the last tab per repository. A fresh repository with no commits shows an empty history while still showing staged/untracked files. Missing/inaccessible paths show an error with Relocate or Remove registration; removing registration does not touch the working directory.

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
