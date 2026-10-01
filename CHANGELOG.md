# Changelog

All notable changes to Brainiac are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). The section for a version is used
verbatim as the GitHub release notes and as the text shown by the in-app updater.

Write one short line per user-visible change, saying what the user notices.
Add it under Unreleased with the change; `scripts/release.sh` turns that section
into the release's section.

## [Unreleased]

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

[Unreleased]: https://github.com/B87/brainiac/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/B87/brainiac/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/B87/brainiac/releases/tag/v0.1.0
