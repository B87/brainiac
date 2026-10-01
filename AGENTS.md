# Brainiac — working notes for Claude Code

Brainiac is an open-source macOS desktop app: Rust backend inside a Tauri v2 shell, React + TypeScript + Vite frontend. What the app does is `SPEC.md`; how it is built is `docs/architecture.md`; milestones and later releases are `docs/roadmap.md`.

## Current scope: M0 then v0.1 only

Build the Git viewer and repository tracker. Nothing from v0.2 onward (notes, vault, tasks, FTS5, imports, global capture, embeddings, AI) is in scope. Do not add dependencies or tables for later releases.

Read `SPEC.md` and `docs/architecture.md` for v0.1 work. Open `docs/roadmap.md` only for planning or milestone checklists.

Docs lifecycle: write a behavior change in `SPEC.md` first (or in the same commit as the code); update `docs/architecture.md` in the same commit as the code it describes; add decisions at the end of its Decisions section and never edit an accepted one; when a release starts, move its design from `docs/roadmap.md` into `SPEC.md`.

## Decisions already made (do not re-open; the full log is `docs/architecture.md`, Decisions)

- Rust↔TS types via `ts-rs`, exported to `src/lib/generated/` by a Rust test and committed. Hand-written `invoke` wrappers in `src/lib/ipc.ts`.
- `pnpm` for the frontend and as the shared command entry point through `package.json` scripts; no Taskfile. `rustup` stable pinned in `rust-toolchain.toml`.
- Git is invoked as the system CLI with argument arrays, never through a shell. Minimum Git 2.30.
- Default editor: VS Code `code` CLI, configurable.
- v0.1 backup = SQLite snapshots only.
- DTO shapes are defined only in `src-tauri/src/models.rs` and exported to `src/lib/generated/`; the docs do not copy them. Describe any behavior change in `SPEC.md` before changing a shape.

## Hard rules

- This is a public, general-purpose project. Never commit paths, repository names, workspace files, or settings from the maintainer's machine or employer. Examples and fixtures use generic names.
- The app never writes to a repository: no checkout, commit, stash, pull, hook execution, or index changes. The one exception is fetching, and only as `SPEC.md` (Fetching) and `docs/architecture.md` (Git, Fetch invocation) define it: the explicit Fetch now action and opt-in per-workspace auto-fetch (off by default), with the hardened invocation that updates remote-tracking refs and tags only. Everything else is inspection.
- Keep business logic out of Tauri command handlers so it is testable with `cargo test` without a WebView.
- Git test fixtures are built by the tests themselves (`git init` in a temporary directory), not checked in.
- The updater signing private key lives outside the repository (`~/.tauri/brainiac.key`, CI secret `TAURI_SIGNING_PRIVATE_KEY`). Never read it into a file in the repo or print it. Only the public key belongs in `src-tauri/tauri.conf.json`.
- The app version is declared only in `src-tauri/Cargo.toml`; Tauri reads it from there. Do not add a `version` to `package.json` or `tauri.conf.json` (`scripts/check-version.sh` fails if one appears). Bump it only through `scripts/release.sh`.

## Commands

- `pnpm install` then `pnpm tauri dev` to run; `pnpm tauri build` for a packaged app.
- Run `pnpm check` from the repository root before finishing: version agreement, Rust formatting, Clippy (all targets, warnings as errors), Rust tests, Biome, TypeScript, and frontend tests.
- Frontend: `pnpm check:web` for Biome lint, formatting, and import checks; `pnpm check:fix` for safe fixes; `pnpm format` to format. Generated DTOs in `src/lib/generated/` are excluded from Biome; regenerate them with Rust tests rather than editing them.
- Rust: `pnpm format:rust` to format; `pnpm format:rust:check`, `pnpm lint:rust`, and `pnpm test:rust` for individual checks. Rust tests also regenerate TypeScript bindings; commit them with the corresponding Rust changes.
- `pnpm typecheck` and `pnpm test` for frontend types and tests; `pnpm build` for the frontend build only.
- `pnpm run` lists available scripts.
- Releases: `pnpm release X.Y.Z` (tag + push, CI builds a draft), then `pnpm release:publish vX.Y.Z`. These wrap the scripts in `scripts/`; `pnpm version:check` verifies version agreement. Process in `docs/RELEASING.md`. Every user-visible change gets one short line under `## [Unreleased]` in `CHANGELOG.md`, saying what the user notices, not how it was done; internal changes get none. Do not create version sections by hand: `scripts/release.sh` turns Unreleased into the release's section and updates the links.

The maintainer is new to Rust. When introducing a Rust concept for the first time in a change (ownership, lifetimes, traits, `Result`/`?`, `Arc<Mutex<_>>`), add a one-line comment explaining why it is used there.
