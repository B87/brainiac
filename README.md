# Brainiac

A keyboard-oriented macOS desktop app for programmers: a Git workspace tracker today, a personal second brain over time. Rust backend, Tauri v2 shell, React + TypeScript frontend. Everything runs locally; the app only ever reads your repositories.

**Status:** early development. The current release line (v0.1) is a read-only Git viewer and multi-repository tracker. See [`SPEC.md`](SPEC.md) for the full specification and roadmap.

## What works now

- Register local repositories and see branch, upstream ahead/behind, and staged / unstaged / untracked / conflict counts at a glance.
- Inspect staged and unstaged diffs with line numbers, bounded output, and binary / submodule / symlink / LFS detection.
- Browse paginated commit history anchored to a fixed commit, with message and hash filtering.
- Changes on disk are picked up by filesystem watchers and coalesced refreshes; nothing is polled per keystroke.
- Registrations, pins, and settings persist in SQLite with daily snapshot backups.

Brainiac never runs `checkout`, `commit`, `stash`, `fetch`, hooks, or anything from a workspace file. Inspection only.

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
src-tauri/      Rust crate: git.rs, db.rs, workspaces.rs, watcher.rs, commands.rs
src-tauri/migrations/   Ordered SQL migrations
src-tauri/tests/        Integration tests that build Git fixtures in temp dirs
scripts/                Release helpers (version check, release, publish)
docs/                   RELEASING.md
```

## License

To be confirmed before the first public release; the intended license is MIT OR Apache-2.0.
