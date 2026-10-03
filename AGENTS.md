# Brainiac — working notes for Claude Code

Brainiac is an open-source macOS desktop app: Rust backend inside a Tauri v2 shell, React + TypeScript + Vite frontend. What the app does is `SPEC.md`; how it is built is `docs/architecture.md`; milestones and later releases are `docs/roadmap.md`.

## Current scope: v0.3 — pull requests

v0.1 (the Git viewer and repository tracker) shipped as 0.1.3 and v0.2 (vault, notes, tasks, Today, keyword search, links, export/restore) as 0.2.0; both are maintained, and v0.2.x (agent access over MCP, `SPEC.md` section 9) is merged and awaits release. Build v0.3: GitHub and Bitbucket Cloud accounts, per-workspace pull request tracking, the workspace's and each repository's Pull requests tab, the pull request overview, files changed, review, and merge (`SPEC.md` section 10; `docs/architecture.md`, Pull requests — v0.3). Ship read-only first, then the review writes, merge last. Nothing from v0.3.x onward (a daily pull request view, agent tools for pull requests, imports, global capture, embeddings, AI) is in scope. Do not add dependencies or tables for later releases.

Read `SPEC.md` and `docs/architecture.md` for v0.3 work. Open `docs/roadmap.md` only for planning or milestone checklists.

Docs lifecycle: write a behavior change in `SPEC.md` first (or in the same commit as the code); update `docs/architecture.md` in the same commit as the code it describes; add decisions at the end of its Decisions section and never edit an accepted one; when a release starts, move its design from `docs/roadmap.md` into `SPEC.md` and `docs/architecture.md`.

## Decisions already made (do not re-open; the full log is `docs/architecture.md`, Decisions)

- Rust↔TS types via `ts-rs`, exported to `src/lib/generated/` by a Rust test and committed. Hand-written `invoke` wrappers in `src/lib/ipc.ts`.
- `pnpm` for the frontend and as the shared command entry point through `package.json` scripts; no Taskfile. `rustup` stable pinned in `rust-toolchain.toml`.
- Git is invoked as the system CLI with argument arrays, never through a shell. Minimum Git 2.30.
- Default editor: VS Code `code` CLI, configurable.
- v0.1 backup = SQLite snapshots only.
- DTO shapes are defined only in `src-tauri/src/models.rs` and exported to `src/lib/generated/`; the docs do not copy them. Describe any behavior change in `SPEC.md` before changing a shape.
- v0.2 storage: the vault's Markdown files are the source of truth for notes; `brainiac.db` holds what cannot be rebuilt; `index.db` holds everything derived from the vault and can be deleted; `history.db` holds revisions and drafts (`docs/architecture.md`, Storage layout).
- Every write goes through a domain service, which emits the committed change event; task writes carry the expected version.
- Agent access (v0.2.x): MCP served by `rmcp` over a Unix socket closed to other users, reached through `brainiac mcp` (the app's own executable as a stdio byte pipe). MCP tool shapes live in `src-tauri/src/mcp/tools.rs`, not `models.rs`. Tools call the domain services; none deletes, touches Git, or changes settings.
- Pull requests (v0.3): a provider-neutral `PullRequestService` over one `ForgeAdapter` per provider, HTTP only in Rust (`reqwest` with rustls), tokens only in the Keychain (`security-framework`), a deletable cache in `forge.db`. Writes to GitHub and Bitbucket happen only on an explicit user action; review, approve, and merge carry the expected head commit.

## Hard rules

- This is a public, general-purpose project. Never commit paths, repository names, workspace files, or settings from the maintainer's machine or employer. Examples and fixtures use generic names.
- The app never writes to a repository: no checkout, commit, stash, pull, hook execution, or index changes. The one exception is fetching, and only as `SPEC.md` (Fetching) and `docs/architecture.md` (Git, Fetch invocation) define it: the explicit Fetch now action and opt-in per-workspace auto-fetch (off by default), with the hardened invocation that updates remote-tracking refs and tags only. Everything else is inspection. A vault may be a Git repository: saving a note there edits a file the user asked to edit and never touches Git's state; trash, drafts, and revisions never go in the vault.
- Notes stay ordinary Markdown: opening or indexing a note never rewrites it, unknown frontmatter keys are preserved, and Brainiac writes into a note only on a save or an explicit action (`SPEC.md`, section 5).
- Keep business logic out of Tauri command handlers so it is testable with `cargo test` without a WebView.
- Git test fixtures are built by the tests themselves (`git init` in a temporary directory), not checked in.
- The updater signing private key lives outside the repository (`~/.tauri/brainiac.key`, CI secret `TAURI_SIGNING_PRIVATE_KEY`). Never read it into a file in the repo or print it. Only the public key belongs in `src-tauri/tauri.conf.json`.
- The app version is declared only in `src-tauri/Cargo.toml`; Tauri reads it from there. Do not add a `version` to `package.json` or `tauri.conf.json` (`scripts/check-version.sh` fails if one appears). Bump it only through `scripts/release.sh`.

## Commands

- `pnpm install` then `pnpm tauri dev` to run; `pnpm tauri build` for a packaged app. `pnpm tauri:dev` runs under a separate app identifier, so it uses its own data folder and never migrates the installed app's database.
- Run `pnpm check` from the repository root before finishing: version agreement, Rust formatting, Clippy (all targets, warnings as errors), Rust tests, Biome, TypeScript, and frontend tests.
- Frontend: `pnpm check:web` for Biome lint, formatting, and import checks; `pnpm check:fix` for safe fixes; `pnpm format` to format. Generated DTOs in `src/lib/generated/` are excluded from Biome; regenerate them with Rust tests rather than editing them.
- Rust: `pnpm format:rust` to format; `pnpm format:rust:check`, `pnpm lint:rust`, and `pnpm test:rust` for individual checks. Rust tests also regenerate TypeScript bindings; commit them with the corresponding Rust changes.
- `pnpm typecheck` and `pnpm test` for frontend types and tests; `pnpm build` for the frontend build only.
- `pnpm test:editor` runs the WebKit tests (Playwright): the note editor alone (`e2e/editor/`) and the app's v0.2 views over an in-memory fake backend (`e2e/app/`). Run it after changing `src/lib/editor/` or the views. The first time, install the browser with `pnpm exec playwright install webkit`. CI runs it on every push.
- `pnpm vault:gen <folder> [--notes 10000] [--seed 1]` writes a synthetic vault, the same for a given seed on every machine, for measuring scans, search, and the watcher (`src-tauri/examples/gen_vault.rs`). Never commit a generated vault.
- `pnpm run` lists available scripts.
- Releases: `pnpm release X.Y.Z` (tag + push, CI builds a draft), then `pnpm release:publish vX.Y.Z`. These wrap the scripts in `scripts/`; `pnpm version:check` verifies version agreement. Process in `docs/RELEASING.md`. Every user-visible change gets one short line under `## [Unreleased]` in `CHANGELOG.md`, saying what the user notices, not how it was done; internal changes get none. Do not create version sections by hand: `scripts/release.sh` turns Unreleased into the release's section and updates the links.

The maintainer is new to Rust. When introducing a Rust concept for the first time in a change (ownership, lifetimes, traits, `Result`/`?`, `Arc<Mutex<_>>`), add a one-line comment explaining why it is used there.
