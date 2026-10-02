# Changelog

All notable changes to Brainiac are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). The section for a version is used
verbatim as the GitHub release notes and as the text shown by the in-app updater.

Write one short line per user-visible change, saying what the user notices.
Add it under Unreleased with the change; `scripts/release.sh` turns that section
into the release's section.

## [Unreleased]

- Locate… points a repository whose folder moved at its new place, keeping its workspaces, pin, and activity; Rescan suggests the move when a member was renamed.

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

[Unreleased]: https://github.com/B87/brainiac/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/B87/brainiac/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/B87/brainiac/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/B87/brainiac/releases/tag/v0.1.0
