# Design: scheduled runs and briefings

Design notes for a run that starts on a schedule on an always-on run host and leaves a short report, a briefing, for the user to read on Today. The motivating case: every morning, check a production environment (Google Cloud and Sentry, through scripts and skill instructions kept in one repository), and turn what is found into a note and proposed tasks or Jira issues.

## Terminology (16 Oct 2026)

This document says "briefing" for the saved definition because it was written first. The visible words are now **Routine** for the definition (what to run, where, and optionally when) and **Briefing** for a result it leaves, as `routines-ux.md` sets out. Read "briefing" below as "routine" where it means the definition, and as "briefing" where it means the report or `.brainiac/briefing.json`.

## Status: proposal (15 Oct 2026)

Phase 3 is built (16 Oct 2026): briefings, the `briefing.json` result and its checks, Run now, and the Today card with Create Task and Save as Note (`SPEC.md`, section 6; `architecture.md`, Decisions). Schedules (phase 4) are not. How briefings and their schedules are listed, opened and read is redesigned in [`routines-ux.md`](routines-ux.md) (proposal, 16 Oct 2026); the Today block built so far is the first slice that design replaces. Phase 1 is started (15 Oct 2026): the controller's `Health` request and a Health section on a run host's page (`SPEC.md`, Remote hosts; `architecture.md`, Decisions). The Machines pane of its own, the last hour kept in memory, and everything else here are not built, and none of it is in scope for v0.6. It reverses three lines in [`agent-runs.md`](agent-runs.md) and [`secrets.md`](secrets.md) (below), so it needs its own decision in `architecture.md` before any code.

## Why

A run today starts when the user presses a button, and a remote controller keeps working while the Mac sleeps. A morning check is the same work started by the clock instead of the user. Doing it on the host removes the need for the Mac to be awake; the briefing waits there until the app connects.

## What the user gets

- A **schedule** on a run profile: repository, prompt or skill, days and time, the host. Edited in the app; kept and fired on the host.
- **Run now** from the same profile, which is the first thing to build and most of the value.
- A **briefing** on Today: when it ran, a status (nothing found, findings, or failed), the findings with their Sentry links, and proposed tasks or issues. It can be saved as a note.
- A **failed or missed run is shown as such.** A missing briefing never reads as "no errors".
- Nothing outward-facing happens unattended. Jira issues and tasks are proposed and created when the user approves.

## Design

### The host owns the schedule

The app edits schedule records and sends them to the controller, which fires runs, collects results, and keeps them until the app connects, as it already does for a run that ends while the Mac is away. The Mac being asleep at 07:00 changes nothing. If the host was down at the time, the schedule records a missed run; it does not start one late unless the profile says a late start is fine.

A scheduled run is still a run of the existing kind: same container, deadline, journal, and collection. The only new thing is who presses Start, and that the start is not confirmed by the user at that moment. So the profile is approved once, when the schedule is saved, with the disclosure New run shows today (code, prompt, provider, host, network policy, credential scope), and a change to any of them disables the schedule until it is approved again.

### The briefing result

The agent writes `.brainiac/briefing.json`; Brainiac reads back only that file from the container's archive and validates it before showing anything, as it does for `.brainiac/explanation.json` (`code-explanations.md`). Shape, to be defined in `models.rs` when built: status, a summary, findings (title, severity, a link, a stable key such as the Sentry issue id), and proposed items (title, body, kind, the finding key). Unknown fields are dropped; sizes are bounded. The stable key lets Today say "seen yesterday" and lets Create skip an item already created.

### Memory and earlier results

Decided (16 Oct 2026). A briefing is useful across days only if a run knows what the last ones found. Two things carry that, both in the run's prompt, as data and never as instructions:

- **Memory:** bounded text (16,000 characters) on the briefing that the agent may replace by writing `memory` in `briefing.json`, and the user may edit. Absent in the file means unchanged; a user edit made while a run is going wins. It is in `brainiac.db` with the definition, so a backup holds it. It is not a file copied into the container: the container receives nothing new, and the controller needs no new input.
- **Earlier results:** the last N ready results (0 to 10, three by default), as dates, summaries and the keys and titles of their findings, cut to 6,000 characters.

Both come from earlier runs, so both can carry text that came from outside (a Sentry message, a web page). The prompt labels them as data, the agent's own file contract is the only thing that makes Brainiac act, and nothing in them is run or fetched by Brainiac. When schedules fire on the host (phase 4) the host needs the memory and the earlier results too; the definition then moves to the host with them.

### Result vocabulary

A briefing is not only about problems. `status` is optional (`nothing_found` and `findings` are claims about a check; no status is a report), `severity` is optional, and a proposal may be a task, an issue, or a note. A result with no usable file is still a failure and never a report.

### Continuing a run later

Not built; the design keeps it possible. A result stores its `run_id`, and following a run (reading the file) is separate from ending it (Finish, or Cancel when Finish refuses). A later setting could keep the run open for a few minutes after the file is read, so the user can continue its conversation from Runs, at the cost of the container and the host secrets staying alive. A scheduled run would rather start a new run from the same commit with the briefing as context than keep one open, because an agent session does not outlive its container.

### Credentials on the host

Decided (15 Oct 2026): credentials live in the **host's own local credential store**, Linux hosts only for now. Google Secret Manager is not used in this design; it stays a later source (`secrets.md`, phase 2). This is the real change. Today a credential is resolved on the Mac, handed to the controller's memory once, and never stored there; the controller "cannot resolve Mac credential sources or renew credentials while the Mac is away" (`agent-runs.md`, Credentials and disclosure). An unattended run has no Mac to ask.

The user keeps each secret in one place, Settings → Secrets on the Mac, with the source it has today (Keychain, Ask, a command, an environment variable). The host's page (Settings → Agents, where run hosts are) has a **Secrets on this host** section: **Share** resolves the secret on the Mac, with the user present and any unlock prompt, and sends the value over the control channel to the host, which stores it in its local store. The host never sends it back. Sharing is a copy, not a link: the host keeps working when the Mac is off, and a rotated value reaches it only on **Share again**. The app records, per host, what was shared and when, and marks a share out of date when the saved source changed since (the app cannot read the host's copy). **Remove from host** deletes it there. Settings → Secrets lists "shared with <host>" on each secret. Sharing shows what the host's operator can read, and the profile records the credential's name and host, never the value (`secrets.md`). The controller reads the value when a run starts, hands it to the container as today (bootstrap stdin, never Docker's environment), and keeps nothing afterwards.

A Sentry token is neither an account's token nor a connection's password, so Settings → Secrets gains a plain named secret (a label, a source, a scope note) that can be shared; the profile names which ones a run receives.

- **Model credential: the profile's existing choice.** A profile already delivers exactly one of a Claude plan token or an API key (`agent-runs.md`, Subscription token). A scheduled run uses its profile's choice. The Claude plan option stays subject to the open terms question there.
- **Sentry and Google Cloud credentials:** named secrets (above) chosen on the profile, delivered to the container under names the descriptor approves. The Google Cloud one is a service account key for the checks themselves, with viewer roles only.
- **Which local store.** Decided (15 Oct 2026): `systemd-creds --user`, which encrypts a value so that only that user on that machine can decrypt it, with the ciphertext as a file in the controller's state folder. Tried on the target host (Ubuntu, systemd 259) as the controller's user `brainiac`: a value encrypted and decrypted without root or a login session. Still to confirm on that host: a stored value decrypts again after a reboot. `secrets.md` plans the Secret Service for a desktop Linux host later, and a host without `systemd-creds --user` cannot take secrets, with the reason shown; there is no plain-file fallback.
- **What this saves.** No Google authentication changes, no service account that exists only to read secrets, and no second network hop at 07:00. The cost is that rotation means sharing again from the app, and there is no central audit log.
- The home server is reached over Tailscale with the system SSH, verified host keys, and the controller as a service, as in `agent-runs.md` (Remote sessions); a private network is not a reason to loosen any of that.

Each share on a host has a visible state, in the host's Secrets section and on the profile that uses it:

- **Shared**, with the date.
- **Out of date**: the saved source changed on the Mac after the share. Share again.
- **Not on this host**: the profile names a secret the host does not have (never shared, removed, or the host's store was reset). A schedule that needs it is paused with that reason, not run without it.
- **Failed**: the last Share or removal did not finish, with a safe reason (host unreachable, no credential store on the host, disk full) and a Retry. A failed share is never shown as shared.
- **Rejected at run time**: a run whose provider or API refused the token ends as failed with the credential named, and the share is marked rejected until it is shared again. This is the only way a rotated or revoked token is noticed, since the app cannot read the host's copy; the doc does not claim otherwise.

Scope rules that follow, because anyone with access to the host can read what it holds:

- **Read-only, narrow tokens.** Sentry: read scope on the project. Google Cloud: a dedicated service account with viewer roles only, not the user's own login. The agent can copy a delivered key into its output (`agent-runs.md`), so a credential that could do harm is not delivered.
- **No Jira write token in the run** (below).
- The profile shows each credential's scope at approval, and warns from the same age as the subscription token.

### The tools layer

Decided (15 Oct 2026). The run image today is Node 22 on Debian with `git`, `ripgrep` and the two agents (`src-tauri/src/agents/image/Dockerfile`), and it is one image for every run. The morning check needs Python and the Google Cloud CLI (`gcloud`). Adding them to that image would make every run's image and build larger for a tool most runs never use, and installing at run time would also make a run depend on the network at its start.

So Brainiac builds an optional **tools layer** on top of the base image, from a Dockerfile compiled into Brainiac like the base one, with the same rules: pinned base digest, pinned packages, the build context is those files and nothing else, and a repository's own Dockerfile or `requirements.txt` is never read at build.

- The layer adds `python3` (Debian's, with its standard library) and `gcloud` at a pinned version from Google's package source. It installs no Python packages: the first script this was designed for imports only the standard library (`argparse`, `json`, `os`, `re`, `sys`, `time`, `urllib`), so there is no `pip` step, no package index to reach at build, and nothing to pin there. A fixed list can be added later if a script needs one.
- Built and tried on 15 Oct 2026 (`src-tauri/src/agents/image/tools/Dockerfile`, not yet wired into Build image): Python 3.11.2 and Google Cloud CLI 589.0.0 run as the unprivileged `node` user, the first script's imports all resolve, and the layer adds about 640 MB to the 1.07 GB run image. The apt repository's signing key is pinned by SHA-256 in the Dockerfile and the package by version.
- A profile ticks **Python and Google Cloud CLI** to use it. Other profiles keep the base image. The layer has its own recipe hash, tag and current/rebuild state, shown next to the base image on the host's page, and **Build image** builds both when the profile needs the layer.
- The layer is part of what a schedule's approval records (image digest), so a rebuild asks for approval again.
- The user's scripts live in the repository, which the run already has. A script that imports a third-party package will not find it; that is a limit of the layer, shown in its description, until a list exists.

### The network allowlist is out of scope

Decided (16 Oct 2026): this work does not include the network allowlist. It was a prerequisite in the first draft; the maintainer moved it out. The consequence is plain and is shown, not hidden: a run that holds host secrets has the same unrestricted egress as any run (`agent-runs.md`, Isolation and permissions), so a prompt-injected run could send a secret it holds to any address. What limits that, until an allowlist exists:

- Only secrets the user names on a profile reach a run, and only on a host the user shared them to.
- Use tokens that can read and nothing more; the Share section says so.
- New run and the profile list the secrets a run will get, next to the unrestricted network disclosure.
- Schedules (phase 4) show the same disclosure in their approval.

The allowlist remains a later, separate design (`agent-runs.md`). It is not a gate for schedules.

### Outward writes are proposals

A briefing may propose tasks (Brainiac's own, created on approval through the task service) and Jira issues. Creating a Jira issue is a write to an outside service, so it happens on an explicit user action, like review, approve, and merge in v0.3. Two ways to perform it, to choose later: a Jira adapter in Rust (new work, would also serve the Linear/Jira reference work in `task-connections.md`), or a script the user runs. Neither puts a Jira write token in the container.

Decided (15 Oct 2026): every Jira issue is created only after the user approves it. There is no unattended creation and no opt-in for it in this design.

### Cost and duration

An unattended daily run is a recurring cost, so it is shown:

- **Per run:** how long it took and, on an API key, its token usage and cost as the adapter reports them; on a Claude plan, "uses your plan" with its usage as reported (`agent-runs.md`, Subscription token), never a dollar figure computed at API prices.
- **Per schedule:** the last run and a running total for the current month.
- **A limit that stops a runaway run:** every scheduled run has the run deadline (required, finite, shown at approval) and a cost or usage ceiling where the adapter reports usage. Hitting either ends the run as failed with its partial briefing kept, and Today says which limit it hit.
- **Pause** on a schedule, and Pause all on a host, from the app.

### Failure and silence

Today shows the last scheduled run's outcome and next time: ran, failed (with the reason), or missed (host unreachable or off). The app marks a schedule overdue when its host has not reported a run within a window of the scheduled time, so silence is visible. A notification channel on the host (email or a webhook) is later work.

## Machines: the first slice

Scheduled runs need a host to be more than an entry in Settings → Agents, so this adds the second role on a machine that `architecture.md` (Machines) anticipates: a Machines pane of its own, where the host's secrets are shared and its health is shown. It is not a general machine manager. There is no remote shell (decided 15 Oct 2026; a terminal tab running `ssh` already does that), no file transfer, and no arbitrary commands.

- **Machines pane.** Each machine: name, address, the approved host key, whether it is reachable, the controller's version, and the roles on it. Add host and its approval stay as they are; the pane moves out of Settings → Agents, with a migration-free move since the data is in `machines` already.
- **Health.** A live view for one machine, like a database's Health (`SPEC.md`, Health): not monitoring, no alerts, sampled only while the tab is visible and the window is on screen, with the last hour kept in memory. It shows what the user needs before trusting a 07:00 run: reachable and since when, uptime, load and memory, disk used on the volume holding the engine and run workspaces, the engine's state and version, the controller's state and version, the runs on it now and their limits, and the time of the last scheduled run and the next. A problem is named with what to do (disk almost full, engine stopped, controller older than the app).
- **How it is collected.** The controller answers a bounded `health` request with a fixed set of fields it reads itself, over the control channel it already has. It is not a shell and not an Engine API relay (`agent-runs.md`, Host setup and authority). A host with no reachable controller still shows what plain SSH can tell: reachable or not, and the host key.
- **Secrets on this host** (above) and the schedules on it live in the same pane.
- **Overdue.** Health's last-seen and each schedule's last and next run are what makes "the host is down on a Monday" visible, which the panel named as the main risk.

## Phases

1. **Machines pane and Health:** the pane, reachability, and the controller's `health` request. Useful with runs on a host today, before any schedule.
2. **Secrets on a host, and a profile's choice of them:** write-only secrets stored on a Linux host and read by its controller when a run starts, with the share states; named secrets in Settings → Secrets.
3. **Briefing and Run now:** the `briefing` result kind, validation, the Today card, Save as note, Create task. No schedule yet.
4. **Schedules on the host:** records, firing, missed-run handling, approval and re-approval, cost and pause, overdue display.
5. **Later:** a Jira adapter for approve-and-create, notifications, several schedules per profile.

## Not in this design

- Scheduling on the Mac (it sleeps), or a hosted service.
- Unattended writes to Git, pull requests, Jira, or any outside service by default.
- Starting a follow-up turn automatically, or a run that continues from yesterday's.
- Production access beyond read-only tokens; the user's own gcloud login in a container.
- Recurring tasks and reminders in Brainiac itself (`roadmap.md`), which this does not need.

## Open questions

| Question | Decides |
| --- | --- |
| Does a value stored with `systemd-creds --user` still decrypt after the host reboots? Works as the controller's user without root (tried 15 Oct 2026) | Whether the store survives a restart |
| Does the skill need anything beyond one repository and API access? | Whether a multi-repository run is needed (the design assumes not) |
| Does the model credential for an unattended run (a `claude setup-token` token or an API key) fall under the open terms question in `agent-runs.md`? | Whether a Claude plan may power scheduled runs at all |
| How long may a host be silent before a schedule is overdue? | The overdue window |
| How are extra secrets (a Sentry token, a Google key) delivered? The bootstrap contract carries the model credential and "descriptor-approved keys" exported as environment variables, and phase 1 injects no registry tokens (`agent-runs.md`, Credentials and disclosure) | The descriptor and entrypoint change that phase 2 needs, including trace filtering of each new value |
| The Google service account key is a JSON file. Deliver the JSON as one environment variable that the script passes to the client library or writes to tmpfs, or let the entrypoint write a file to tmpfs? | Whether the delivery contract stays environment-only |
| Which third-party Python packages might other scripts import? Resolved for the first script (standard library only); decide a fixed list only when a second script needs one | Whether the layer ever needs a `pip` step |
