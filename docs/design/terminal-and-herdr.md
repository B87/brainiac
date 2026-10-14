# Design: a terminal, and herdr inside it

Notes on two features that work together and stand alone: a **terminal** pane in Brainiac, and an integration with [herdr](https://github.com/ogulcancelik/herdr), a terminal multiplexer made for coding agents, that shows herdr's agents on Brainiac's repositories and workspaces. Today the maintainer keeps herdr open in another terminal beside Brainiac, one herdr workspace per project, with Claude Code and other agents in its panes. Nothing here is on the roadmap or in scope for any release.

It builds on workspaces and repositories (`SPEC.md` section 4), Needs you and the Runs list (section 13), Machines (`docs/architecture.md`, Machines), the module boundaries (`docs/design/modules.md`; `src-tauri/tests/module_boundaries.rs`), and the in-checkout pane of `docs/design/checkout-runs.md`, which this design overlaps and is compared with below.

UX design canvas: https://claude.ai/artifact/2fRcZ9uiUUxFRxyNrMNtF7 (the Terminal view with herdr, a shell in the dock beside a diff, Needs you, Settings → Terminal and herdr, and the dock's states)

## Status: phase 2 in progress (13 Oct 2026)

**Update, 13 Oct 2026:** the terminal (phase 2) is being built on `design/terminal-and-herdr`, its behavior moved into `SPEC.md` section 15 and its design into `docs/architecture.md`, Terminal, with the decision at the end of Decisions. The spike of phase 1 is not done yet: xterm.js against the maintainer's terminal with herdr and Claude Code, on macOS and WebKitGTK, is the first test of the build. herdr (phases 3 and 4) is not started. What follows is the design as written before that.

Nothing was built when this was written. Written with the maintainer from a reading of the code on `main` at v0.6.3 and of herdr's public descriptions; nothing about herdr here has been tested against a build of it. The first draft was one herdr-only module; the client panel reviewed it the same day, and this version splits it into two modules and takes in that round (What the panel said). Every herdr behavior this design relies on is in Open questions, and the spike in Phases answers them first.

## The decision in brief

| Question | Answer | Why |
| --- | --- | --- |
| A general terminal or a herdr module? | **Both, as two independent modules**: `terminal` (a pane that runs a program) and `herdr` (herdr's agents on Brainiac's rows) | They share almost no code but the program the pane starts. Each persona wanted a different one: a plain shell, herdr's states without the pane, or both |
| Which comes first? | The terminal, with a plain shell; then herdr in the pane; then herdr's states | A plain shell is useful to everyone; herdr only to those who install it |
| Who keeps processes alive? | herdr's server, for herdr; for a plain shell, nothing: the shell ends with the window, as a terminal app's tab does | Persistence for bare shells would need the run controller; herdr already gives it to those who want it |
| Where does it live? | **A Terminal view** that fills the main area, **and a dock** beside any view: two sizes of the same tabs | herdr is a full-screen program and is cramped in a strip; a shell beside a diff is the dock's job (Where the terminal lives) |
| How is it drawn? | xterm.js in the WebView | The only option that fits Brainiac's layout on macOS and Linux (How it is drawn) |
| How does Brainiac learn about herdr's agents? | herdr's CLI, polled while Brainiac is visible | The CLI is herdr's public face; the socket and screen reading are not |
| Can it be turned off? | Yes: Settings → Terminal, on by default for a plain shell; the herdr integration only when herdr is found and the user turns it on | A team that wants Brainiac to run nothing turns the terminal off |

## Why

- **Two windows for one piece of work.** The repository's diff, branches, and pull requests are in Brainiac; the shells and agents changing that repository are in another terminal. Switching between them is constant, and neither knows about the other.
- **herdr already solves the hard part of agents in terminals.** It keeps sessions alive when its client detaches, detects a dozen agents' states without setup (process names plus output heuristics, and optional hooks), notifies when a background agent needs the user, splits panes, and reaches remote machines (`herdr --remote ssh://…`). Rebuilding that in Brainiac would be months of work that exists.
- **Brainiac knows what herdr does not.** Which folders are repositories, which belong together in a workspace, what changed in each, which branch it is on, and which pull request it belongs to. An agent's state is more useful on the repository's row than in a sidebar of pane names; all three panel personas named the rows, not the pane, as the value.

## What the user gets (sketch)

### The terminal

- **Terminal in the sidebar,** under Runs, opens the **Terminal view**: the terminal's tabs fill the main area, and Brainiac's sidebar stays beside them with its repositories and their agents. Collapsed to its rail (`Cmd+B`), the sidebar leaves the terminal almost the whole window. **`Cmd+T`** opens the view from anywhere; **`Cmd+Shift+T`** opens a new tab, in the folder of the repository last selected: in the view when it is shown, otherwise in the dock.
- **The dock** shows the same tabs at the bottom of any other view (or on the right; remembered), for a shell beside a diff. **`Cmd+J`** shows or hides it, as VS Code's panel does. **Maximize** in the dock opens the Terminal view; **Dock** in the view goes back to the view before it with the dock open.
- **One set of tabs, two sizes.** Moving between them moves the same tab, and the program in it only sees its window resized: nothing restarts and no output is lost. Opening a repository from the sidebar while the Terminal view shows leaves every tab running, in the dock if it was open.
- **New terminal** on a repository, a linked worktree, or a workspace (its More menu, the palette, the overview row's context menu) opens a tab in that folder (Workspaces and repositories, below), in the dock when another view is showing and in the Terminal view when that is.
- **It runs the user's login shell** by default, with their environment. Settings → Terminal can name another program and its arguments, which is how herdr runs in it.
- **Focus.** While a tab has focus, keys go to it (Keys, below). **`Cmd+Esc`** (`Ctrl+Esc` on Linux) moves focus between the tab and the rest of the window without hiding it; the terminal says so the first time it opens.
- **Closing a tab ends its shell**, as in a terminal app; Brainiac asks first when the shell has a program running in front. Quitting Brainiac ends every plain shell, and the quit dialog says how many are busy.

### herdr in it

- **Run herdr in the terminal** (Settings → herdr, offered when `herdr` is found): the terminal's first tab runs `herdr session attach <session>` instead of a shell, and stays a herdr client: herdr's prefix key, splits, mouse, sidebar, and notifications work as in another terminal. Closing its tab or quitting Brainiac detaches the client, and the tab says "Your agents keep running in herdr" the first time.
- **Open in herdr** on a repository, a worktree, or a workspace opens the Terminal view on herdr's tab, where herdr has room, and focuses the herdr workspace that belongs to the Brainiac workspace and a pane in that folder, creating either if missing.
- **Agents on the rows.** Each repository's row in the sidebar shows its agents from herdr, and a workspace's header in the sidebar sums its repositories. Clicking one opens the Terminal view on that pane. The workspace overview (the dashboard) shows no agents for now (Phases).

  | Shown | herdr's state | Means | Clears |
  | --- | --- | --- | --- |
  | `● 2 working` | working | An agent is doing a turn | When the turn ends |
  | `▲ 1 waiting for you` | blocked | An agent asks a question or a permission, in its pane | When answered |
  | `✓ finished` | done | An agent ended its turn since you last opened its pane | When you open the pane, or after 30 minutes |
  | nothing | idle | An agent is waiting for your next prompt | — |

- **Needs you** counts herdr agents waiting for you beside runs that wait, in their own group, **Answer in the pane**, so they are not mistaken for runs answered in Brainiac. A row opens the Terminal view on that pane.
- **herdr's agents also work without the terminal.** The integration needs only the `herdr` executable, not the terminal module: a user who keeps herdr in their own terminal app gets the rows and Needs you, and Open in herdr then copies the `herdr` command to run instead of opening the Terminal view.
- **Settings → herdr:** the executable found and its version, the session mode (Sessions, below), whether Brainiac also notifies (off by default, because herdr already does), and **Test**, which checks the version, each CLI call this design uses, and, with the terminal on, that the pane draws. A failing step says what to do in words, not which call failed.

### When something is wrong

| Situation | Rows and Needs you | herdr's tab |
| --- | --- | --- |
| herdr not installed | Nothing; Settings → herdr says how to install it | A plain shell, as before the integration was on |
| herdr's server not running (after a reboot, or killed) | States hidden; the workspace header says **herdr is not running** | **Start herdr**, which attaches and so starts the server; no agents come back, because herdr's own sessions did not survive the reboot |
| herdr's version outside the supported range | States hidden; Settings → herdr names the version found and the range | Still runs herdr; Open in herdr is unavailable, with the reason |
| A CLI call times out (5 s) or fails | The last states kept for 30 seconds marked stale, then hidden | Unaffected |
| A repository missing on disk | Its panes stay on its row, marked missing | Unaffected |
| The session was attached elsewhere (another terminal) | Unaffected | Both clients show the same session, as herdr does; the tab says so once |

## Design it twice

### What Brainiac builds

| | A. A terminal | B. herdr only | C. Brainiac's own multiplexer | D. herdr's states, no pane | A + D, two modules (chosen) |
| --- | --- | --- | --- | --- | --- |
| What it is | A pane running a program, the login shell by default | A pane that runs `herdr`, plus its client | Splits, sessions, agent detection, notifications in Brainiac | Brainiac reads herdr's agents; the user's own terminal shows herdr | A and D, wired by settings |
| Knows agent states | No | Yes | Yes | Yes | Yes, with herdr |
| Useful without herdr | Yes | No | Yes | No | Yes |
| One window | Yes | Yes | Yes | No | Yes, or not, as the user prefers |
| Work | Pseudo-terminal, xterm.js, keys | A plus a client | A plus months | A small client | A plus a small client |
| Depends on | Nothing | herdr's CLI | Nothing | herdr's CLI | herdr's CLI, only for D |

**A + D.** The first draft chose B, one herdr module. The panel showed three different wants: a plain shell for someone who does not use herdr (A), herdr's states for someone whose terminal is already tuned (D), and both for someone who lives in herdr. B serves only the third, and costs the same as A + D, because B's pane is A with herdr's command. C rebuilds herdr.

The two modules do not import each other. The shell (`lib.rs`) wires them: when the integration is on and Run herdr in the terminal is chosen, it sets the terminal's first tab to herdr's attach command, and gives the integration a callback that opens the dock on a pane. Without the terminal module, that callback copies the command instead.

### Where the terminal lives

| | A dock only | A full view only | Both, one set of tabs (chosen) | A window of its own |
| --- | --- | --- | --- | --- |
| herdr | Cramped: in a 430-point strip, herdr's sidebar and two splits leave about 60 columns per agent | Room for herdr's sidebar and splits, beside Brainiac's sidebar | The view | Room, on a second display |
| A shell beside a diff | Yes | No: the view replaces the diff | The dock | Beside the window, not in it |
| Brainiac's rows and Needs you | Visible | Visible in the sidebar | Visible | In the other window |
| Cost | One placement | One `View` kind | One `View` kind, a dock, and moving a tab between them | A second WebView window, its own focus and menus |

**Both.** The view is where herdr and agents live, as they do today in a terminal window of their own; the dock is for a quick shell while reading a change. Moving a tab between them moves its xterm.js instance's element, and the pseudo-terminal gets a resize, which full-screen programs like herdr redraw for. A window of its own needs nothing more from the terminal module, because the tabs do not belong to a view, so it is left for later (Phases).

### Who owns the pseudo-terminal

| | In the app process (chosen) | In the run controller (`brainiac runner`) | herdr's own thin-client protocol |
| --- | --- | --- | --- |
| How | `portable-pty` in a task of the app | The controller opens it and the app attaches, as checkout runs plan | Brainiac speaks herdr's client protocol and draws its frames |
| On quit | Plain shells end; a herdr client detaches and herdr's agents keep running | Shells keep running | As the app process |
| Redraw after reattach | Nothing to reattach; herdr redraws itself on attach | Needs a screen model (`vt100`) in the controller | herdr sends frames |
| Cost | Least | The controller gains a second workload | The protocol is internal to herdr and undocumented |

**In the app process.** Those who want shells and agents to outlive Brainiac use herdr, which already does it; a plain shell behaves like a terminal app's tab. If checkout runs are built, their pane needs the controller, and the pseudo-terminal moves there behind the same interface (Compared with checkout runs).

### How it is drawn

| | xterm.js in the WebView (chosen) | A native terminal view over the window | Brainiac's own renderer over a Rust screen model |
| --- | --- | --- | --- |
| Examples | VS Code, Tabby, Hyper | `libghostty`, an `NSView` placed over the WebView | `alacritty_terminal` or `vt100`, drawn on a canvas |
| Fidelity for a full-screen TUI | Good: true color, mouse, bracketed paste, Unicode with the unicode11 add-on; to be measured with herdr | Best | Whatever we write |
| Speed | The WebGL add-on handles floods of output; to be measured | Best | Unknown |
| Fits the layout (docking, resizing, dialogs above it) | Yes, it is a DOM element | Hard: a native view above the WebView ignores CSS, z-order, and focus | Yes |
| macOS and Linux | Same code (WKWebView, WebKitGTK) | One embedding per platform | Same code |

**xterm.js**, loaded only when a terminal first opens, with the fit, WebGL, unicode11, and web-links add-ons. The spike measures it against the maintainer's terminal with herdr and Claude Code; a native view is the fallback if it fails, not the start.

### How Brainiac talks to herdr

| | The `herdr` CLI (chosen first) | herdr's Unix socket directly | Reading the pane's screen |
| --- | --- | --- | --- |
| How | `herdr workspace create`, `herdr pane run`, `herdr pane read`, … with argument arrays, never a shell | The JSON protocol the CLI uses | Brainiac parses what herdr draws |
| Stability | The CLI is herdr's public face | Internal unless herdr documents it | Breaks with any change |
| Live updates | Polling, unless the CLI has a watch or event command | Events, if the protocol has them | Live |

**The CLI first.** One call per poll that lists every pane with its folder and agent state (if herdr has one; Open questions), every 3 seconds while Brainiac's window is visible, and **none while it is hidden or minimized**: herdr notifies in the background, so Brainiac does not need to. A poll that would start more than one process, or takes over a second, is a reason to move to the socket or ask herdr for an event stream. Screen reading is rejected.

## Workspaces and repositories

### Where a new terminal opens

| Opened on | Folder |
| --- | --- |
| A repository or a linked worktree | Its working tree |
| A discovered workspace with a root | The root repository |
| A discovered workspace without one | The selected folder |
| A manual workspace | Asks which member, remembered per workspace |

Brainiac never opens one tab per member: a workspace of twelve repositories would open twelve shells nobody asked for. The tab's title is the repository's or workspace's name, and the tab list groups tabs by workspace.

### Sessions

How herdr's sessions map to Brainiac's workspaces is a setting, because the panel split on it: one persona wanted every agent in one sidebar, another wanted each client kept apart, so that one client's pane names never show while screen-sharing with another.

| Mode | herdr sessions | herdr workspaces | For |
| --- | --- | --- | --- |
| **One session** (default) | One, named `brainiac` | One per Brainiac workspace, plus **Repositories** for repositories in none | Seeing every agent at once in herdr's sidebar |
| **A session per workspace** | One per Brainiac workspace, named after it | herdr's default inside each | Clients kept apart; switching workspace in Brainiac switches herdr's tab to its session |
| **My own session** | One the user names | Whatever the user made | Someone whose herdr setup predates Brainiac; Brainiac creates nothing and only reads |

- **Links are stored, not guessed by name.** A small table in `brainiac.db` maps a Brainiac workspace to herdr's session or workspace ID, so renaming either side keeps the link. A herdr workspace the user removed leaves a link that Open in herdr recreates. Brainiac never removes or renames anything in herdr; it only creates a workspace or pane on Open in herdr, and Settings → herdr says so.
- **Isolation is herdr's to give.** A session per workspace keeps herdr's sidebar apart; whether herdr's agent socket lets an agent in one session read another session's panes is herdr's design (Open questions), and Settings → herdr says what was found.

### Which repository an agent belongs to

A pane belongs to the registered repository or linked worktree whose working tree holds the pane's current folder, by **the longest canonical prefix**. In a discovered workspace with a root (`product/` holding `services/billing/`), a pane in `product/services/billing/src` counts on `billing`, and one in `product/docs` on `product`. Then the cases the panel found:

- **An agent at a workspace's root or selected folder** (often with Claude Code's `--add-dir`) may be editing any member. It counts on the **workspace header**, labelled "at the workspace folder", not on the root's row, and its tooltip says it may be changing members; Brainiac does not guess which. A later version could count it on every member whose working tree changed during its turn.
- **A worktree not registered in Brainiac** (`.worktrees/x` inside a checkout, or outside every repository) is found by asking Git for the folder's working-tree root, read only. It counts on the main checkout's row as **in an unregistered worktree**, with **Add worktree** to register it, rather than silently on the checkout.
- **A repository in two workspaces** shows its agents on its row in both. In One session mode, a workspace's header sums only agents in its own herdr workspace, so one client's header never shows another's agents; in A session per workspace, the session decides.
- **An agent that `cd`s elsewhere** moves with it, if herdr reports a pane's current folder; otherwise Brainiac reads the foreground process's folder itself (Open questions).
- **Relocating a repository** (`SPEC.md`, Relocating a repository) re-attributes its panes on the next poll; a missing repository's panes stay on its row, marked missing.
- **Remote panes** have remote folders, attribute to nothing on this Mac, and are listed under their machine (Phases).

## Module shape

Two feature modules, both leaves: each imports only the core, neither imports the other, and only the shell imports them. Added to `FEATURES` in `module_boundaries.rs` as `("terminal", &["terminal"])` and `("herdr", &["herdr"])`, with no `ALLOWED` entries.

```text
src-tauri/src/terminal.rs          TerminalService: tabs, their programs, their events
src-tauri/src/terminal/
  pty.rs                           a pseudo-terminal: spawn an argument array, read, write, resize, signal, exit
  environment.rs                   the login shell's environment, read once per launch
src-tauri/src/herdr.rs             HerdrService: polling, Open in herdr, the agents event
src-tauri/src/herdr/
  cli.rs                           HerdrCli: each call this design uses, parsed into typed results; behind a trait so tests use a fake
  attribution.rs                   pure: (panes with folders, registered repositories and workspaces) → agents per repository and workspace
  links.rs                         the session and workspace links table
src/components/terminal/           the Terminal view, the dock, their shared tabs, the pane (xterm.js, loaded on first open), Settings → Terminal
src/components/herdr/              the row badges, the Needs you group, Settings → herdr
```

- **The core they use:** the workspaces and repositories with canonical folders (through a narrow lookup trait, as `AgentRunService` takes `RepositoryLookup`), `git` for a folder's working-tree root, `db` for herdr's links, and `events`. `environment.rs` starts in `terminal`; when herdr needs it too (to find `herdr` on the user's PATH), it moves to the core, since agent runs and checkout runs want it as well.
- **What they give:** `terminal` emits tab events to the view and the dock; `herdr` emits one `herdr_agents` event with agents per repository and workspace, which the sidebar and Needs you subscribe to. The repository code imports neither.
- **The seam between them** is two values the shell sets: the terminal's first-tab program (from Settings), and herdr's "open this pane" callback (open the Terminal view when the terminal module is on, else copy the command).
- **Turned off,** each does no work: the terminal starts no process until a tab opens; herdr polls nothing unless it is on and the window is visible.
- **DTOs** go in `models.rs` as the accepted decision says, exported by `ts-rs`; command handlers stay thin.
- **Removing it:** turning both off and **Forget herdr links** in Settings → herdr deletes the links table's rows; nothing else is stored.

### The pane, end to end

```text
View or dock (xterm.js) ── invoke: write, resize ──▶ TerminalService ── pty.rs ──▶ $SHELL -l, or herdr session attach <session>
       ▲                                          │
       └──── Channel<Vec<u8>>: output, batched ───┘
Sidebar, Needs you ◀── herdr_agents ── HerdrService ── poll (visible only) ──▶ herdr CLI (argument arrays)
```

- **Output** goes over a Tauri `Channel` in batches of at most 16 ms or 64 KB, never as an event per chunk. Input, resizes, and signals are plain commands.
- **The environment.** An app started from Finder has no shell PATH, so a program would not find `node` or `claude`. `environment.rs` reads the login shell's environment once per launch (`$SHELL -l -c 'env -0'`, with a timeout), as checkout-runs plans, and starts programs with it and `TERM=xterm-256color`. A plain shell tab is itself the login shell, so it needs this only for programs named in Settings.
- **Shortcuts this takes.** Two of Brainiac's keyboard defaults (`SPEC.md`, Keyboard defaults) change:

  | Shortcut | Today | With the terminal |
  | --- | --- | --- |
  | `Cmd+T` | New query tab (v0.4) | Open the Terminal view |
  | `Cmd+Shift+T` | — | New terminal tab, in the view when it is shown, otherwise in the dock |
  | `Option+Cmd+T` | — | New query tab, in Databases (moved from `Cmd+T`) |
  | `Cmd+J` | — | Show or hide the dock |

  `Cmd+T` follows Terminal and iTerm, where it is a new terminal tab, and the terminal is used all day while a query tab is opened now and then. The query tab's shortcut moves with an entry in the changelog, and Databases' File menu shows it; `Option+Cmd+T` follows `Option+Cmd+N` (New run) for a second "new" of a narrower kind.
- **Keys.** While a tab has focus, every key goes to it except `Cmd+Esc` / `Ctrl+Esc` (focus out), the dock toggle (`Cmd+J`), `Cmd+T`, `Cmd+Shift+T`, and on macOS Brainiac's `Cmd` shortcuts. On Linux, Brainiac's shortcuts use `Ctrl` (`linux.md`), which collides with shells' and herdr's keys (herdr's prefix is `Ctrl+B`, Brainiac's collapse-sidebar is `Ctrl+B`): with a tab focused, every `Ctrl` key goes to the tab (`Ctrl+J` and `Ctrl+T` too, which shells use), and `Ctrl+Esc` is the way out; `Ctrl+J` and `Ctrl+T` are Brainiac's only while focus is elsewhere. Option is sent as Meta, a setting.

## Boundaries

- **Brainiac starts programs the user chose,** with argument arrays and no shell of its own: the login shell, or the program in Settings. What runs in them is the user's, as in any terminal: commands and agents edit repositories and run Git. This is a deliberate change to the rule that the app never writes to a repository, made together with the planned Git operations; when either starts, the rule's new wording goes into `SPEC.md` section 2 and `AGENTS.md`, and a decision goes at the end of `docs/architecture.md`, Decisions. Settings → Terminal states it next to the switch.
- **No provider question.** v0.6's question (whether a repository's code may go to a provider) guards code that Brainiac sends. In a terminal the user starts the agent, as in Terminal, and Brainiac sends nothing; asking there would gate a terminal no other terminal gates. A team that wants agents kept away from a repository uses the agent's own managed settings. The panel's CTO asked for the question; this is where the design says no, and why.
- **Brainiac never writes herdr's configuration** (`~/.config/herdr/config.toml`) or installs herdr's agent integrations. Settings → herdr says which ones herdr offers (`herdr integration install <agent>`) for the user to run.
- **No agent tool reaches the terminal or herdr.** Brainiac's MCP tools do not open tabs, type into them, or read them.
- **Nothing a terminal shows is stored.** Scrollback lives in the pane while it is open; herdr keeps its own.

## License

herdr is AGPL-3.0-or-later. Brainiac starts it as a separate program the user installed and calls its CLI as any program would; it does not link, copy, bundle, or download herdr's code. That keeps herdr's license from applying to Brainiac's code, which is the ordinary reading for separate programs that communicate at arm's length, but it is not legal advice, and Brainiac's own license is not yet confirmed (`modules.md`, Open questions). Because the terminal does not need herdr, a team whose policy excludes AGPL tools still has it. If Brainiac ever ships herdr in its bundle or uses its code, this section is decided again first.

## Compared with checkout runs

`checkout-runs.md` puts the real `claude` CLI in a pane owned by the run controller, observed through Claude Code's hooks, with snapshots of the folder per turn. It lists "a general terminal in Brainiac" as not in its design; this design is that terminal, and the two now meet:

- **What this gives instead:** a terminal for anything; with herdr, persistence and agent states for many agents without hooks, at far less to build.
- **What only checkout runs give,** and what the panel said it would miss most: what each turn changed (By turn), what the agent is asking without reading the pane, the mode it is in, cost, an API key per workspace for billing each client, and guardrails (bypass disabled, notes on Git commands).
- **They can share a pane.** A checkout run is a terminal tab started with a fixed command, Brainiac's hooks, and snapshots. Building it moves `pty.rs` into the run controller behind the same interface and the xterm.js pane into the core; the terminal module then uses both like checkout runs do.
- **And later meet in herdr:** Brainiac could take a snapshot of a repository when herdr reports its agent went from working to finished, giving By turn for herdr agents without hooks.

## Phases

1. **Spike, before any design is accepted.** xterm.js running a shell and `herdr` with Claude Code in it, in the WebView on macOS and WebKitGTK on Linux: colors, mouse, herdr's prefix, `Cmd+Esc`, bracketed and image paste, a flood of output, resize; and every herdr call in Open questions, with the version tested. Written up in this file, as `agent-runs.md` records its spikes.
2. **The terminal.** The Terminal view and the dock with shared tabs, moving a tab between them, a plain shell in a repository's or workspace's folder, focus and keys, closing a busy tab, Settings → Terminal with the switch and another program.
3. **herdr in the terminal.** Run herdr in the terminal, the session modes and links, Open in herdr, Settings → herdr with Test, and the failure states of herdr's tab.
4. **herdr's agents on the rows.** Polling while visible, attribution with its edge cases, the states and when they clear, workspace headers, Needs you's Answer in the pane, and the failure states of the rows; working without the terminal module.
5. **Later:** remote machines through Machines, if the spike finds a way that keeps Brainiac's host-key approval (`herdr --remote` with SSH options, or `ssh -t … herdr session attach` over `machines/ssh.rs`); herdr agents in the workspace overview (a column per repository and a summary above it), in Today, and on tasks; attributing a root-folder agent to the members it changed; shells that outlive Brainiac without herdr, which needs the controller; the terminal in a window of its own.

## Testing

- `pty.rs` with a fake program: start, write, resize, signal, exit, a program that ignores hangup, output larger than one batch, a busy foreground process on close.
- `environment.rs`: a slow, noisy, and failing login shell; `fish`.
- `cli.rs` against a fake `herdr` (a script the test writes) for each call and version string, including an unknown version, a timeout, and a failure.
- `attribution.rs` as a table: a root with discovered members, nested folders, a registered and an unregistered worktree, a symlinked folder, a pane at the workspace folder, a pane outside every repository, a missing repository, a repository in two workspaces under each session mode.
- The links table: a renamed workspace, a herdr workspace removed by the user, a repository in no workspace, Forget herdr links.
- The view and the dock in the WebKit tests over the fake backend: opening, hiding, moving a tab between them with its output kept, focus in and out, keys reaching the tab, the Linux `Ctrl` keys going to the tab, and each failure state.
- By hand before a release: the maintainer's real herdr and agents; quit and reopen Brainiac during a turn; reboot with herdr running; herdr's own notifications not doubled; the window hidden starting no herdr process.

## Not in this design

- Bundling, installing, updating, or configuring herdr.
- A multiplexer, session manager, or agent detection of Brainiac's own.
- Splits inside the terminal (herdr's are there for those who want them).
- Brainiac answering an agent's question or permission from outside its pane.
- MCP tools that open, type into, or read terminals.
- Storing what terminals show.
- Billing identity per workspace for herdr's agents: they use whatever login the user's shell has. Checkout runs' API key per workspace is the design for that.

## What the panel said

The client panel (three fictional personas: a skeptical CTO, a learner, a freelancer with several clients) reviewed the first draft, a herdr-only module, on 13 October 2026. Treat it as hypotheses to check with real users.

- **The rows are the value, the pane second.** The CTO and the freelancer would keep herdr's states even with herdr in their own terminal; the learner valued them but could not read them ("what's the difference between blocked and waiting for me?"). Now D works without the terminal, and the states are named in words with when they clear.
- **herdr as a requirement.** The learner does not use herdr and would not install it for a terminal; the draft's terminal was a dead end without it. Now the terminal is first and needs nothing.
- **Failure states were missing:** herdr not running, a version out of range, the session gone after a reboot, a poll failing, and when `done` clears. Now a table.
- **Attribution missed common layouts:** an agent at a workspace root counted on the root, an unregistered worktree on the checkout, and a shared repository's agents on another client's header. Now each has a rule.
- **One session for everything** suits the CTO and leaks across clients for the freelancer. Now a setting with three modes.
- **No way out of the pane** without hiding it, three layers of keys untaught, and Needs you mixing answers given in Brainiac with answers given in a pane. Now `Cmd+Esc`, a first-time note, and its own Needs you group.
- **Polling on battery** while hidden. Now none while hidden.
- **Guardrails, by turn, cost, and a key per client** were what each missed most from checkout runs. They stay there (Compared with checkout runs).
- **Where this design says no:** the CTO asked for an off switch, now in Settings → Terminal, and for v0.6's provider question before Open in herdr, which Boundaries declines, with the reason.

## Open questions and spikes

| Unknown | How to resolve | Needed before |
| --- | --- | --- |
| xterm.js fidelity | The spike: a shell, herdr's sidebar and borders, mouse drag to resize, true color, wide characters, Claude Code inside a herdr pane inside xterm.js, against the maintainer's terminal | Phase 2 |
| Keys | herdr's prefix and other keys, whether its prefix can be changed in its config, `Cmd+Esc` / `Ctrl+Esc` free in shells and herdr | Phase 2 |
| Machine-readable CLI | Whether one call lists every pane with its folder and agent state, in JSON; otherwise parsing text pinned to a version range | Phase 3 |
| Creating panes with a folder | Whether `herdr workspace create` and `pane run` (or `split`) take a working folder, and whether a pane can be focused from the CLI while a client is attached | Phase 3 |
| Session attach semantics | Whether `herdr session attach <name>` creates a missing session, and what two attached clients see | Phase 3 |
| Isolation between sessions | What herdr's agent socket lets an agent in one pane or session do to others | Phase 3 |
| Reboot | Whether herdr's sessions survive a reboot or a logout (assumed not) | Phase 3 |
| A pane's current folder | Whether herdr reports where a pane's shell is now, or only where it started; else the foreground process's folder (`/proc/<pid>/cwd`, `proc_pidinfo` on macOS) | Phase 4 |
| Events instead of polling | Whether the CLI or socket streams state changes (`herdr wait` suggests the server knows them) | Phase 4 |
| Version compatibility | How often herdr's CLI changes and how actively it is maintained; the range Brainiac supports | Phase 3 |
| Notifications | Whether Brainiac can tell that herdr's own notifications are on, to avoid two for one event | Phase 4 |
| Remote and host keys | Whether `herdr --remote` accepts SSH options, or whether herdr over `ssh -t` works as well | Phase 5 |

## When this starts

Move its behavior into `SPEC.md` (a new section, plus section 4's sidebar and repository view and section 13's Needs you), its design into `docs/architecture.md`, move `Cmd+T` from New query tab to the terminal and add `Option+Cmd+T` and `Cmd+J` in `SPEC.md`'s keyboard table, add both modules to `module_boundaries.rs`, add a roadmap entry, change "A general terminal in Brainiac" in `checkout-runs.md`'s Not in this design to point here, and add decisions at the end of Decisions: "A terminal runs programs the user chose in a pane in the app process, drawn with xterm.js", "herdr is reached through its CLI as a program the user installed; Brainiac maps its sessions and workspaces to Brainiac's and attributes panes to repositories by folder", and the change to the repository rule if it has not already been made for Git operations.

## Sources

Read on 13 October 2026; not yet tested against a build.

- herdr: [repository](https://github.com/ogulcancelik/herdr) (AGPL-3.0-or-later); [Better Stack guide](https://betterstack.com/community/guides/ai/herdr-ai-agent/) (sessions, `--remote`, the CLI while the TUI runs, notifications); [Bitdoze review](https://www.bitdoze.com/herdr-agent-multiplexer/) (state detection, CLI commands, `Ctrl+B` prefix, mouse, license); [Tecmint](https://tecmint.com/herdr-run-ai-coding-agents-in-linux-terminal).
- xterm.js add-ons (fit, WebGL, unicode11, web-links) and `portable-pty` from WezTerm, as compared in `checkout-runs.md`, Open questions.
