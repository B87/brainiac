# Changelog

All notable changes to Brainiac are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). The section for a version is used
verbatim as the GitHub release notes and as the text shown by the in-app updater.

## [Unreleased]

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

[Unreleased]: https://github.com/B87/brainiac/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/B87/brainiac/releases/tag/v0.1.0
