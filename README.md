# Brainiac

A keyboard-oriented macOS desktop app for programmers: a Git workspace tracker today, a personal second brain over time. Rust backend, Tauri v2 shell, React + TypeScript frontend. Everything runs locally; the app reads your repositories and, only when you ask it to, fetches.

**Status:** early development. The released line (v0.1) is a read-only Git viewer and multi-repository tracker; v0.2, not released yet, adds notes, tasks, Today, and keyword search. See [`SPEC.md`](SPEC.md) for what it does, [`docs/architecture.md`](docs/architecture.md) for how it is built, and [`docs/roadmap.md`](docs/roadmap.md) for what comes next.

## What works now

- Register local repositories and see branch, upstream ahead/behind, and staged / unstaged / untracked / conflict counts at a glance.
- Inspect staged and unstaged diffs with line numbers, bounded output, and binary / submodule / symlink / LFS detection.
- Browse paginated commit history anchored to a fixed commit, with message and hash filtering.
- Changes on disk are picked up by filesystem watchers and coalesced refreshes; nothing is polled per keystroke.
- Workspace Activity: see what was merged or tagged on your team's watched branches since you last looked, with warnings when it touches files you are changing.
- Registrations, pins, and settings persist in SQLite with daily snapshot backups.
- v0.2 (unreleased): a vault of ordinary Markdown notes edited where they are, with Live Preview, autosave, conflict handling, version history, and a trash outside the vault; tasks with dates and Today; links between notes, tasks, and repositories; ⌘K keyword search across notes and tasks; export and restore.

Brainiac never runs `checkout`, `commit`, `stash`, `pull`, hooks, or anything from a workspace file. The one write it can make is a fetch, which updates remote-tracking branches and tags only: when you press Fetch now, or on a schedule for a workspace where you turned auto-fetch on (off by default).

## Install

Download the `.dmg` from the [latest release](https://github.com/B87/brainiac/releases/latest), open it and drag Brainiac to Applications. The build is universal (Apple Silicon and Intel).

Or let a script do it, including the Gatekeeper step below:

```sh
curl -fsSL https://raw.githubusercontent.com/B87/brainiac/main/scripts/install.sh | bash
curl -fsSL https://raw.githubusercontent.com/B87/brainiac/main/scripts/install.sh | bash -s -- v0.1.0   # a specific tag
```

From a checkout, `scripts/install.sh [tag]` does the same, and `scripts/install.sh --dmg <file>` installs a local build.

Releases are not yet signed with an Apple Developer certificate, so macOS blocks the first launch. Right-click `Brainiac.app` in Applications, choose **Open**, and confirm once. Alternatively:

```sh
xattr -d com.apple.quarantine /Applications/Brainiac.app
```

After that, Brainiac updates itself: it checks the latest GitHub release shortly after launch and shows a banner when a newer build exists. **Brainiac > Check for Updates…** runs the check on demand. Updates are verified against a signing key embedded in the app before they are installed.

## Use with Claude Code

Turn on **Settings → Agent access** (off by default), then, in Claude Code:

```text
/plugin marketplace add B87/brainiac
/plugin install brainiac@brainiac
```

The plugin connects Claude Code to Brainiac and adds a skill for planning your day, tracking work as tasks, and writing up decisions as notes. It expects Brainiac in `/Applications`; for an app elsewhere, use the `claude mcp add` command that Settings shows instead. Other MCP clients run `<the app>/Contents/MacOS/brainiac mcp`.

## Requirements

- macOS 13 or newer.
- Git 2.30 or newer on your `PATH` (Xcode Command Line Tools are enough).
- For development: Rust stable via `rustup`, Node 22, `pnpm`.

## Development

```sh
pnpm install
pnpm tauri dev        # run the app
pnpm tauri build      # packaged .app and .dmg in src-tauri/target/release/bundle
```

Development commands:

```sh
pnpm check           # version agreement, Rust checks/tests, Biome, TypeScript, frontend tests
pnpm check:web       # frontend lint, formatting, and import organization
pnpm check:fix       # safe Biome fixes and formatting
pnpm format          # format the frontend
pnpm format:rust     # format Rust
pnpm test            # frontend tests
pnpm test:rust       # Rust tests and TypeScript binding generation
```

`cargo test` also regenerates the TypeScript bindings in `src/lib/generated/` from the Rust DTOs; commit them with the Rust change that produced them. Run `pnpm run` to list all available scripts.

## Releasing

Tag-driven: `scripts/release.sh X.Y.Z` bumps the version, tags and pushes; GitHub Actions builds, signs and drafts the release; `scripts/publish-release.sh vX.Y.Z` publishes it, which is what makes installed apps offer the update. The full process, including the one-time secrets setup, is in [`docs/RELEASING.md`](docs/RELEASING.md).

## Layout

```
src/            React frontend (components/, lib/ipc.ts, lib/generated/)
src-tauri/      Rust crate: git.rs, db.rs, workspaces.rs, watcher.rs, commands.rs;
                v0.2: vault.rs, notes.rs, index.rs, tasks.rs, backup.rs
src-tauri/migrations/   Ordered SQL migrations (index/ and history/ for the v0.2 files)
src-tauri/tests/        Integration tests that build Git fixtures in temp dirs
scripts/                Release helpers (version check, release, publish)
docs/                   RELEASING.md
```

## License

To be confirmed before the first public release; the intended license is MIT OR Apache-2.0.
