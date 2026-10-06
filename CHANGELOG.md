# Changelog

All notable changes to Brainiac are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). The section for a version is used
verbatim as the GitHub release notes and as the text shown by the in-app updater.

Write one short line per user-visible change, saying what the user notices.
Add it under Unreleased with the change; `scripts/release.sh` turns that section
into the release's section.

## [Unreleased]

- A run's replies are shown as Markdown, with headings, lists, and code blocks.
- Runs can execute on an approved Linux host over SSH, and keep working while this Mac sleeps. Settings → Agents installs that host's run controller, and asks before it trusts a new or changed host key.
- Settings → Agents, for runs of Claude Code in a container: choose the Docker engine, add an API key from the Keychain, a command, or a variable, and build the run image from a Dockerfile you can read.
- Runs: hand a repository's commit to Claude Code in a container on this Mac (New run…, ⌥⌘N), follow the conversation live, answer its permission requests or let it act, send follow-up prompts, and cancel or finish; a run keeps going while Brainiac is closed.
- A finished run's working tree, uncommitted edits included, comes back as Changes to review in the diff viewer, with Copy patch and Save patch…; new files the repository's ignore rules leave out are listed and can be added.
- Settings → Agents has a Test that starts a short run, sends a prompt, cancels, and collects; runs can start once it passes.
- Runs lists what needs you first (a question, a session ready for your prompt, or work waiting for a decision), and filters by state, repository, and host.
- Ended runs are removed after 30 days unless their work still waits on you; Delete run… removes one now.

## [0.4.0] - 2026-10-05

- Databases: save connections to SQLite files and PostgreSQL servers, with passwords in the Keychain, and query them in tabs with completion, a fast result grid, an inspector, Explain, Copy, and Export.
- Connections are read only unless you allow writes; on Production, writes are turned on per tab and run in a transaction you commit or roll back.
- Saved queries with `:name` parameters, found in ⌘K and run in one step, and a searchable history of what ran on each connection.
- Health for a PostgreSQL server: connections, sessions, locks, the slowest queries, the largest tables, and memory and CPU from Docker or Google Cloud SQL.
- Changes lists each untracked file inside a new folder, instead of only the folder.
- Databases go easier on the server: a schema change in a writable tab gives up after 5 seconds waiting for a lock, Production ends an idle transaction after 5 minutes, and Health pauses while the window is hidden.
- An account's token or a connection's password can come from a command, such as `gh auth token` or `op read`, or an environment variable, instead of the Keychain.
- Settings → Secrets shows where each token and password comes from, and lets you allow a restored source, refresh a secret, or retry a Keychain cleanup that failed.
- Accounts has a Test button that checks a token before you save it.
- A restored backup no longer reads any token or password until you allow its source.

## [0.3.1] - 2026-10-03

- New app icon: a white brain on Brainiac blue, with a transparent background.
- The sidebar hides and shows with ⌘B, and the side panel on the right with ⌥⌘B. Brainiac remembers both.
- Settings → General: pick VS Code, Cursor, Warp, or ChatGPT for Open in Editor, or set up any other program under Custom.

## [0.3.0] - 2026-10-03

- Agent access: Claude Code and other MCP agents can search your notes, read Today and your tasks, and, if you allow it in Settings, create notes and tasks and link notes to repositories. Off by default.
- A Claude Code plugin adds Brainiac to Claude Code in one step, with a skill for planning your day, tracking work as tasks, and writing up decisions as notes.
- The cursor stays in view when images above it finish loading as you move through a note.
- Settings → Accounts: add a GitHub or Bitbucket Cloud token for pull requests. It is kept in the macOS Keychain, and a token that cannot write can be saved as read-only.
- Pull requests: a workspace's Pull requests tab (⌘3) lists the open pull requests of its repositories on GitHub and Bitbucket Cloud, grouped by repository, with filters for what needs your review, yours, others, and drafts. Off per workspace until turned on there.
- Each repository's Pull requests tab (⌘5) shows its own, the checked-out branch's first, plus merged and closed ones of the last 30 days, and can point a fork at its upstream.
- Opening a pull request shows its overview and merge checklist, its conversation, the files it changes, its checks, and a side panel with the local checkout and the repository's notes.
- Files Changed shows each file's diff and marks it viewed; "Since your review" shows what arrived after the commit you last reviewed.
- Review from Brainiac: comment, reply in a thread, resolve or reopen it, and comment on a line. Line comments stay as drafts until Review Changes sends them with a summary and a verdict.
- Merge a pull request from its Overview once its checklist is complete. The confirmation names the commit it merges, and nothing is merged if new commits arrived.
- The sidebar shows how many pull requests wait on your review in each workspace, and opening the workspace from there goes to them.
- Settings (⌘,) is now a full page, and Back returns to where you were. It also sets the editor, how often repositories refresh and auto-fetch, and how large a diff is shown.

## [0.2.0] - 2026-10-02

- Notes: choose a folder of Markdown files as your vault and edit them where they are, in Live Preview or Source, with autosave, version history, and a trash outside the vault.
- A note changed in another editor reloads by itself; edits never overwrite a newer version on disk, and a conflict offers Compare…, Reload from Disk, and Save Draft as Copy.
- Notes keep their identity, tasks, and links when they are renamed or moved outside Brainiac, including by a `git pull`.
- Links between notes, wikilinks, backlinks, and unresolved links you can create with one click.
- A note's file is renamed after its title when you change it, unless the file was named differently or other notes link to it; then the header offers Rename File to Match Title….
- Tasks with a status, planned date, deadline, and a linked note and repository; Today shows what is due or planned, with the repositories in today's work.
- Link notes to repositories; each repository gains a Notes tab (⌘4).
- ⌘K searches notes and tasks by keyword, including code identifiers, alongside repositories and workspaces.
- Export everything to a folder and restore it on another Mac, with tasks and links intact.

## [0.1.3] - 2026-10-02

- Locate… points a repository whose folder moved at its new place, keeping its workspaces, pin, and activity; Rescan suggests the move when a member was renamed.
- After going back to an older version, Brainiac says its data was saved by a newer version and where the snapshots are, instead of running on it.

## [0.1.2] - 2026-10-02

Workspaces with an activity feed of what your team merged, Fetch now and opt-in auto-fetch, and a redesigned Git viewer.

### Added

- Workspaces: group repositories by picking them, or by choosing a folder and the repositories inside it; a folder that is itself a repository becomes the root.
- Workspace dashboards with totals, filters, a preview panel for the selected repository, and a notice when new repositories appear in the folder.
- Pin repositories and workspaces to the sidebar.
- Branches & tags shows each ref's latest commit, its age, and how far it is ahead of or behind its upstream, with a history preview.
- Fetch: update a repository, or every repository in a workspace, from its remote; only remote-tracking branches and tags change.
- Optional auto-fetch per workspace (off by default) of the watched branches, every 15 minutes while Brainiac runs.
- Workspace Activity tab: what was merged, force-pushed, or tagged on the watched branches since you last looked, with a warning when it touches files you are changing or leaves your branch behind.
- The sidebar shows how many unread updates each workspace has; macOS notifications and a morning digest can be turned on per workspace.
- A weekly team pulse: commits, merges, releases, and the most active people on the watched branches.
- Changes shows added and removed lines per file, folds its groups, and offers a combined Both comparison for files that are staged and unstaged.
- Diffs offer a split layout, changed-word highlights, Ignore whitespace, and hunk navigation with the current hunk header pinned.
- History shows each commit's author, filters by `author:name`, can hide the file list, and steps through a commit's files.
- Branches & tags sorts by recent activity, compares every branch with the default branch, folds branches untouched for three months, and lists the commits that are not on main.
- Keyboard shortcuts: J and K move through lists, [ and ] through files, N and P through hunks, ⌘1 to ⌘3 switch tabs, and / focuses the filter.

### Changed

- New layout: the window's title bar holds the repository switcher, tabs, branch, and actions, and the side inspector is gone.
- History shows commits grouped by day next to the commit's files and patch; the branch or tag being shown is picked from a menu.
- Changes groups files with their folder, and a file that is both staged and unstaged switches between the two comparisons.
- Diffs can wrap long lines, and "Open at line" opens the editor at the first change.
- Secondary text and status colors have more contrast in both appearances, and conflicted or missing repositories are marked by shape as well as color.
- Reloading keeps the current content on screen with a thin progress line instead of a "Loading…" placeholder.

## [0.1.1] - 2026-10-01

Changed files and patches in commit details, a working Branches tab, and smooth scrolling for long diffs.

### Added

- Commit details list changed files with line counts; select one to see its patch.
- Merge commits can be compared with either parent.
- Commit details show the full message and the committer.
- Branches tab with local branches, remote branches and tags; select one to see its history.
- "Copy path" in the diff header.

### Changed

- Long diffs scroll smoothly.

### Fixed

- Flickering in the Changes and History tabs.
- Comparing a merge commit with its second parent showed the first parent.
- Renamed files in a commit showed the whole file as added.

### Known limitations

- Not signed with an Apple Developer certificate; the first launch needs right-click > Open (see the README).

## [0.1.0] - 2026-10-01

First public build: a read-only Git viewer and multi-repository tracker for macOS.

### Added

- Track local repositories, including linked worktrees, in a sidebar and an overview with dirty, conflict and stale filters.
- Branch, ahead/behind and change counts per repository, refreshed automatically.
- Changes tab with staged and unstaged diffs; binary, submodule, symlink and LFS files are labelled.
- History tab with message and hash filtering.
- Open in editor and Reveal in Finder.
- Command palette (⌘K) and native macOS menus.
- Local database with daily backups.
- In-app updates from GitHub releases.

### Known limitations

- Long diffs can be slow to render.
- Commit details show metadata only.
- The Branches tab is a placeholder.
- Not signed with an Apple Developer certificate; the first launch needs right-click > Open (see the README).

[Unreleased]: https://github.com/B87/brainiac/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/B87/brainiac/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/B87/brainiac/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/B87/brainiac/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/B87/brainiac/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/B87/brainiac/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/B87/brainiac/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/B87/brainiac/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/B87/brainiac/releases/tag/v0.1.0
