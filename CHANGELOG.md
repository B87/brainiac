# Changelog

All notable changes to Brainiac are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). The section for a version is used
verbatim as the GitHub release notes and as the text shown by the in-app updater.

Write one short line per user-visible change, saying what the user notices.
Add it under Unreleased with the change; `scripts/release.sh` turns that section
into the release's section.

## [Unreleased]

- Brainiac installed from a release can set up and upgrade a run host: it downloads the host's run controller from its own release instead of needing a copy of the source code and a build in Docker.

## [0.7.0] - 2026-10-17

- Routines are saved runs that check something and leave a short report, a briefing. Run Now starts one; the Routines section lists them, each has its own page, and Today shows the ones with news.
- A routine keeps a memory that the agent and you can edit, and is shown its earlier briefings.
- A routine on a run host can have a schedule: once you approve it, the host runs it at its time with this Mac off or asleep, and Brainiac takes the briefing when it next reaches the host. The Schedule tab shows the host's time and time zone beside the time you set.
- Approving a schedule states what it permits in a table, and prepares the host when it needs it: it upgrades the host's controller, builds its image, and tests the agent there, step by step.
- A schedule says when its host missed a run, when the host has not answered since a run was due (Overdue), and when it cannot be reached (Host unreachable).
- Settings → Secrets keeps secrets of your own, such as a Sentry token, as a value or as a file for tools that read a key file. A run or routine chooses the secrets it gets (Access); a run host keeps a sealed copy of one when a run there first needs it, and a run on this Mac reads it when it starts.
- A secret a run was given is taken out of its briefing or explanation too, not only its conversation.
- A run or routine can add Python and the Google Cloud CLI to its image (Tools).
- Settings has a Runs heading with Agents, Run Hosts, Secrets, Code Sharing, and MCP (formerly Agent Access). Run Hosts is a table with what each host needs next. A host's page shows the machine, its health, the agents tested there, the routines it runs, the secrets it holds, and any schedule it runs for another Mac, which stays until you remove it. Adding a host is a page in steps.
- An agent is tested from its own page, on each host under Where it can run.
- A run host's page has a Health section: whether it answers, its uptime, load, memory, disk, Docker engine, and runs, with what to fix when something is wrong. Upgrade the host's controller to see it, and to use schedules and secrets.
- Settings → Code Sharing can allow or block a provider for an existing repository, or every repository in a workspace, before starting a run.
- Removing a repository also removes its routines, taking their schedules off their hosts first.
- Reaching a run host no longer fails when Brainiac asks it two things at once.
- Right-click a repository in the sidebar to open it in your editor or a terminal, reveal it in Finder, fetch, copy its path, or remove it.
- Right-clicking no longer shows the browser's menu (Reload, Cut, Spelling and so on); it still appears in text fields and the note editor.

## [0.6.4] - 2026-10-15

- In a pull request, opening a file or stepping with the keyboard no longer marks it reviewed: tick the file or use **Mark reviewed and next**. The toolbar shows how many files are reviewed, **Next unreviewed**, and **Unreviewed only**, and reviewed files stay where they are in the list.
- Files Changed steps by change with `J` / `K` (into the next or previous file at a file's end) and by file with `Shift+J` / `Shift+K`; `⌘P` finds a changed file, `T` focuses the file tree (and hides it when pressed again), and `?` lists the shortcuts. A file reopens where you left it.
- The bottom bar shows the shortcuts for what you are doing in a pull request (the diff, the file tree, a comment box, a dialog), dimming ones that cannot act yet.
- Every tab of a pull request keeps a summary line with the branches, the checks (failed ones open Checks at the failing check), and your drafts; failed checks are listed first.
- **Review Changes** is now **Finish review**, and its final button says **Post comments**, **Approve**, or **Request changes**.
- A long pull request description folds behind **Show full description**, with a link to the conversation and its thread count.
- A terminal: Terminal in the sidebar (⌘T) opens your shell in the repository you were looking at, and the same tabs show in a dock beside any view (⌘J); ⇧⌘T opens a new tab and ⌘1 … ⌘9 switch between them. New terminal is in a repository's Open in menu and a workspace's ⋯ menu; Settings → Terminal can run another program, such as herdr.
- A new query tab in Databases is now ⌥⌘T; ⌘T opens the terminal.

## [0.6.3] - 2026-10-12

- A run cancelled while its run controller was down no longer says "Cancel requested" forever once the controller is back and the run has ended.
- A repository's header is quieter: the repository and its branch read as one breadcrumb, and the branch always shows ahead, behind, and how fresh the last fetch is, with the upstream and Fetch now in its popover.
- Open in opens your editor in one click; its menu reveals the folder or copies its path. Fetch now and Refresh moved to the ⋯ menu.
- ⇧⌘F fetches now and ⇧⌘O opens the repository in your editor.
- ⌘B collapses the sidebar to a rail of icons instead of hiding it, so every section stays one click away.
- The repository and workspace tabs are shorter (Branches, PRs) and become one menu in a narrow window; PRs counts the pull requests waiting on your review.
- History marks when you are viewing another branch or tag, with Back to HEAD.
- The status bar names the right shortcut for the repository tabs (⌘1–5).
- All repositories has a Workspace menu in its header (All repositories › All workspaces) to show one workspace's repositories, or those in none, and a Workspace column; its totals moved into the header, so the list starts higher.
- With the sidebar collapsed, the title bar runs across the rail, so the window's buttons no longer straddle two bars.
- Drag the edge of the list in History and Changes, or of the ref history in Branches, to give it more or less room; Brainiac remembers each width.
- Brainiac works in half and three-quarter windows: tables drop their least useful columns, headers and toolbars shorten instead of overlapping or wrapping, and History, Changes, and a pull request's files fold their lists to a strip that slides over the patch.
- The patch toolbar fits a narrow pane with a View menu, and a commit's file tree becomes a menu beside the file stepper when there is no room for it.
- The commit list no longer scrolls sideways when a commit has long ref names.
- Databases' tabs no longer sit under the window's buttons when the sidebar is collapsed.
- The close button of All repositories' side panel now hides the panel, as ⌥⌘B does.
- In a long note, the cursor no longer ends up partly below the bottom edge after Page Up or Page Down, or out of view after switching to Live Preview.

## [0.6.2] - 2026-10-11

- A branch comparison marks binary files, as a run's result does.
- Runs lists the runs that wrote your explanations, tagged Explain, with a filter it remembers; each says what it explained and how deeply, whether its explanation is ready or failed, and opens it.
- Runs keeps its filters when you open a run and go back.

## [0.6.1] - 2026-10-10

- Text fields and menus in New Run, Explain, Code Sharing, and Settings → Explanations have a border and padding like the rest of the app.
- An explanation says how many concepts it left out because you know them, lists them, and lets you undo one.
- A concept you know can be edited: change its name or kind, and its old name stays with it.
- Concepts You Know can be sorted by newest, name, or kind and grouped by repository, and a repository's concepts can be forgotten at once.
- Concepts are labelled a protocol or standard (such as DKIM), a tool or service (such as git), or a technique (such as idempotency) instead of a "system tool"; concepts you already know as system tools become tools.
- Explanations leave out a concept you know even when the agent words it a little differently, such as "Closure" for "Closures".

## [0.6.0] - 2026-10-09

- Explain asks an agent to explain a commit, a branch compared with the default branch, a run's result, or a pull request: a summary, the files in reading order, notes beside the lines, the ideas the change relies on, and questions to check yourself, each claim checked against the code before it is shown.
- New run and Explain ask once, for each repository and provider, whether its code may be sent, in place of the agreement on each agent's page; Settings → Code Sharing lists the answers.
- The Explain dialog shows what an explanation usually takes in time and cost, and a finished or failed one shows what it took.
- A branch's explanation stays with the branch: notes move with their lines, and those whose lines changed are marked out of date, with Re-explain.
- Branches & tags shows a branch's changes against the default branch as one patch.
- A pull request's Files Changed shows its explanation beside the review: notes marked as staying on this Mac, the files in reading order with your viewed marks, and the Overview says whether it is explained. A pull request and its branch share one explanation.
- Settings → Explanations sets the agent, the model and time limit for each depth, your level by language, the concepts you know, and lists stored explanations to delete.
- A run's header shows its cost when the agent reports one.

## [0.5.0] - 2026-10-08

- Runs can use OpenCode as well as Claude Code, with an Anthropic, OpenAI, or OpenRouter API key and the model you choose; Settings → Agents sets up and tests each agent and provider on its own page, and New run asks which one to use.
- Installing, upgrading, building the image on, and testing a remote host now show each step as it happens, with the build's output and how long it has taken; an install or upgrade can be cancelled until it changes the host, and a failure says at which step and offers Try again.
- Upgrading a host with a live run now waits for the run to end instead of refusing, and later upgrades build the run controller much faster.
- Settings → Agents lists This Mac and your remote hosts under Run hosts, each with its own page; Add host… walks through the key, the install, the image, and the test in one go.
- The status bar and New run show a host that is being installed or upgraded, with a link to its progress, and Brainiac says when it ends.
- Start run opens the run straight away and shows its start as steps while the repository is copied and sent to the host; it can be cancelled meanwhile.
- A run's edits show what changed, as removed and added lines, in the conversation and in its permission requests.
- Changes so far shows a live run's working tree in the diff viewer, read after each turn and on Refresh, while the agent keeps working.
- Copy branch command gives a Git command that brings a run's reviewed result into your repository as a new branch.
- A run whose image was removed from the engine, for instance by a cleanup of unused images on a host, fails at once and says to build the image again, instead of offering a collection that cannot work.
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

[Unreleased]: https://github.com/B87/brainiac/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/B87/brainiac/compare/v0.6.4...v0.7.0
[0.6.4]: https://github.com/B87/brainiac/compare/v0.6.3...v0.6.4
[0.6.3]: https://github.com/B87/brainiac/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/B87/brainiac/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/B87/brainiac/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/B87/brainiac/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/B87/brainiac/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/B87/brainiac/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/B87/brainiac/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/B87/brainiac/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/B87/brainiac/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/B87/brainiac/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/B87/brainiac/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/B87/brainiac/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/B87/brainiac/releases/tag/v0.1.0
