# Changelog

All notable changes to Brainiac are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). The section for a version is used
verbatim as the GitHub release notes and as the text shown by the in-app updater.

## [Unreleased]

## [0.1.1] - 2026-10-01

Completes the Git viewer: changed files and patches in commit details, a working Branches tab, and smooth scrolling through long diffs.

### Added

- Commit details list the changed files with their change kind, renames, and added/removed line counts. Selecting a file shows its patch for that commit.
- Merge commits can be compared with any parent; the first parent is the default. Root commits compare with an empty tree.
- Commit details show the full message and the committer when it differs from the author.
- The Branches tab lists local branches, remote-tracking branches, and tags, marking the current branch and its upstream. Selecting one shows its history without checking anything out; "Show HEAD" returns to the current branch.
- "Copy path" in the diff header.

### Changed

- Long diffs are virtualized: only the lines near the viewport are rendered, so patches of thousands of lines scroll smoothly. Diffs under 1,000 lines still render in full so they can be selected and copied in one go.

### Fixed

- The Changes and History views flickered and `git status` ran continuously while a repository was open: loading the Changes tab refreshed status, which announced a change, which reloaded the tab. Views now reload only when the repository actually changed.
- Comparing a merge commit with its second parent compared with the first parent instead.
- A renamed file's patch in a commit showed the whole file as added instead of the changes relative to its old path.

### Known limitations

- Builds are not signed with an Apple Developer certificate yet, so the first install needs the right-click > Open workaround (see the README).

## [0.1.0] - 2026-10-01

First public build: a read-only Git viewer and multi-repository tracker for macOS.

### Added

- Register local repositories, including linked worktrees, and keep them in a sidebar and overview table with dirty / conflict / stale filters.
- Per-repository status: branch, upstream ahead/behind, staged, unstaged, untracked and conflicted counts, refreshed by filesystem watchers, on window focus, and on a timer.
- Changes tab with staged and unstaged diffs, line numbers, rename detection, and detection of binary, submodule, symlink and LFS entries. Oversized diffs are truncated, never loaded whole.
- History tab with paging anchored to a fixed commit, message and hash filtering, and commit metadata.
- Open in editor (VS Code by default, configurable) and Reveal in Finder.
- Command palette (⌘K) to switch repositories, native macOS menus, keyboard navigation.
- SQLite persistence with ordered migrations and daily snapshot backups.
- In-app updates from GitHub releases, with signed artifacts and a "Check for Updates…" menu item.

### Known limitations

- Diff output is not virtualized; very long diffs can be slow to render.
- Commit details show metadata only; changed-file lists arrive in a later 0.1.x release.
- The Branches tab is a placeholder.
- Builds are not signed with an Apple Developer certificate yet, so the first install needs the right-click > Open workaround (see the README).

[Unreleased]: https://github.com/B87/brainiac/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/B87/brainiac/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/B87/brainiac/releases/tag/v0.1.0
