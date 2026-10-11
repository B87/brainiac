# Design: Brainiac on Linux

Design notes for a Linux build of the desktop app, beside the macOS one. Tauri v2 already targets Linux (the webview is WebKitGTK, the same engine family as macOS's WKWebView), and most of Brainiac is portable Rust or React: Git, the vault, the index, pull requests, databases, MCP, and the run controller already run on Linux, because remote hosts run the controller there today. What is macOS-only is a short, finite list: the secret store, a few hard-coded paths, window chrome, ⌘ shortcuts, the packaging and release pipeline, and some assumptions about how the app's executable is found. This document names each, says what replaces it, and orders the work. Nothing here is on the roadmap or in scope for any release yet.

It builds on `docs/design/secrets.md` (phase 3 there is this document's secret store), the run controller and remote hosts (`SPEC.md` section 13; `docs/architecture.md`, Agent runs — v0.5), and the release pipeline (`docs/RELEASING.md`).

## Status: phase 1 started (11 Oct 2026)

Proposed 10 Oct 2026 from reading the code on `main` at v0.6.1. On 11 Oct the app was built and tested on Linux for the first time (below, Found by running), and the parts of phases 1 and 2 that change nothing on macOS went into 0.6.2: the build, a Linux CI job, the data folder, and the sockets. Nothing ships for Linux yet.

## Found by running (11 Oct 2026)

In an Ubuntu 22.04 container (arm64, under OrbStack on a Mac) with the Tauri build packages, Rust 1.99, Node 22.14, and PostgreSQL 14, as an ordinary user:

- **The build failed in one place:** `lib.rs` built the Keychain store with no `cfg`, although `credentials/mod.rs` already compiled it only on macOS. `security-framework` itself built on Linux as an empty crate. Fixed: the store is chosen by target, and Linux gets `NoStore` (nothing in it; a save is refused) until phase 3; `security-framework` is a macOS-only dependency.
- **One test assumed a case-insensitive volume** (`a_note_created_in_a_folder_spelled_differently_is_recorded_once`). The code was right: on a case-sensitive volume a note created in `projects/` beside `Projects/` gets its own folder and is recorded as spelled on disk. The test now expects whichever the volume does, which also covers case-sensitive APFS on a Mac.
- **Then `pnpm check` passed in full:** Clippy with all targets and features, 793 Rust tests (the PostgreSQL ones included), Biome, TypeScript, 140 frontend tests, and the frontend build, with the TypeScript bindings unchanged.
- **The WebKit view tests: 44 of 51 pass.** All seven failures press ⌘ (`Meta+Enter`, `Meta+z`, `Meta+a`, `Meta+ArrowUp`, `Meta+ArrowDown`) where Linux uses Ctrl: undo and copy in the editor, moving the cursor past a note's title and its images, and running a query. This is the Shortcuts and hints row: the tests move to Playwright's `ControlOrMeta` with the `mod` helper, and the CI job runs them after that.
- **The linker ran out of memory** linking the test binaries in parallel in a container with 16 GB; `CARGO_BUILD_JOBS=4` was enough. CI's runners have their own limits.
- **Built in 0.6.2 besides:** `mcp::data_dir` follows Tauri on Linux (`$XDG_DATA_HOME`, else `~/.local/share`), the socket limit is 107 bytes off macOS, and the socket fallback is `$XDG_RUNTIME_DIR/brainiac/` on Linux when the session has one. macOS behaves as before.

## Why

- **Who asks.** The same people who use Brainiac for repositories, notes, and agent runs on a Mac, on a Linux desktop. The agent-runs half is already Linux-shaped: the container engine, the controller, and the approved hosts are Linux.
- **The cost is mostly known.** `credentials/mod.rs` already puts the store behind `SecretStore`, `agents/controller.rs` already picks `CLOCK_BOOTTIME` off macOS, and `docs/design/secrets.md` already chose Secret Service. The work is finishing those seams, not a rewrite.
- **A reason not to wait.** Every macOS-only assumption added from now on makes the port larger. This document gives them a place to be recorded.

## What the user gets

- A download from the release page: an **AppImage** (runs anywhere, updates itself), a **.deb**, and an **.rpm**. x86_64 first; aarch64 once the x86_64 build is steady.
- The same app: Git viewer, vault, tasks, pull requests, databases, agent runs, explanations. The same data and the same file formats, so a vault or a backup moves between a Mac and a Linux machine.
- Secrets in the desktop's own store (GNOME Keyring, KWallet, or KeePassXC, through the Secret Service API). Where none runs, Brainiac says so and offers the sources that need no store: an environment variable or a command.
- Ctrl where the Mac has ⌘, in the menus, the hints, and the shortcuts, and the window drawn with the desktop's own title bar.
- Agent runs on the local Docker Engine, in the user's own account, with no Docker Desktop or VM between.

## What is macOS-only today

Found by reading the code. "Fix" is the proposed change; the phases below say when.

| Area | Where | Today | Fix |
| --- | --- | --- | --- |
| Secret store (builds, 0.6.2: `NoStore`) | `credentials/macos.rs` (`security-framework`), gated by `cfg(target_os = "macos")` | Only the Keychain; on Linux the build has no store | `LinuxStore: SecretStore` over the Secret Service (below) |
| Data folder (done, 0.6.2) | `mcp::data_dir` | Hard-coded `~/Library/Application Support/<identifier>` "as Tauri resolves it on macOS" | Use Tauri's `app_data_dir` result everywhere, or branch: `$XDG_DATA_HOME` / `~/.local/share/<identifier>` |
| MCP and runner sockets (done, 0.6.2) | `mcp::socket_path`, `agents/runtime.rs` | Limit 103 bytes (macOS's `sun_path` is 104); fallback in `/tmp/brainiac-<uid>/` | Limit 107 on Linux; prefer `$XDG_RUNTIME_DIR` (already private to the user, mode 0700) for the fallback |
| Container engine | `agents/engine.rs`, `databases/health.rs` | Candidate sockets are Docker Desktop's, Colima's, OrbStack's, Rancher's, and `/var/run/docker.sock`; supported list is "OrbStack", "Docker Desktop" | Add `$XDG_RUNTIME_DIR/docker.sock` (rootless) and name `/var/run/docker.sock` "Docker Engine"; add it to the tested list only after the spike (below) |
| Window chrome | `tauri.conf.json`: `titleBarStyle: Overlay`, `hiddenTitle`, `trafficLightPosition` | Draws under the macOS traffic lights; `data-tauri-drag-region` bars in `Dashboard.tsx` and `SettingsPage.tsx` leave room for them | A `tauri.linux.conf.json` with native decorations, and one platform flag in the frontend so only macOS reserves the traffic-light gutter |
| Shortcuts and hints | `App.tsx` (`metaKey`), `CommandPalette.tsx`, `RunsView.tsx`, titles in about a dozen components, and the editor keymap | ⌘ and ⌥ glyphs written into strings; `metaKey` tested directly | One `mod` helper that both matches the event and renders the hint (⌘ / Ctrl, ⌥ / Alt, ⇧ / Shift) |
| Menu | `lib.rs` | The "Brainiac" app menu with Services, Hide, Hide Others, Show All; `CmdOrCtrl` accelerators already | Build the menu per platform: Settings, About, Check for Updates, and Quit move to File, Edit, and Help on Linux; drop the macOS-only predefined items |
| Executable path | `mcp::` settings snippet, `mcp/helper.rs`, `agents/controller.rs` (`current_exe`) | Assumes a path that stays put, like `/Applications/Brainiac.app/Contents/MacOS/brainiac` | See "The executable moves", below |
| Updater | `tauri-plugin-updater`, `latest.json`, `.app.tar.gz` | macOS bundle only | AppImage updates in place; .deb and .rpm do not (below) |
| Packaging and CI | `release.yml` (`macos-26`, universal target), `ci.yml` (`macos-26`), `scripts/install.sh` | macOS only | A Linux job in both; an install note for Linux (below) |
| Text | `git.rs` ("Install the Xcode Command Line Tools"), "This Mac", "Finder", "macOS login keychain", "macOS notification" | macOS words | Words chosen by platform at the edges, or neutral ("this computer", "file manager", "the system store") |

Already portable, and worth recording so a reader does not re-check it: `since_boot()` in `agents/controller.rs` picks `CLOCK_BOOTTIME` off macOS, and the sleep behavior it measures is the same; Git is run as the system CLI with argument arrays; `rustls-platform-verifier` reads the system roots on Linux; `reqwest` uses `rustls`, not Secure Transport; `rusqlite` bundles SQLite; `tauri-plugin-notification` talks to the notification daemon over D-Bus; `tauri-plugin-opener` uses `xdg-open`; the run controller's remote-host install already ships `brainiac-runner` for Linux.

## Design it twice

### The secret store

- **A. Secret Service through a Rust crate (chosen).** Two crates fit. `oo7` is pure Rust and async, speaks D-Bus, and also reads the Flatpak portal, but it is Linux only. `keyring` is one API over the macOS Keychain, Windows Credential Manager, and the Linux Secret Service (with a kernel keyutils option), so the same crate would also serve Windows if it ever comes. Items carry the attributes `service=brainiac` and `account=<key>`, so `secret-tool store --label='Brainiac: github' service brainiac account github` adds a token from a terminal, the way `security add-generic-password` does on a Mac (`docs/design/secrets.md`, Stores). A locked collection asks to unlock it, a prompt like the Keychain's. The choice between the two waits for the spike, with the store contract in `docs/design/secrets.md` as the test: create, replace, delete, an existence check that does not read the value, a locked collection, and no service at all.
- **A2. `keyring` on every platform, macOS included.** One backend to maintain. Rejected for macOS: the secrets design depends on an existence check that does not read the value (it is what avoids the Keychain approval prompt), on pending-marker recovery when a save is cut off, and on how Keychain approval interacts with code signing (`docs/keychain-access.md`). `keyring` may not expose all of that on every backend, and moving macOS onto it adds risk and no capability. So `credentials/macos.rs` stays on `security-framework`, and `SecretStore` stays the seam: each platform has its own implementation behind it, and `keyring` (or `oo7`) is only the Linux one.
- **Other portable options.** Tauri's Stronghold plugin is an encrypted vault file that is the same everywhere, but it needs a user password or key, and Tauri has said it will be deprecated in v3; not a base to build on. An encrypted file of our own is B, below.
- **B. A file encrypted with a key the user types.** Works on a bare window manager, but it is a second secrets design with its own recovery story, and a passphrase prompt on every start. No.
- **C. A plain file with mode 0600.** No: it breaks "a secret is never in Brainiac's databases, logs, snapshots, or exports", and it would be the weakest store in a project that spent a phase on this.

When no Secret Service is running (a bare window manager, a server session, a container), `LinuxStore` reports itself unavailable with a message that names the fix ("Start GNOME Keyring or KWallet, or choose an environment variable or a command as the source"). Settings → Secrets shows that state; saving a secret with the Store source is refused rather than falling back to a file. Sources that need no store keep working.

### How the window looks

- **A. Native decorations on Linux (chosen).** GTK, KDE, and tiling window managers each expect their own title bar behavior, snap, and tiling hints; drawing our own is the usual source of "feels wrong" reports. The cost is a title bar above the toolbar, and a frontend flag so the overlay gutter is a macOS-only rule.
- **B. A custom title bar with our own buttons.** Looks the same on both systems. Needs window controls, resize edges, and Wayland move and resize requests done by hand, and still fails to match a user's theme. No.

### How it is installed

- **AppImage (first-class).** One file, no root, and the only Linux format the Tauri updater can replace in place.
- **.deb and .rpm.** The native route for Debian, Ubuntu, Fedora, and openSUSE. They do not self-update; "Check for Updates…" says a new version is available and links the release page.
- **Flatpak, Snap, and distribution repositories.** Not in this design. A Flatpak sandbox blocks the program launches Brainiac depends on (`git`, `ssh`, `docker`, `code`, `op`) unless the user grants host access, which erases the sandbox's point. Worth a separate look if people ask.

## The executable moves

Three places assume the executable stays at one path:

- **The MCP settings snippet** (`mcp.rs`, `mcp/helper.rs`) shows the app's own executable so an agent client can run `brainiac mcp`. An AppImage runs from a mount at a new random path (`/tmp/.mount_Brainia…/usr/bin/brainiac`) every launch, so a snippet copied once is wrong on the next start. The fix: the snippet uses the `$APPIMAGE` path when set (the file the user downloaded), and Brainiac offers to install a `brainiac` link in `~/.local/bin` pointing at it.
- **The run controller** is started by running the app's own executable as `brainiac runner` (`agents/controller.rs`, `agents/runtime.rs`), detached in its own process group so it survives the app. If that executable is inside an AppImage mount, the mount can go away while a run is in flight. The fix reuses what remote hosts already do: install the `brainiac-runner` binary the app carries into the data folder (versioned, digest-checked, as `agents/hosts.rs` does over SSH, here with a local copy) and run it from there. This also makes the local and remote paths one code path, which the run-controller contract already asks for.
- **Child processes inherit the AppImage's environment.** The AppImage runtime sets `LD_LIBRARY_PATH`, `APPDIR`, and similar. If they leak to `git`, `ssh`, `docker`, `code`, `op`, or a user's command, the child loads the bundle's libraries and fails in odd ways. Every spawn site that builds a `Command` (`git.rs`, `credentials/command.rs`, `machines/`, the editor launcher, `agents/`) goes through one helper that removes them, with a test.

## Agent runs on Linux

- **The engine is the user's own.** On a Mac, Docker runs in a VM, so a container sees only what Docker shares. On Linux it runs on the host kernel. The rules in `docs/architecture.md` (no checkout mounted, containers non-TTY with logging off, the credential through stdin, a read-only offline collector) do not depend on the VM, but they were tested only through one, so the spike re-runs the v0.5 exit gates on Linux rather than assuming them.
- **Who can talk to Docker.** A user outside the `docker` group cannot open `/var/run/docker.sock`. Settings → Agents says that in words and points at rootless Docker, and does not suggest adding the user to the group (which is root-equivalent) as a first remedy.
- **File ownership.** Bind mounts keep the host's user and group IDs, which Docker Desktop hides. The places that mount something Brainiac wrote (the input files for an explain run, the collection folder) need a check that the container user can read them and that what the collector reads back belongs to the user.
- **Rootless Docker and Podman.** Rootless Docker is the same API on a different socket and is in the first phase. Podman's Docker-compatible socket is not: `docker.rs` talks to the engine's API, and differences there are a design of their own.
- **"This Mac" becomes "this computer"** in Settings → Agents, the host list, and the run's header, with the host's name when it helps.

## Files and watching

- **inotify.** The `notify` crate uses inotify on Linux. `vault.rs` already treats event kinds as untrusted because FSEvents merges flags; inotify has its own gaps (no recursive watch, so one watch per directory; queue overflow). The watch limit (`fs.inotify.max_user_watches`, often 8192 on older distributions) can be lower than a vault with thousands of folders plus several repositories. The watcher must degrade to the full reconcile it already runs on activation, and say so once, instead of failing silently. `pnpm vault:gen` builds the vault to measure this.
- **Case.** macOS volumes are usually case-insensitive and Linux ones are not. Note links, `NoteTree`, and the index compare paths; anything that lower-cases to match on a Mac must be checked for the opposite mistake. A vault copied from a Mac can hold two names that differ only in case.
- **Trash.** Notes move to Brainiac's own trash, not the desktop's, so nothing changes. The "reveal" actions use the opener plugin, which asks the file manager over D-Bus where it can and opens the folder where it cannot.

## Window and desktop behavior

- **Closing the window.** On macOS the app stays running with no window; on Linux a closed last window usually quits the program. Because the run controller is its own process, quitting never stops a run, but a watch on a branch (notifications) and the MCP socket stop with the app. Decide whether Linux quits on close or keeps running with a tray icon (open question).
- **Notifications** go through the desktop's notification daemon. The words "macOS notification" in docs and the settings description change; behavior does not.
- **Wayland and WebKitGTK.** Known WebKitGTK problems on some GPU and Wayland combinations (a blank or flickering window) have environment-variable workarounds. The spike records which machines show them, and the release notes carry the fix, rather than the app setting the variables for everyone.
- **WebKitGTK version.** The oldest supported distribution decides which WebKitGTK we can use (Tauri v2 needs `webkit2gtk-4.1`). The editor (CodeMirror) and the diff virtualization were tested on macOS's WebKit and Playwright's WebKit build; both must pass on the system WebKitGTK of the oldest target.
- **The `vault:` image protocol.** The CSP allows `img-src vault:` and `lib.rs` registers an asynchronous custom scheme. Tauri builds the URL differently per platform; check that images from a note render on Linux, including under the CSP.

## Packaging, signing, and releases

- **Bundle targets** become `app`, `dmg` on macOS and `appimage`, `deb`, `rpm` on Linux, through per-platform config (`tauri.linux.conf.json`), keeping `tauri.conf.json` the one place for shared settings. Icons: the PNG set already exists.
- **The build image.** Ubuntu 22.04 (glibc 2.35 and `webkit2gtk-4.1`) is the oldest baseline that gives a binary running on current Debian, Ubuntu, Fedora, and Arch. The Rust side needs a C compiler and `cmake` (for `aws-lc-rs`), the GTK and WebKitGTK development packages, and `libssl` is not needed (rustls).
- **`release.yml`** becomes a matrix of macOS and Linux jobs uploading to the same draft release with `tauri-action`, which merges both platforms into one `latest.json`. The updater's minisign key signs the AppImage the way it signs the macOS archive; there is no notarization step on Linux. The "tag must match the manifests" and changelog steps run once, not per job.
- **`scripts/install.sh`** installs `Brainiac.app` into `/Applications` and stays macOS-only. Linux gets a section in the README (AppImage: download, `chmod +x`, run; .deb and .rpm: the package manager), not a second installer script.
- **`ci.yml`** gains a Linux job running `pnpm check` (Rust format, Clippy, tests, Biome, TypeScript, frontend tests) and `pnpm test:editor` with Playwright's WebKit. This is the early warning that prevents the next macOS-only assumption; it can start before any Linux build ships.
- **Tests with macOS paths.** Some tests use `/Users/…` or `Library/Application Support` strings; they test string handling and pass anywhere, but a few build a real data folder through `mcp::data_dir` and must follow the fix.

## Phases

1. **Spike and CI (no user-visible change).** Add the Linux CI job, make the crate build and the tests pass with the macOS-only code behind `cfg`, run `pnpm tauri dev` on a real Fedora or Ubuntu desktop, and write down what broke. Exit: a list that replaces the table's "found by reading" with "found by running".
2. **The platform seams.** Data folder, sockets, engine candidates, executable path and the AppImage environment helper, words, and the `mod` helper for shortcuts. Each is small, testable with `cargo test` or Vitest, and also correct on macOS.
3. **The secret store.** `LinuxStore`, the unavailable state in Settings → Secrets, the `secret-tool` line in the docs. Exit: the store contract tests from `docs/design/secrets.md` pass against a real GNOME Keyring and a KWallet session.
4. **Window and menu.** Linux config, the traffic-light gutter as a macOS-only rule, the per-platform menu.
5. **Runs on a Linux host.** Re-run the v0.5 gates on a local Docker Engine and on rootless Docker; the local runner installed into the data folder.
6. **Packaging and release.** The bundle targets, the release matrix, the updater for AppImage, and the README section. Ship as a pre-release first (`vX.Y.Z-beta`; `release.yml` already marks tags with a hyphen as pre-releases).

Phases 1 and 2 can start on any day, because they also tighten the macOS build.

## Not in this design

- **Windows.** A Linux build removes the Unix-only assumptions the code already has (Unix sockets, process groups, `libc`), and Windows would need its own versions of each. Separate document if it comes.
- **Podman, Flatpak, Snap, and distribution repositories** (above).
- **A tray icon or autostart**, unless the close-behavior question below says otherwise.
- **Linux-only features.** The port is for parity, not for new behavior.

## Testing

- `cargo test` and `pnpm check` on Linux in CI from phase 1; the store tests that need a Secret Service run in a job that starts `gnome-keyring-daemon` under `dbus-run-session`, and are skipped on machines without one.
- A unit test per spawn site that the AppImage variables are removed from the child's environment.
- The Playwright WebKit suites on Linux, which load the editor and the app views over the fake backend.
- By hand, before the first pre-release: a clean Fedora (Wayland, GNOME Keyring), a clean Ubuntu LTS (X11), and KDE with KWallet; each runs Settings → Accounts, a database, a note, and a container run end to end. An AppImage launched twice, with a run left going between them, to check the executable-path fixes.

## Open questions and spikes

1. **Which crate for the Secret Service?** Compare `oo7` and `keyring` against the store contract (create, replace, delete, existence without a read, a locked collection, no service at all). Owner: phase 3; the spike can run earlier.
2. **Does a run work the same on a local Docker Engine?** File ownership of mounts, the collector's read-only mount, the container's network isolation, and `--network none` behavior are the places the VM may have hidden a difference. Owner: phase 5 gates.
3. **Does a local runner survive an AppImage quit?** Starting from a mounted AppImage and quitting the app while a run is in flight is the test; the proposed fix is to install the runner into the data folder either way.
4. **Quit on close, or keep running?** Quitting loses watch notifications and the MCP socket until the next launch; a tray icon is a new feature with its own desktop-environment problems (GNOME has none by default). Default to quitting and say so in the docs.
5. **Oldest supported WebKitGTK.** Pick the oldest distribution (Ubuntu 22.04 LTS?) and test the editor and the diff view on its WebKitGTK; drop it if they do not work.
6. **x86_64 only, or aarch64 as well?** GitHub offers Linux arm64 runners; decide after the x86_64 build has shipped once, and from whether anyone asks.
7. **Is a `brainiac` link in `~/.local/bin` acceptable?** It is the one file Brainiac would write outside its data folder; it would be an explicit action in Settings → Agent access, with a button to remove it.
8. **A .deb/.rpm update story.** The package manager is the user's; say so, or publish an apt/dnf repository later. Not needed to ship.

## When this starts

When a person who does not use a Mac asks for it, or when the maintainer wants to run Brainiac on a Linux machine. Phases 1 and 2 need no decision beyond doing them. When it starts, move the user-visible parts of this document into `SPEC.md` (platforms, secrets, install) and the technical parts into `docs/architecture.md` (the platform seams, the store, packaging), add a decision there for the secret store and one for the window, and put the milestone in `docs/roadmap.md`.

## Sources

- Tauri v2, Linux prerequisites and distribution (AppImage, deb, rpm), the updater's platform support, and per-platform configuration files — <https://v2.tauri.app/>
- Freedesktop Secret Service API — <https://specifications.freedesktop.org/secret-service/latest/>
- `oo7` — <https://crates.io/crates/oo7>; `keyring` — <https://crates.io/crates/keyring>
- inotify(7) and `fs.inotify.max_user_watches` — <https://man7.org/linux/man-pages/man7/inotify.7.html>
- AppImage runtime environment (`APPIMAGE`, `APPDIR`) — <https://docs.appimage.org/>
- `docs/design/secrets.md`, `docs/design/agent-runs.md`, `docs/architecture.md` (Agent runs — v0.5), `docs/RELEASING.md`
