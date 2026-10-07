# Design: shared workspaces

Notes on a possible later Brainiac where a workspace can be shared. Its owner invites people, and they work from the same repositories, database connections, saved queries, notes, tasks, agent profiles, and agent runs. The credentials those need are shared as references, and each person can override any of them or keep their own. Nothing here is on the roadmap or in scope for any release. Brainiac today is one person on one Mac with no required account (`SPEC.md`, section 1). Sharing changes the architecture's starting assumption: the unit that owns data today is the installation, and it would become a scope inside it. This file exists so that this change is understood before it is needed, and so that work done until then does not make it harder.

## Status: idea, not planned (7 Oct 2026)

It came up while reviewing the credentials module (`src-tauri/src/credentials/`). There is no need today for more than one account per provider, or for anything shared. The Preparation section lists what this file asks of the code now. Everything else waits until the product exists.

## Why

- **A team works on the same things.** Each person adds the same repositories, the same staging connection, the same saved queries, and the same agent profile by hand. Their copies drift: one has the old host, another the wrong TLS mode.
- **Knowledge and work are shared.** A runbook note, a task list for a release, the query that answers a recurring question, an agent run whose patch someone else reviews: today each stays on one Mac.
- **Credentials are the hard part of onboarding.** A new person asks where the staging password lives, which token GitHub needs, and which command reads it. A shared workspace can answer that once.
- **People differ.** One person keeps their GitHub token in `gh`, another in the Keychain. One has read-only database access, another can write. A shared setup that cannot be overridden per person will be worked around.

## What the user gets (sketch)

- **Share Workspace…** on a workspace: invite by email, with a role (Viewer, Member, Admin; Permissions, below).
- A member who accepts **links** the shared workspace to this Mac. Brainiac matches its repositories to the clones already here by remote URL and offers to locate the rest.
- Everything the workspace shares appears for every member: its connections and saved queries under Databases, its notes in a shared section of Notes, its tasks in Tasks and Today, its agent profiles and runs under Agents. Each item shows that it is shared, and with whom.
- Personal and shared live side by side. A personal note can link to a shared one, a personal task can point at a shared repository, and nothing personal is ever uploaded.
- **Use my own…** on anything that needs a secret sets a personal override, kept on this Mac only.
- Every feature keeps working offline with the last copy, and catches up when the network returns.
- Leaving or being removed from a workspace deletes its shared data from this Mac. It does not, by itself, revoke the secrets a member could read: that happens where the secrets live (Credentials, below).

## What the architecture assumes today

| Assumption | Where | What sharing breaks |
| --- | --- | --- |
| The installation owns all data | One `brainiac.db`, `history.db`, `index.db`, one vault | Data needs an owner scope: personal or one shared workspace |
| A workspace is a list of local folders | `workspace_members.canonical_path` | Paths mean nothing on another Mac; a repository needs a portable identity |
| The vault's Markdown files are the source of truth for notes | `docs/architecture.md`, Storage layout | A teammate's save must reach this Mac's files |
| Brainiac writes into a note only on a save or an explicit action | `SPEC.md`, section 5 | A remote save is neither, unless the rule says so |
| Every write goes through a domain service, which emits the committed change event | `docs/architecture.md`, Decisions | Still holds, and becomes the seam that sync hooks into (below) |
| Deletes are hard deletes | Every table | A deletion must reach other replicas: tombstones |
| One account per forge provider, global | `forge_accounts.kind UNIQUE` | Each member uses their own account, and one person may need a personal and a work account |
| Connections, saved queries, and agent profiles are global | `db_connections`, `saved_queries`, `agent_profiles` | Each needs a scope |
| Runs live in this Mac's data folder | Run journals, collected bare repositories | Another member cannot see them |

The domain-service rule and UUID IDs are the two choices already made that sync relies on. Most of the rest has to change.

## The core changes

### 1. A scope on everything shareable

Every shareable entity belongs to exactly one **scope**: personal (this installation, as today) or one shared workspace. An entity never changes scope in place. **Move to workspace…** and **Copy to personal…** create a new entity and, for a move, delete the old one, so a scope change is never a partial sync.

### 2. Shared fields and personal fields are separate rows

Each shared entity splits into what the team agrees on and what is one person's:

| Entity | Shared | Personal (this Mac only) |
| --- | --- | --- |
| Workspace | Name, members, watched branch patterns, pull requests switch and provider/organization | Discovery folder, auto-fetch, notifications, digest |
| Repository | Name, normalized remote URL | Path of the clone, pin, recent list, last tab, cached status |
| Activity | Nothing (each Mac observes its own fetches) | Events, read state |
| Database connection | Kind, host, port, database, TLS, environment, access ceiling, repository links, credential reference | User override, password override, health settings |
| Saved query | SQL, folder, connection, name | Nothing |
| Query run history and tabs | Nothing | Everything (`history.db`) |
| Note | Text, frontmatter, links, note identity | Drafts, revisions not yet saved, pins |
| Task | Title, description, status, dates, links, assignee | Today's triage of it ("planned for me") |
| Agent profile | Image, limits, permissions, network rules, credential reference | API key or subscription token override |
| Agent run | Record, prompt, conversation, outcome, collected result | Draft follow-ups, review position |
| Remote host | Address and host key, if the team shares a runner host | SSH key and access |
| Secret | Where it is kept (a reference) | Overrides; the value, always |

Personal fields go in personal tables keyed by the shared entity's ID, never as columns in the shared row. This keeps the shared row replicable as a whole, and keeps a field from leaking to the server because someone added it to the wrong table.

### 3. A replica per shared workspace

Recommended layout: personal data stays where it is, and each linked shared workspace gets its own database file in the data folder (`spaces/<id>.sqlite3`), a replica of the server's copy plus an outbox of local changes not yet acknowledged.

- Leaving a workspace deletes one file and one vault folder. Nothing has to be found and filtered out of a mixed database.
- A file holds one scope's data, so what is uploaded is a property of the file, not of a `WHERE` clause that could be forgotten.
- Backups and restore treat each file separately: a shared workspace is restored from the server, not from a snapshot.
- Links between scopes (a personal task linked to a shared note) are IDs without foreign keys, as note-to-repository links already are, and show the target as unavailable when its scope is gone.

The alternative, a `scope_id` column on every table in `brainiac.db`, keeps queries across scopes simple, but every query, export, and deletion has to filter correctly forever. Separate files are the stronger boundary.

### 4. Sync through the domain services

- A write to a shared entity goes through its domain service as today. In the same transaction it appends an operation to the replica's outbox: entity, ID, expected version, the new shared fields.
- A sync worker sends the outbox to the server, which accepts each operation whose expected version matches. It returns the authoritative version, or a conflict.
- Changes from the server are applied through the same domain services, which emit the same committed change events. The UI, search index, and MCP server see a teammate's change exactly as they see a local one.
- Conflicts are surfaced, never merged silently: the local change is kept as a draft beside the new version (for a note, as a revision; for a task or a connection, as an edit the user re-applies or discards).
- Deletions are tombstones with a version, kept until every replica has seen them (or for a fixed period, after which a replica that was away that long resyncs from scratch).

### 5. Notes stay Markdown

A shared workspace has its own vault folder (inside the data folder, or a folder the user chooses), and its notes are ordinary Markdown files there, as today.

- A teammate's save arrives as a file write by Brainiac. SPEC's rule that Brainiac writes a note only on a save becomes "only on a save, by this user or a member of the note's shared workspace", and the editor treats it like a change from outside (`SPEC.md`, Saving and changes from outside).
- Edits made outside Brainiac to a shared vault folder (another editor, `git pull`) are picked up by the watcher and synced like a save.
- Two people editing the same note: the second save conflicts and keeps both texts, the newer as the note and the other as a revision with a visible conflict. Live co-editing (CRDTs) is a later choice, only if conflicts turn out to be common.
- The shared vault folder is never a Git repository that Brainiac commits to. Teams that want notes in Git keep using a personal vault inside a repository.

### 6. Tasks

Tasks already carry a version on every write, which is what sync needs. Shared tasks gain an **assignee** (a member). Today shows the tasks assigned to me or created by me, plus my personal tasks. Triage ("planned for today") is personal: it goes in a personal table keyed by the task.

### 7. Agent runs

Runs are the heaviest to share: a live conversation, a container, a time limit, and a collected Git result.

- **The record and the result are shared, the execution is not.** A run executes on one host, owned by the run controller of whoever started it, exactly as in v0.5. Members see its record, conversation (synced as it is journaled), and outcome. Only the person who started it, or an Admin, can send follow-ups, approve permissions, cancel, or finish it.
- **The collected result** is a patch and a bare repository in Brainiac's data folder today. Shared, it travels as the patch (small) and, when the team wants a branch, through Push branch and Create pull request (v0.5's later phases), so Git hosting stays Git's job and the server never stores repositories.
- **A shared remote host** fits naturally: v0.5's approved Linux hosts can be a team's runner. Approving a host is still each member's decision, since it receives their credential.
- **Credentials stay personal.** A run uses the API key or subscription token of the person who starts it. A Claude subscription token belongs to one person and must never become a shared reference. A team API key can be a shared reference like any other secret.

### 8. Permissions

Coarse roles, per shared workspace:

- **Viewer:** sees everything shared and can run saved queries through their own credential, within each connection's access ceiling.
- **Member:** also creates and edits notes, tasks, saved queries, and runs.
- **Admin:** also changes connections, agent profiles, credential references, hosts, and membership.

Per-item permissions are out of this design. The database's own grants still decide what a member's credential can do: a shared connection's "Read and write" is a ceiling, not a grant.

### 9. Agent access (MCP)

An agent working through Brainiac's MCP server sees shared notes and tasks the way its user does, and its writes are that user's writes, synced to the team. That turns a local tool into one that writes to other people's data. So agent writes to a shared scope need to be marked as made by an agent in the operation log, and possibly limited to drafts a person confirms. This is an open question. The rule that agents can neither read a secret nor change where one comes from is unchanged.

## Credentials

### Share where a secret is kept, never the secret

Brainiac already saves a secret's *source* and never its value (`SPEC.md`, section 12). A shared workspace keeps that rule. What it shares about a credential is a reference, such as a 1Password item `op://Team/staging-db/password` read with `op read`, or a Google Secret Manager resource name (secrets phase 2). Each member's Brainiac resolves the reference on their own Mac, with their own access to the team's password manager.

- The server never holds a team secret. It is not a secrets service, and a breach of it leaks references, not passwords.
- Granting and revoking access to a secret happens where the secret lives: removing someone from the team's 1Password vault is what stops them reading the password. Brainiac's membership controls who sees the setup, not who can read the secret.
- Rotating a secret in the password manager reaches everyone at their next read, with no copy to update.

### Needs and bindings

Split what a shared item *needs* from what *provides* it:

- A **need** belongs to the shared item: "the password of connection *staging*", "a GitHub token for organization *acme*", "an API key for agent profile *review*". It carries where the secret is sent (the destination), which is shared too.
- A **binding** says how one person satisfies it, chosen in this order:
  1. this person's override on this Mac (any source, including the Keychain and Ask);
  2. the shared workspace's reference, if it has one and this person has allowed it (below);
  3. nothing: the item shows that it needs a credential, with **Use my own…**.

`CredentialService::resolve` takes a `Binding` and does not care where it came from. The reading, caching, shared reads, and lease rejection stay as they are. What is new is the step that picks the binding, and it belongs next to the credential rows (Preparation), not in each domain service.

Forge tokens are personal by nature: a token acts as its person. A shared workspace says which provider and organization its pull requests use, and each member brings their own account. That needs more than one account per provider (a personal GitHub account and a work one).

### Which sources can be shared

| Source | As a shared reference | Notes |
| --- | --- | --- |
| Keychain | No | An item on one Mac; always personal |
| Ask each run | No | Typed by each person |
| Environment variable | The name only | Each Mac sets its own value; rarely useful in an app opened from the Dock |
| Command | Program name and arguments | The absolute path is found on each Mac; `/opt/homebrew/bin/op` means nothing on another one |
| Google Secret Manager (phase 2) | Project and secret name | Read with each person's gcloud credentials |
| Git's credential helper (phase 2) | No | Personal by nature |

### Trust: a teammate cannot run a program on my Mac

A shared Command reference is a program and arguments chosen by someone else, run on this Mac with this person's permissions. That is the largest risk in the design. Today's restore approval is the model for handling it:

- A reference is not run until this person allows it.
- **Allow…** shows the exact program and arguments and where the secret will be sent.
- Any change to either (a new revision) needs allowing again.

A shared workspace can never change a binding silently. Still to decide: whether shared commands are limited to a short list of programs (`op`, `bw`, `gcloud`, `vault`), and whether an Admin's change is held until a second Admin accepts it.

The destination is part of what is allowed, as today. Suppose a teammate points the staging connection at another host. Every member's password, including a personal override, then waits for approval again, so a changed host cannot collect passwords.

## Design it twice

### The sync substrate

**A. A server that keeps the authoritative copy and an operation log (recommended).** Clients keep a replica and an outbox, and the server orders operations and checks versions. This matches how Brainiac already writes (domain services, expected versions, change events), it handles permissions and membership in one place, and conflicts are rare for structured data. It costs a service to build, host, and keep secure, and accounts with it.

**B. CRDTs for everything.** Every entity is a CRDT document (Automerge or Yjs), synced peer to peer or through a relay. It merges without conflicts and works offline indefinitely. But merging is not always right: two people changing a connection's host should conflict, not merge. CRDTs also replace SQLite as the source of truth for structured data, and permissions still need a server. Worth it for live note co-editing only, later, inside A.

**C. Git as the substrate.** The shared setup is files in a Git repository the team already has: notes as Markdown, tasks, connections, saved queries, and profiles as one file each (TOML or Markdown with frontmatter). Brainiac reads them, and the team commits and pulls. There is no server, it is reviewable in pull requests and versioned, and offline works by default. It also has no invitations, membership, or live updates, and a pull is the only way changes arrive. Writing those files would mean Brainiac commits to a repository, which the hard rules forbid. Reading them is allowed, so C is a read-mostly mode: Brainiac shows a repository's `.brainiac/` folder, and people edit it with their editor and Git.

A is the design. C is a possible first step that needs no service and could stay as an export format. B is only for note co-editing, if it is ever needed.

### Credentials

**References only (recommended)**, as above. The alternative is a **cloud vault**: secrets encrypted per member with their public keys, with sharing, rotation, and audit. That makes Brainiac a secrets manager, with key management, device enrollment, lost-laptop recovery, and re-encryption when someone leaves. A breach would leak the ciphertext of every team's production passwords, and revoking a member does not revoke what they already decrypted. Consider it only if teams without a password manager ask for it.

## Rules this would change

- *One person, one Mac, no required account* (`SPEC.md`, section 1): a shared workspace needs an account with the service. Everything personal must keep working without one, and without a network.
- *Brainiac writes into a note only on a save or an explicit action* (`SPEC.md`, section 5): a teammate's save becomes one of those.
- *The app never writes to a repository*: unchanged. Sharing never clones, pulls, or commits; a missing repository is located or cloned by the user.
- *A secret never reaches Brainiac's databases, an export, or the network except its destination*: unchanged, because only references are shared.
- *Backup = SQLite snapshots*: personal data only. A shared workspace's copy comes back from the server.
- *Agents can neither read a secret nor change where one comes from*: unchanged. Agent writes to shared data are a new question (Agent access).

## Preparation (now, no behavior change)

Worth doing anyway, and cheaper now than later:

- **Every write through a domain service, always.** This is already a decision. A write that skips it (a migration fixing data, a background job writing rows directly) would be a write sync never sees. Keep it absolute.
- **UUIDs for every shareable entity.** This is already true. Local sequence numbers such as `tasks.seq` stay local and never become references.
- **Keep personal and portable fields apart in new tables.** Paths, read state, last-opened times, and machine-specific details (a command's resolved program path, a clone's folder) go in fields or tables of their own, not mixed into what a team would share.
- **Links between entities by ID, tolerant of a missing target**, as note-to-repository links already are, rather than new foreign keys between things that could end up in different scopes.
- **Credentials as rows of their own.** Today each domain repeats the same columns and save sequence (`forge/accounts.rs`, `databases/connections.rs`, `agents/settings.rs`). Move them into one table owned by `credentials/`, with domains holding a reference. Do it when the next owner kind arrives (registry tokens in v0.5), not for this design. It is also where a shared reference and a personal override would attach.
- **Owner keys are IDs.** New credential owners use `<kind>:<id>`, never a provider name.
- **Repositories keep a normalized remote URL**, as they already do for restore and pull requests.

Not yet: scopes, replicas, an outbox, tombstones, accounts with a service, roles, or more than one account per provider.

## Phases, if it happens

1. **Read from a repository (C).** Connections, saved queries, and agent profiles defined in a repository's `.brainiac/` folder appear in Brainiac, with credential references allowed per person. There is no service yet, and it tests the shared and personal split and the trust rules.
2. **Shared workspace with configuration.** Accounts, the service, membership, and roles. Shared repositories, connections, saved queries, agent profiles, and credential references.
3. **Tasks.** Shared tasks with assignees, in Tasks and Today.
4. **Notes.** Shared vault folders, with conflicts kept as revisions.
5. **Runs.** Shared run records, conversations, and patches; a shared remote host.

Each phase is useful alone, and the hardest data (notes, runs) comes last, after sync has carried simpler records.

## Not in this design

- Live co-editing and presence.
- Per-item permissions.
- A cloud vault for secrets.
- Running agents on the service's own machines.
- Self-hosting the service, billing, and hosting.
- Sharing between workspaces, or public links.

## Open questions

- The name: *shared workspace*, or a new noun such as *team* or *space* that contains workspaces. A local workspace is a list of folders on this Mac; one word for both makes every sentence ambiguous.
- Whether phase 1 (read from a repository) is enough for most teams, and phase 2 can wait.
- How a repository is matched across Macs: by normalized remote URL, by its first commit, or both, and what happens with forks and several remotes.
- Whether agent writes through MCP to a shared scope are allowed, marked, or held as drafts.
- Whether shared Command references are limited to known programs, and whether an Admin's change needs a second Admin.
- Whether a Viewer's query run history is ever visible to others (today it is personal, in `history.db`).
- How long tombstones are kept, and what a replica that was offline longer does.
- Whether a shared vault folder can live in a folder the user chooses (and so inside a synced folder such as iCloud Drive, which would be a second sync competing with the first).
