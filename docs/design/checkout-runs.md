# Design: agent runs in your checkout

Design notes for a second place an agent run can work: **In your checkout**, beside v0.5's **In a container**. A checkout run starts Claude Code (and later Codex) on this Mac, in the working tree of a repository Brainiac tracks, so the user develops that project with the agent from inside Brainiac. The agent is the real CLI in a terminal pane beside the repository's diff. Brainiac adds what a terminal cannot: one place that shows which agent is waiting, what each turn changed in the folder and in Git, and what the run costs. Nothing here is on the roadmap or in scope for any release yet.

This is a deliberate exception to Brainiac's first rule. Until now the app only reads a user's repository (fetching aside), and v0.5's runs work on a copy in a container so that stays true. A checkout run edits the checkout itself, because that is the point: the agent works on the project the user is working on, with their tools, their uncommitted changes, and their editor open on the same files. The design keeps the exception narrow, off until the user turns it on, and named everywhere it applies.

It builds on v0.5's New run, Runs list, and run controller (`brainiac runner`) (`SPEC.md` section 13; `docs/architecture.md`, Agent runs — v0.5), the repository view and its live working-tree diff (section 4), v0.6's once-per-repository question before code is sent (section 14), and the credentials layer (section 12).

UX Design Artifact: https://claude.ai/artifact/NQGRrb5KszwQbXhHNLfx4A

## Status: proposal (9 Oct 2026)

Nothing is built. The first draft (an ACP view drawn by Brainiac, as a separate "session" feature) was reviewed by the client panel and replaced by this one; the panel then reviewed this design and its canvas, and this version takes in that round. What the panel said, below, records the three rounds. The open questions at the end are gates, not details to settle while building.

## Why

- **Containers are for work you hand off; the checkout is for work you do.** A container run is good when the user wants a result to review later, isolated from their machine. Most day-to-day agent work is not like that: the user is in the middle of a change, has uncommitted edits, runs the app and its database locally, and wants the agent to help with exactly that state. A container run cannot see uncommitted changes, has only the image's tools, and ends in a patch the user has to apply.
- **The terminal does not show what is waiting.** Today the user runs `claude` or `codex` in a few terminal tabs next to Brainiac, one per worktree or client, and checks each to find out which one is waiting for them, what it changed, and what it cost.
- **The user's own setup.** On the Mac, the agent uses the CLI the user installed and the login they already have, their PATH, toolchains, SSH agent, MCP servers, slash commands, and plugins. Nothing to build, no image, no engine.

## What the user gets

- **New run… asks Where:** **In a container** (an isolated copy of one commit; ends in a patch to review: today's run) or **In your checkout** (works in your folder as it is, on this Mac, with your tools and uncommitted changes; it does not switch branches). The choice is remembered per repository. In your checkout is offered only when it is turned on in Settings → Agents (The repository rule). The same two names are used everywhere: New run, the run's header, the Runs list, and notifications.
- **No time limit.** A checkout run lasts until the user ends it, like a terminal. New run says so, and on an API key that cost keeps growing while the agent works.
- **It opens in the repository view.** A checkout run is a tab of its repository, beside History, Branches, and Changes, so the sidebar keeps the repository selected: the real `claude` (or `codex`) in a terminal pane on one side, the run's changes on the other. Runs lists it too, and a row opens that tab. Everything the CLI does works as in Terminal: slash commands, `/model`, `/compact`, `/rewind`, Esc to interrupt, Shift+Tab to change mode, image paste, plugins. Closing the tab or the window does not stop the agent; reopening the run reattaches to it.
- **Around the pane, Brainiac shows:**
  - what the agent is doing, from the CLI's own hooks: **Working, turn n**, **Waiting for you** (with what it asks: a command, or an edit as removed and added lines), **Ready for your prompt**, **Ended**, and **No update for n min** when the hooks go quiet;
  - **the mode it is in now** (Ask before actions, Edit without asking, Plan), read from every hook, not the one it started in;
  - **This run** (the folder now against when the run started) and **By turn** (each turn's changes, and the commands it ran), with **Copy patch** for either;
  - **Git during this run:** commits made, the branch switched, and pushes Brainiac can see;
  - the model, and the cost: an estimate on an API key, "Claude plan" on a plan; in the header and in the Runs row;
  - a macOS notification when the agent waits for the user and Brainiac is not the active app.
- **Needs you counts only a run that waits for an answer.** A checkout run idles between prompts all day; "ready for your prompt" is shown on its row but not counted. Its row says the answer is given **in the pane**, so it is not mistaken for a container run's request, answered in Brainiac.
- **Coming back.** Quitting Brainiac does not end a checkout run; the controller keeps the CLI running. It stops when the user ends it or quits the CLI, when they log out or restart, or when the CLI crashes. Then the run shows **Interrupted** or **Ended**, and **Resume** opens the CLI's own resume (`claude --resume <id>`, `codex resume <id>`), which continues the same conversation in the mode it was last in. A turn cut off by the stop is marked **cut off** in By turn, with the folder as it was found.
- **End run** stops the CLI and everything it started. The changes stay in the folder, because they are the user's now.

## The repository rule

Today's rule: the app never writes to a repository, except fetching. Checkout runs change it to:

> Brainiac itself never writes to a repository, except fetching. A checkout run the user starts runs an agent that edits the working tree it was started in, with the user's permissions, until the user ends it.

- **Off until turned on.** Settings → Agents → **Runs in your checkout** is off by default. Turning it on shows this rule and what the agent can reach (Authority, below). Turned off, New run offers only In a container, and Brainiac only reads, as before. A team that wants Brainiac read-only leaves it off.
- **Brainiac's own code runs no Git write.** No checkout, commit, stash, index change, worktree, or branch. It reads the folder (status, diff, `HEAD`, branches, snapshots) as the Git viewer does. The writes are the agent's: file edits and whatever commands it runs.
- **The agent can run Git, including on your work.** The CLI commits, switches branches, and pushes when asked or when it decides to; a push uses the user's SSH agent or credential helper. A `git commit -a` or `git add -A` takes in the user's own uncommitted changes from before the run, and a push publishes them. Before you start says so in those words, and New run repeats it next to the count of uncommitted files.
- **The repository's hooks run.** A commit the agent makes runs the repository's Git hooks, and Claude Code runs the repository's `.claude` hooks, on the Mac, as the user, as in Terminal.
- **Only folders the user chose.** A checkout run starts only in a working tree Brainiac tracks, from New run, never from an agent's request or over MCP. Agents (section 9) cannot start runs.
- **Undo is the CLI's, and it is partial.** Claude Code's checkpoints (`/rewind`, Esc twice) restore the files its edit tools changed. They do not undo what a command did, a commit, or a push. Brainiac says exactly that wherever it mentions `/rewind`, and does not write files back into the folder; a later Revert this turn would be a Brainiac write, and is listed under Not in this design.

## Design it twice

### What the user talks to

| | A. The real CLI in a pane (chosen) | B. Brainiac's own view over ACP | C. Both, with a toggle |
| --- | --- | --- | --- |
| What it is | A pseudo-terminal running `claude` / `codex`, drawn with xterm.js | The agent's ACP adapter as a child process; Brainiac draws the conversation, as container runs do | A for those who want the CLI, B for those who want guidance |
| Fidelity | Every CLI feature, the day the vendor ships it | What ACP carries: no slash commands, `/model`, `/compact`, `/rewind`, plugins, Esc, image paste | Both |
| Brainiac knows | What the CLI's hooks report: turns, waits and what they ask, edits, commands, the current mode, session ID | Everything, as normalized events | Both, with two sets of states |
| Permissions | Answered in the pane, by the CLI's rules; Brainiac sets the starting mode and disables bypass | Answered in Brainiac; Brainiac enforces the level | Two permission models |
| Resume after restart | The CLI's own `--resume` | ACP `session/load`, if the adapter supports it | Both |
| Model, usage | The CLI's own (`/model`, its status line); Brainiac reads model and tokens from hooks and the transcript | Brainiac must build them | Both |
| Reuse | Run controller, Runs list, snapshots; new: pseudo-terminal, xterm.js, hooks | `AcpClient`, journal, run view, permission UI | Everything, twice |

**A for In your checkout; container runs keep Brainiac's view**, because a container has no terminal for the user. Two of the panel's three personas would leave a view that lags the CLI's weekly releases ("Continue in Terminal just admits it"), and A removes the first draft's biggest gaps (resume, model, usage, prompt fatigue) by using the CLI's own answers. What A costs is the learner's guided permission view; the pane narrows that by showing, beside the terminal, what the agent is waiting for, with the edit as removed and added lines and a note for Git commands (Permissions, below), so the prompt in the pane is not the only place to read it. B in the checkout (option C's toggle) is added later only if real learners need it.

### Where the agent works

| | The tracked working tree (chosen) | A new worktree per run | A copy in Brainiac's data folder |
| --- | --- | --- | --- |
| Sees uncommitted work | Yes | No (starts from a commit) | No |
| User's editor sees the changes live | Yes | Only if they open the worktree | No |
| Two runs at once | Conflict | Independent | Independent |
| Brainiac writes to the repo | No | Yes: worktree, branch | No; this is In a container |

The working tree, because that is what developing the selected project means. **One live checkout run per working tree.** For parallel agents the user makes worktrees with Git (`git worktree add`); Brainiac already recognises linked worktrees (section 4), and New run lists each one, with its branch, as a place to run. Brainiac does not create worktrees: that would be its own write.

## How the pane works

```text
Main WebView ── xterm.js pane ── Tauri commands ──▶ AgentRunService (where = checkout)
                                                     ├─ CheckoutSnapshots (the folder at start and at each turn, in Brainiac's own object store)
                                                     ├─ CredentialService (only when paid with an API key)
                                                     └─ RunRuntime ── control socket ──▶ RunController (brainiac runner)
                                                                                           ├─ HostTerminal: pseudo-terminal, scrollback, attach
                                                                                           │    └─ claude --session-id … --settings <data folder>/…/settings.json
                                                                                           └─ hook events ◀── brainiac hook <event> (the CLI's hooks)
RunGuard ── ends the controller's terminals' process groups when the controller dies
```

- **The controller owns the terminal.** `RunController` already outlives the app and owns each container run's session. A checkout run's workload is a pseudo-terminal instead: the controller opens it, starts the CLI on it directly (an argument array, no shell: exiting the CLI ends the run, and there is no prompt to type other flags at), in its own process group, with the chosen folder as its working directory. It keeps the last 10,000 lines of output so a reattaching pane redraws, forwards keystrokes and resizes from the attached pane, and allows one attached pane at a time. A `Workload` interface hides the difference between this and a container (start, attach, stop with grace then force, wait). The controller's ten-minute exit counts a checkout run as live while its CLI runs.
- **Launch.** `claude --session-id <run's UUID> --settings <file> --permission-mode <mode> [--model <model>]`, and the first prompt, if given, typed into the pane once the CLI is ready. Codex: `codex --cd <folder> [--model …] --sandbox workspace-write --ask-for-approval on-request` with Brainiac's hooks (Codex, below). **Resume** passes `--settings` again, because the CLI does not restore it with the session, and `--permission-mode` set to the mode the last hook reported. A conversation resumed by hand in Terminal runs without Brainiac's hooks and is not part of the run.
- **Brainiac's settings file** lives in the run's folder in Brainiac's data folder, never in the repository, and **Brainiac's hooks** in the run's header shows it. It holds only hooks and `permissions.disableBypassPermissionsMode`. Each hook runs `brainiac hook <event>` (the app's own executable, like `brainiac mcp`), which forwards the hook's JSON to the controller and exits at once with no output, so it never blocks, answers, or changes what the CLI does. Settings passed with `--settings` sit above the repository's own and below managed settings; hooks from every source all run, in parallel, so Brainiac's hooks neither replace nor are replaced by the repository's (Claude Code docs, Settings and Hooks).
- **What the hooks give Brainiac.** Every hook's input carries the session ID and the current `permission_mode`, which is how the header shows the mode the CLI is in now.

  | Hook | Brainiac shows |
  | --- | --- |
  | (process started, no hook yet) | Starting |
  | `SessionStart` | The CLI's session ID and transcript path, kept for Resume |
  | `UserPromptSubmit` | Working, turn n; a snapshot of the folder before the turn |
  | `PermissionRequest` | Waiting for you, with the tool and its input (the command, or the edit as removed and added lines); counts in Needs you; a notification |
  | `Notification` (`permission_prompt`, `idle_prompt`, `agent_needs_input`, `elicitation_dialog`) | Waiting for you when no `PermissionRequest` came first; a question from the agent shows as Waiting for you · question |
  | `PostToolUse` on edit tools | The files the agent's edit tools changed (tagged **agent edit**); the diff refreshes |
  | `PostToolUse` on `Bash` | The command it ran, listed with its turn; Brainiac reads the folder's `HEAD` and branches after it, for Git during this run |
  | `Stop` | Ready for your prompt; a snapshot after the turn |
  | `SessionEnd` | Ended |
  | (process exited without `SessionEnd`) | Interrupted |

- **When the hooks go quiet.** While a turn is Working, a hook arrives at least with every tool call. After 5 minutes without one, the state reads **Working · no update for n min**, in a neutral style: a long test run is silent too, so Brainiac does not call it stuck. Sleep, a lost network, and a CLI retrying a request all look like this. A **plan limit** shows as its own state only if the CLI reports it through a hook (Open questions); otherwise it is visible in the pane alone, and the row keeps its last state.
- **Codex** has its own hooks with the same events (`SessionStart`, `UserPromptSubmit`, `PermissionRequest`, `PostToolUse`, `Stop`, `SessionEnd`), so a Codex run gets the same states. Codex asks the user to review and trust a hook that is not managed, by a hash of its definition, in its `/hooks`: the first Codex run shows that prompt in the pane, and Brainiac's hook definition stays the same across releases so it is asked once. How Brainiac passes hooks to Codex without writing the user's `config.toml` or the repository's `.codex/` is a spike; if it cannot, Codex falls back to its `notify` program (turn completed only), and the row says "Waiting is not reported" rather than leaving Needs you silently empty.
- **Keys.** While the pane has focus, every key goes to the CLI except Brainiac's Command shortcuts (`Cmd+K`, `Cmd+B`, `Option+Cmd+N`, and the rest of the keyboard table in section 4) and **`Cmd+Esc`, which switches focus between the pane and the changes**. The CLI's own keys are Control, Escape, and Shift+Tab, so the two do not collide. Option is sent as Meta, a setting, for CLIs that use it. In By turn, ↑ and ↓ move between turns.
- **Image paste** works because the CLI runs on the Mac: Claude Code reads the image from the clipboard itself when the user presses its paste key in the pane.
- **The conversation lives in the CLI's own store** (`~/.claude/projects/…` for Claude Code, `~/.codex/sessions/` for Codex), as it does in Terminal, where the CLI removes it after its own retention (30 days by default for Claude Code), so Resume can fail for an old run and says so. Brainiac keeps the run, the hook events (turns, their times, waits, files, commands, modes), the snapshots, and the pane's scrollback while the run lives; not a copy of the conversation. Later features that need its text (Save as note, Explain these changes) read the transcript at the path the CLI reported, on that explicit action only; the transcript's format is internal to each CLI and can change, so those features fail visibly rather than guess.

## Changes and snapshots

The repository's Changes tab shows everything uncommitted, including what the user had before the run and what they edit by hand meanwhile. The run's own panel answers "what happened in this run":

- **A snapshot at start**, before the CLI starts: the folder's tracked and non-ignored files, written into Brainiac's own bare repository in its data folder, borrowing the user's objects through `alternates` as container runs already do, with a temporary index outside the repository. The user's index, refs, `HEAD`, object store, and configuration are not touched. A snapshot is a tree, not a commit in the user's history.
- **A snapshot at each turn's start and end**, from the hooks. The panel has three tabs:
  - **This run**: start → now.
  - **By turn**: each turn's before → after, its prompt, the commands it ran, and **cut off** for a turn that never reached `Stop`.
  - **All uncommitted**: the folder against `HEAD`, as in Changes, with a line saying how many of those files were already changed before the run.
- **Who changed a file.** A file changed by the agent's edit tools is tagged **agent edit**. Any other change is tagged **other change**, explained once: "not made by Claude Code's edit tools: a command it ran (such as a formatter) or an edit of yours". Brainiac does not guess which; By turn lists the turn's commands next to it so the user can tell. The panel's note says "changes in this folder since the run started, including any edits of yours".
- **Git during this run.** After every `Bash` command and every turn, Brainiac reads `HEAD`, the branch, and its upstream (read only). The panel lists commits made since the start (with their files), a branch switch, and a push it can see because the remote-tracking branch moved. After a commit, This run still compares with the start, so committed work stays visible; All uncommitted no longer shows it, and the panel says why.
- **What a snapshot holds.** Untracked files that are not ignored are copied into the snapshot: an unignored `.env.local` is then in Brainiac's data folder too. Before you start says so, deleting the run deletes its snapshots, and ended checkout runs follow runs' 30-day removal. A snapshot that would exceed the run's limit (2 GB, a setting) or the disk's free space is skipped, and the panel shows **Snapshot skipped: over 2 GB** (or **disk full**) on that turn; the run is not stopped.

## Authority and disclosure

A checkout run runs as the user. Unlike a container run, nothing stands between the agent and the Mac.

- **What the agent can reach:** every file the user can, the user's SSH agent and Git credentials, Keychain items the user's processes can read without a prompt, the network and local services, and the user's own MCP servers. Its commands, the repository's scripts, and the repository's Claude Code hooks run with the same reach. The Settings switch lists all of these.
- **The first run in a folder is three steps, in this order,** each labelled "Step n of 3":
  1. **Before you start**, in New run: the agent works in this folder as you; it sees your uncommitted changes and can overwrite them, commit them, and push them with your credentials; the repository's hooks run on this Mac; untracked files are copied into Brainiac's snapshots. After the first time it is one line with **Show details**.
  2. **May this code go to this provider?** v0.6's question (section 14), once per repository or workspace and per provider, with the same rule that a No wins. The answers are shared with Explain and listed in Settings → Agents (Code sent to providers) as well as Settings → Explanations. A No blocks New run in your checkout for that provider.
  3. **What this repository runs on your Mac** (folder trust), once per working tree, or once for a workspace, in plain words with a tag per item: **Runs commands** (hooks), **Skips asking** (allow rules in its settings), **Downloads and runs code** (an MCP server started with `npx`, `uvx`, or a similar runner), **Starts a program** (any other MCP server), **Instructions** (`CLAUDE.md`, `AGENTS.md`). Trusting a workspace trusts what each of its repositories runs today; a repository added later, or one whose list changes, is asked again. Both trust buttons also start the run, and say so.
- **Notifications** name the repository and branch ("widgets · feature/csv-export is waiting for you") and the kind of request with the program name only ("asks to run git"), never a command's arguments, which can hold a password.

## Permissions

In the pane, the CLI's own permission rules apply, and the user answers in the CLI as in Terminal. The design says so rather than implying Brainiac enforces a level:

- **Brainiac chooses the starting mode, Ask before actions by default:** **Ask before actions** (Claude Code `default`; Codex `on-request` approvals) or **Edit without asking** (`acceptEdits`; Codex `workspace-write`), from Settings or New run. The user can change it in the CLI (Shift+Tab, `/approvals`), the header shows the mode the CLI reports, and that mode is what holds.
- **Bypass is switched off.** Brainiac's settings file sets `disableBypassPermissionsMode`, and the CLI is not started from a shell where other flags could be typed, so a checkout run cannot enter Claude Code's skip-all-permissions mode. Codex's own `/approvals` can still choose full access; for Codex, the row says the CLI's choice holds.
- **The repository's permission rules apply on top**, as in Terminal: an allow rule in its `.claude/settings.json` lets a command run without asking. Folder trust lists such rules under **Skips asking**.
- **Git commands get a note.** When the agent waits to run a command whose argument list starts a Git command that changes the repository (`commit`, `add`, `push`, `switch`, `checkout`, `reset`, `rebase`, `merge`, `stash`, `clean`), the card beside the pane says what it does to this folder in one line: "Commits every change in the folder, including the 2 files you changed before the run", "Pushes feature/csv-export to origin with your credentials". Brainiac recognises only these, from the command text; a command hidden inside a script or `bash -c` gets no note, and the card says "Brainiac explains Git commands only" so the absence of a note is not read as safe. Other commands show as they are.
- **Answering from Brainiac** (Allow once and Reject beside the pane, writing the answer back to the CLI) is not offered: a hook can answer a permission, but Brainiac would then be deciding for the CLI, and two places to answer the same question invites mistakes. **Answer in the pane** moves focus to the pane, on the question.
- **Teams.** Rollout is per developer: each turns on Runs in your checkout and chooses a default mode. A team that wants to enforce a mode, deny commands, or forbid bypass for everyone uses Claude Code's own managed settings, which win over Brainiac's and the repository's. A team policy in Brainiac waits for shared workspaces (`shared-workspaces.md`).

## Credentials and usage

- **The CLI's own login, by default.** Claude Code reads its own login, Codex reads `codex login`. Brainiac never reads, copies, or stores them, and never runs a browser sign-in. Setup checks that the CLI is installed and signed in by running its own status command.
- **Or an API key per workspace** through the credentials layer (owner `agent:<profile id>`): Settings → Agents → This Mac has a row per workspace choosing the CLI's login or one saved key, so each client's work is billed to its own key; New run shows the one it will use and can change it for the run. The key goes into the CLI's environment when the controller starts it. On the Mac that environment is visible to the user's own processes; the disclosure says the agent and its commands can read it.
- **Cost in the header and the Runs row.** On an API key, Brainiac estimates the cost from the transcript's token counts at the provider's published prices, labelled **≈ $1.84** with "estimate" on hover, and updates it at each turn's end; on a plan, "Claude plan" or "ChatGPT plan". The model is the one New run chose, or the one the CLI reports.
- **Terms.** Whether a public app may start the user's signed-in CLI on a plan is a question for each vendor's terms, like the subscription token (`agent-runs.md`, Subscription token). If the answer is not yet recorded, New run in your checkout offers API keys only and says why.

## Phases

- **Phase 1: Claude Code in your checkout.** The Where choice and the Settings switch; the run as a repository tab with the pane, reattach, keys, and image paste; Brainiac's settings file and `brainiac hook`; states including the current mode and No update, Needs you, notifications, and what the agent waits for with Git notes; snapshots, This run, By turn with commands, All uncommitted, Git during this run, and Copy patch; the three first-run steps; Resume with `--resume` in the last mode; model and cost in the header and row; the CLI's login or an API key per workspace; Settings → Agents → This Mac (the CLI found, signed in, its version, the PATH it will run with, and what the last test checked).
- **Phase 2.** Codex, with its hooks; starting a checkout run from a task, with the task's text as the first prompt and the run linked on the task and in Today; Save as note from the transcript; Explain these changes (v0.6's explain on a run's changes, once 0.6.x covers working-tree changes).
- **Only if needed.** Brainiac's own view in the checkout for learners (option C); Revert this turn; answering permissions from Brainiac.

## Environment

A macOS app does not get the user's shell PATH, so the CLI would not find `node`, `cargo`, or `pnpm`. The controller reads the login shell's environment once per start (`$SHELL -l -c 'env -0'`, with a timeout, as editors do) and starts the CLI with it, plus `TERM=xterm-256color` and the variables Brainiac sets. Settings → Agents → This Mac shows the PATH it found; when the shell prints noise or hangs, it says so and links to how to fix the shell's startup files.

The **test** on that page starts the CLI in a temporary folder and shows what it checked: the hooks reported a turn, the current mode, and a command; skip-all-permissions mode was refused; resume continued the conversation.

## Testing

- A fake CLI on the pseudo-terminal: start, keystrokes and resize, reattach redraw, End reaching a grandchild process, controller death with the guard ending the group, app quit and reattach.
- `brainiac hook` with each event's JSON, a controller that is not answering (the hook still exits at once), and a malformed payload.
- Snapshots in a temporary repository built by the test: the user's index, refs, `HEAD`, object store, and configuration byte-identical before and after; tracked, untracked, ignored, binary, symlink, executable, deleted files; a dirty tree at start; the size limit.
- Git during this run: a commit, a `commit -a` taking pre-run changes, a branch switch, and a push seen through the remote-tracking branch, each in a repository built by the test.
- The Git-command note: each listed subcommand, global options before it (`git -C dir commit`), a chain (`git add -A && git commit`), and `bash -c` getting no note.
- The settings file: its exact contents, and that it is never written inside the working tree.
- A real check by hand before a release, with the user's signed-in CLI: hooks fire with `permission_mode`, bypass is refused, `--resume` continues the conversation in the last mode, image paste works.

## Not in this design

- Brainiac committing, staging, pushing, switching branches, creating worktrees, or writing files back (Revert this turn) for the user.
- Claude Code's skip-all-permissions mode in a checkout run.
- A time limit for checkout runs.
- Checkout runs started by agents over MCP, by schedules, or from a remote request; checkout runs on a remote host.
- Reading or managing the CLIs' logins.
- A team policy enforced by Brainiac (Teams, above).
- A general terminal in Brainiac. The pane runs the agent's CLI only.

## What the panel said

The client panel (three fictional personas: a skeptical CTO, a learner, a freelancer with several clients) reviewed the first draft, then the choice of view, then this design with its canvas. Treat it as hypotheses to check with real users.

- **Round 1, the first draft:** ask before code goes to a provider as Explain does; a switch to keep Brainiac read only; Needs you counting only real waits; resume after a restart; model and cost; retention and what snapshots hold; keys; patch export; per-workspace answers; one live run per working tree with the user's own worktrees.
- **Round 2, the view:** the CLI's own UX matters more than Brainiac's view to the CTO and the freelancer ("My muscle memory is Esc to interrupt, `/compact`, `/review`, and Shift+Tab"); the learner prefers a guided view and accepts the pane if Brainiac shows each turn's changes and takes them to the exact prompt. Brainiac's value is the structure around the agent: "The pane itself adds nothing over iTerm."
- **Round 3, this design and its canvas:** the earlier asks were judged answered. What it found and this version changed: the agent committing the user's pre-run changes with no warning (now in Before you start, New run, and a Git note on the waiting card); command-driven edits blamed on the user ("other change", with the turn's commands); cost missing (now in the header and row, with a key per workspace); the header showing the starting mode (now the current one); no Git activity (Git during this run); no state when hooks go quiet (No update for n min); `/rewind` reading as a full undo; the run opening under Runs although the doc said the repository view; Edit without asking as the unstated default (now Ask); `npx` MCP servers tagged as merely starting a server; command arguments in notifications; "In this checkout" and "In your checkout" as two names, read by the learner as `git checkout`.
- **Where they disagree:** the CTO wants Brainiac to write nothing itself and would make worktrees by hand; the learner wanted Brainiac's own undo. This design leaves undo to the CLI's checkpoints, says what they do not cover, and keeps Brainiac's writes at zero. The freelancer is not yet convinced the run is worth more than iTerm without Today and tasks (phase 2).

## Open questions and spikes

| Unknown | How to resolve | Needed before |
| --- | --- | --- |
| Bypass refused from `--settings` | The docs say `disableBypassPermissionsMode: "disable"` works from any settings scope, but do not name `--settings`: test that it refuses `--dangerously-skip-permissions`, `/permissions`, and Shift+Tab into bypass. Precedence and hook merging are documented | Phase 1 |
| Hook coverage | `PermissionRequest` fires at once for every wait (a command, an edit, a sandboxed network request, a question from the agent; `Notification`'s `permission_prompt` only after about 6 seconds); `Stop` fires after Esc; subagents' hooks carry `agent_id` and must not end the turn; every hook carries `permission_mode` after Shift+Tab | Phase 1 |
| Plan limit | Whether Claude Code reports a plan's usage limit through a hook (`Notification` has `quota_auto_resume_*` matchers) and what it carries; otherwise the limit stays in the pane only | Phase 1 |
| Folder trust | The docs list what runs before an untrusted folder is accepted for `-p` and a trusted parent, not for an interactive first run: test whether the repository's hooks run before the CLI's own trust dialog is answered. Brainiac's question comes first either way | Phase 1 |
| Pseudo-terminal | Crate choice (`portable-pty` or `openpty` directly), xterm.js rendering and input fidelity for Claude Code's TUI (colors, Unicode, mouse, bracketed paste), and reattach redraw | Phase 1 |
| Process group stop | End and the guard reach every descendant: a dev server the agent started, a `nohup` child, a process that calls `setsid` | Phase 1 |
| Snapshot cost and safety | Time on a working tree of 100,000 files, the temporary index and `alternates` leaving the user's repository untouched, the user running `git` at the same time | Phase 1 |
| Usage from the transcript | Token counts per turn in Claude Code's transcript, the price table and its updates, and whether a plan's usage can be shown at all | Phase 1 |
| Git-command note | Parsing a command line well enough for the listed subcommands without a shell (quotes, `&&`, `;`, `git -C`), and what to say for a chain that mixes Git with other commands | Phase 1 |
| Login shell environment | Timeouts, noisy startup files, `fish`, which variables to drop | Phase 1 |
| Terms | Whether starting the user's signed-in Claude Code or Codex from a third-party app is within each plan's terms; record the answers | Before plan login ships |
| Codex | Passing hooks for one run (a `-c` override or another layer) without writing `config.toml` or `.codex/`, the hook trust prompt, `codex resume`, the `notify` fallback's payload, and whether CLI flags win over the user's `config.toml` | Phase 2 |

## When this starts

Move its behavior into `SPEC.md` (section 13's New run, The run, and Boundaries, section 4's repository view, and section 2's rule) and its design into `docs/architecture.md`, change the hard rule in `AGENTS.md` to the wording in The repository rule, and add a decision at the end of Decisions: "Runs can work in the user's checkout when the user turns it on: the real CLI in a pane owned by the run controller, observed through its hooks; Brainiac's own code still writes nothing to a repository except fetching."

## Sources

Checked on 8 October 2026; not yet tested against a live build.

- Claude Code: [CLI reference](https://code.claude.com/docs/en/cli-reference) (`--settings`, `--session-id`, `--permission-mode`), [settings](https://code.claude.com/docs/en/settings) (precedence, managed settings), [hooks](https://code.claude.com/docs/en/hooks) (events, input including `permission_mode`, merging, `Notification` matchers), [sessions](https://code.claude.com/docs/en/sessions) (resume, transcripts, retention), [permissions](https://code.claude.com/docs/en/permissions) (`disableBypassPermissionsMode`, folder trust), [sandboxing](https://code.claude.com/docs/en/sandboxing).
- Codex: [hooks](https://learn.chatgpt.com/docs/hooks), [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) (`notify`, sandbox modes, approval policies).
