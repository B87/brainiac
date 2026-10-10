# Design: explaining changes

Design notes for v0.6: Brainiac explains a change to the person reading it, so they learn the software being built rather than only seeing what moved. An explanation is written by an agent in a run of its own kind, on v0.5's agent-run machinery, and shown beside the diff in v0.1's viewer.

v0.6 started on 8 October 2026. What it does is now `SPEC.md` section 14, and how it is built `docs/architecture.md`, Explaining changes — v0.6; where they differ from the notes below, they win. This file keeps the reasons, the spike, and the questions still open.

UX Design Artifact: https://claude.ai/artifact/R41zyUax5ikqMYXh7Y9TuA (the panel, tour, notes, concepts, staleness, and Settings apply as drawn; its Explain dialog and Working state predate the choice of agent runs)

## Why

Agents now write a growing share of the code (v0.5), and a diff shows what changed, not why, how it fits the rest, or which ideas it relies on. Brainiac already holds what a good explanation needs: the change, the repository it belongs to, the commit message and pull request, and the repository's own spec and decisions. Explaining with those, and remembering what the reader already knows, teaches; a generic summary does not.

## What the user gets

- **Explain** (button and `E`) on a commit, a branch compared with the default branch, a collected run's result, and a pull request (moved into v0.6 on 9 October 2026, sharing one explanation with a branch of the same changes; `SPEC.md`, section 14, Pull requests); working-tree changes in 0.6.x. Never automatic. A short dialog names the agent and host that will read the code, the depth (Brief, Teach me, Deep), and whether to add questions.
- **A panel beside the patch**, not a separate view: a summary; a source line (what the agent read); and three tabs:
  - **Tour:** the files in reading order (the rule, then the fix, then its helpers, then bookkeeping), each with its role. The file list switches between Reading order and Path, and `[` / `]` follow the tour.
  - **Concepts:** ideas the change relies on (a language feature, a library, a protocol or standard, a tool or service, a technique, a project pattern), each with where it appears. **Got it** adds it to the ledger, and later explanations skip it unless a change uses it in a new way.
  - **Check yourself:** two or three questions with Reveal answer.
- **Notes in the patch:** a note sits after the lines it explains, with links to its sources. Concept words in the code are underlined and open the concept.
- **Every claim cites its source.** A note links to the file and lines, or the doc section, it rests on. A disagreement between code and docs is shown only when both quotes are found in the files.
- **While it works:** the files the agent opens, from the run's journal, and Cancel. An explanation takes minutes, not seconds; a finished one is kept and opens at once.
- **Staleness:** each note keeps the hash of the lines it explains. When those lines change, only that note is marked out of date, with Re-explain.
- **Save as note:** the explanation becomes a Markdown note in the vault, with the repository, commit, date, and agent in its frontmatter.
- **Settings → Explanations:** the profile and host (by default those of the last run), the reader's level per language, the default depth, the concepts known, the repositories asked, and stored explanations with Delete all.

## How it works

1. **An explain run.** The same controller, container, input bundle, profile, key, and host as any run, with a kind of its own: Brainiac writes the prompt, the agent is autonomous inside the container, the time limit is short, and the run has no review, no patch, and no place in the Runs list. The agent reads whatever it needs (callers, definitions, `SPEC.md`, decisions, history) instead of a context Brainiac gathers.
2. **The prompt** holds the subject (a commit, or a range for a branch or a run's result), the reader's level and known concepts, the depth, the schema, and the rules: lines on the new side, quotes copied verbatim, no edits to the code, and the result in `.brainiac/explanation.json`.
3. **One file comes back.** The collector reads only that file; anything else the agent changed is ignored, and the run is cleaned up as soon as the file is imported.
4. **Checked before it is shown.** The file must match the schema; each note must anchor to lines in the change; each quote must be found verbatim in the commit's files, or its claim is dropped; a code–docs disagreement needs two verified quotes. A file that fails gets one follow-up prompt in the same session with the errors, then the explanation fails with what was wrong.
5. **Stored by Brainiac, never in the repository.** Explanations are kept per repository, subject, agent, and depth, and can be deleted. The level, the concepts known, and each repository's answer are user settings in `brainiac.db`.

**Privacy and cost.** The code goes to the provider of the chosen profile, as for any run; the first Explain in a repository asks once, and the answer is listed in Settings. A Claude Code profile can use the Claude subscription (subject to Anthropic's answer on the token's terms); on an API key, an agent reading many files costs more than one model call, and the run's reported usage is shown.

## Phases

- **0.6.0:** commits, branch comparisons, and collected run results; the explain run, the checks, the panel, notes, concepts with the ledger, questions, staleness, code–docs disagreement, Save as note, and Settings → Explanations.
- **0.6.x:** planned in the roadmap and `SPEC.md` section 14 as working-tree changes (sent as a patch the container applies to its copy, since Brainiac never writes the user's repository), a question about one note as a follow-up prompt while the session lasts, and a direct model call for small changes, which would also allow a local model. On 9 October 2026 the maintainer doubted that any of the three is worth the effort, and a panel round agreed with most of that (Follow-ups considered, below). Nothing in 0.6.x is committed to; the roadmap and `SPEC.md` still list the three until that is decided. The concept labels (Concept classification, below) are the one refinement the panel asked for.

## Spike before building

No code: ordinary runs on this repository with a prompt that asks for `explanation.json`, over about five commits of different sizes, with Claude Code and with OpenCode. Score every note by hand (correct, useful, grounded) and record the time and cost. It decides the prompt, the schema, the default time limit, and whether the result teaches.

### Spike, first round (8 October 2026)

Claude Code only, run as the headless `claude -p` on a fresh clone of this repository checked out at each commit, not yet as a Brainiac run: no container, so the times below leave out its start and the image. The agent could read files, search, and run `git`, and write only the explanation. Five commits of different sizes, each with Opus 5.5 (Claude Code's default) and with Sonnet 5.5. Costs are what Claude Code reported for the run; on a Claude subscription they are notional.

The prompt, with the commit in place of `<SHA>`:

```
Explain commit <SHA> of this repository (checked out at that commit) to a reader who is new to Rust and comfortable with TypeScript. Do not edit any file except the one named below.

Read whatever you need: the diff (git show <SHA>), callers and definitions, SPEC.md, docs/architecture.md (especially its Decisions), and git history.

Write .brainiac/explanation.json, a single JSON object with:
- "summary": 2–4 sentences on why the change exists, not only what moved.
- "sources_read": the files and doc sections you relied on.
- "tour": the changed files in reading order (the rule, then the fix, then its helpers, then bookkeeping), each {"path", "role"}.
- "notes": each {"path", "new_start", "new_end", "text", "sources": [{"path", "start", "end", "quote"}]}. new_start/new_end are line numbers on the NEW side of the diff, inside a changed hunk. Each quote is copied verbatim from that file at this commit, and lies within start..end.
- "concepts": ideas the change relies on, each {"name", "kind": "language"|"library"|"protocol"|"tool"|"technique"|"project-pattern", "explanation", "appears": [{"path", "line"}]}.
- "questions": 2–3 {"question", "answer"} that check understanding.
- "disagreements": only where code and docs conflict, each {"claim", "code": {"path","start","end","quote"}, "doc": {"path","start","end","quote"}}. Empty array if none.

Every claim must cite a source. If you cannot quote it verbatim, leave the claim out.
```

Each file was then checked the way Brainiac would check it: every key present; each note's lines inside a changed hunk on the new side; each quote found verbatim within its cited lines at that commit (or, failing that, elsewhere in the file); each concept's location an existing line; every tour file part of the change.

| Commit | Change | Opus: time, cost | Sonnet: time, cost | Notes O / S | Quotes O / S | Concepts O / S | Disagreements O / S |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `5a98929` | 3 files, +12/−3 | 79 s, $0.49 | 42 s, $0.18 | 5 / 5 | 13 / 6 | 7 / 4 | 0 / 0 |
| `29bbc56` | 4 files, +35/−11 | 118 s, $0.68 | 61 s, $0.22 | 6 / 6 | 15 / 9 | 7 / 6 | 1 / 0 |
| `4aae4b2` | 6 files, +87/−24 | 142 s, $0.77 | 54 s, $0.22 | 11 / 8 | 30 / 9 | 10 / 6 | 0 / 0 |
| `287bdc9` | 5 files, +196/−17 | 250 s, $1.53 | 90 s, $0.37 | 14 / 12 | 38 / 18 | 13 / 8 | 1 / 1 |
| `7edf3c2` | 14 files, +726/−239 | 231 s, $1.50 | 110 s, $0.52 | 23 / 16 | 46 / 26 | 11 / 7 | 0 / 0 |
| **Total** | | **$4.97** | **$1.51** | 59 / 47 | 142 / 68 | 48 / 31 | 2 / 1 |

What it showed:

- **Both models follow the schema.** Every file parsed with every key, and every tour covered every changed file in a sensible order. The runs left the clone's tracked files unchanged.
- **Opus passed every check.** All 59 notes were inside changed lines and all 142 quotes were verbatim at their cited lines, so a follow-up turn would never have been needed. Sonnet failed two checks in five runs: one note anchored to unchanged lines (`5a98929`), and one quote verbatim but cited at the wrong lines (`4aae4b2`).
- **Disagreements are worth having.** Both of Opus's were real, and both docs were still out of date at the time of the spike: `architecture.md` said a pasted token has only its line breaks removed (`29bbc56`), and that a protocol mismatch is always a failure (`287bdc9`). Sonnet found the second and missed the first. One Opus note on `287bdc9` also found a small bug: Install over an earlier controller restarts the service but reports "the service started".
- **Sonnet is thinner, not wrong.** Its summaries, reading order, and questions were accurate and taught. It cited about 1.4 quotes per note against Opus's 2.4, gave fewer concepts, and caught less.
- **Cost follows what the agent reads** (files opened, turns) more than the size of the diff.
- **The checks cannot judge correctness or usefulness.** They prove that a note sits on the change and that its quotes exist, not that its claim is right. That still needs the hand scoring.

What it suggested for the build, taken into `SPEC.md` section 14 as defaults (the time limits doubled to allow for the container until runs in one are timed):

- The model follows the depth: Sonnet for Brief and Teach me, Opus for Deep, with an override in Settings → Explanations.
- The checker repairs what it can before a follow-up turn: a quote found verbatim elsewhere in its file is re-anchored there rather than reported. A note outside the changed lines still costs the follow-up turn, or is dropped.
- A default time limit of about 5 minutes for Sonnet and 10 for Opus, until runs in a container are timed.

Left for the spike: scoring every note by hand; the same five commits with OpenCode, with at least one provider; and the prompt as a real Brainiac run, to time the container.

## Follow-ups considered (9 October 2026)

A client panel (three fictional personas, so hypotheses to check with real users) reviewed the three 0.6.x items. None is built.

- **Working-tree changes.** The one the panel wanted most (the learner daily, the CTO several times a week), because it covers the moment before a commit. It reuses the explain run, the checks, and the once-per-repository answer. Three things the design must answer first: what the patch holds (untracked and ignored files such as `.env`; the dialog should list the files and byte count before anything leaves the Mac), pinning the explanation to a snapshot (a working tree changes during a 5 to 10 minute run, and "commits and run results do not change" no longer holds), and what happens to the explanation when the changes become a commit tomorrow. The checker also needs a reference for "verbatim at the subject's commit" when there is no commit. Wait for the exit-gate week to show whether explaining uncommitted work is wanted.
- **A question about one note.** The persona who learns by asking wanted it most, and it is the least workable as specified: the container is removed as soon as the explanation is checked, so "while the session lasts" is a few seconds during the run, when nobody is reading yet. It would need a new, costed run seeded from the kept conversation, and an answer needs the same quote checks as a note, a place to be stored, and a rule for Delete explanation. Set aside unless that is designed.
- **A direct model call, and local models.** Wanted by the CTO and the freelancer for cost, speed, and keeping code on the Mac. Doubts: a call with no file access cites fewer sources (Sonnet already cited 1.4 quotes per note against Opus's 2.4), so notes are thinner and the code–docs disagreement check rarely fires; "small" is undefined; a local model has no provider, so the repository's answer needs a separate form for it, never a way around a No; the estimate must be kept per path; and the spike never tried a local model, so whether one writes a valid schema is unknown. Do not build before the spike has data.
- **An agent in the working tree, without a container.** Considered as a way to speed explanations up and set aside. The spike's runs took 42 to 250 seconds with no container, so the model's reading and writing is most of the time; the container's start is not yet measured, and a measurement comes first. The working tree would give up the pinned snapshot the checks rest on and the rule that a checkout is never mounted, and the agent could run commands against the user's real files. Two options that keep that safety if the start turns out to matter: a warm container per repository, and an agent over Brainiac's own bare copy on the host with no container (which needs its own disclosure). The cheapest speed-up is already a default: Sonnet takes about half the time and a third of the cost of Opus.

## Judgments around Explain (considered, not planned)

Small, fast judgments from a model that returns typed answers (one of N options, a yes with a probability, a score) instead of text, with Brainiac's code deciding what to show. It cannot replace the agent that writes the explanation; it could check and order around it. Six uses were put to the panel:

1. A note's quoted lines support its claim (supports, contradicts, says nothing).
2. Triage a change before a run: behavior change or mechanical, risk, context needed.
3. A new commit's hunks "likely address" an open review thread.
4. A change disagrees with the project's docs; a note is out of date only when its claim no longer holds.
5. Grade a free-text answer to a Check yourself question; leave out concepts known under another name.
6. Check a pull request against the team's written conventions, one yes or no per rule per hunk.

What the panel said: only the first is wanted by all three, because it targets the gap the checks admit (a quote can exist and still not support its note). The narrow half of the fourth came second: today every touched line marks a note out of date, and Re-explain costs money. Held back: the third sends teammates' comment text, which contradicts "only the commits are sent" (section 14, Pull requests); the sixth has no shared source for rules (one user per Mac) and would put a pill on every hunk; the second's "skip Explain?" hint is distrusted, since a wrong "mechanical" costs more than the run it saves, and it would send code before the user chose to; grading a person is the error that blames the reader.

If any of it is built, the panel's conditions are:

- Never use the word the checks use ("verified") for a model's verdict. Show only exceptions ("Not confirmed", with a one-line reason such as "the quoted lines do not mention retries"), never a mark on every note, and count them in the "Matched to the change" line.
- Run inside an Explain the user started, never on opening a subject, and obey a repository's No. The judge's provider is a second destination for code: it needs its own once-per-repository answer, and the Explain dialog's estimate must include its cost. If it is unavailable, Explain behaves as today.
- A judgment that keeps a note ("still holds") must say why, and falls back to the lines' hash when the judge is unavailable.
- Measure before designing: run the judge over the spike's notes and quotes, including the two Sonnet failures, and see whether it flags the bad ones without flagging good ones. A judge from the same model family as the writer may share its mistakes (the spike's failures were wrong anchors, not wrong claims). Confidence is not correctness, and the thresholds need tuning on code.

## Concept classification (9 October 2026)

Seen in use: `go:embed` is listed as a Language, and DKIM, DMARC, and SPF as "System tool", though they are DNS record standards, not tools; a filter by kind helps little once the ledger holds hundreds of concepts. The grain (one idea, a sentence or two) is right; the labels and the grouping are not. A panel round on four proposals:

- **Rename the kinds (wanted; built in 0.6.1).** Split "system" into **protocol or standard** and **tool or service**. The learner is misled by "tool" (it reads as something to install); the others take it as cheap. It changes the prompt schema, the checker's list of kinds, and a concept's identity (kind plus name), so existing "system" rows need a defined migration (which kind they become, and whether Merge groups survive). Built: the kinds are language, library, protocol, tool, technique, and project pattern (a technique is a way of solving a problem that means the same in any codebase, such as idempotency; the table or helper one repository uses for it is a project pattern, and the prompt says to choose project pattern when unsure, because a technique is global and a mislabelled project pattern would carry one repository's words into others); every old `system` concept became a `tool` (a name cannot tell them apart), which keeps each merge group, and the reader corrects the wrong ones once concepts can be edited (`architecture.md`, Explain runs).
- **Group by where the concept lives, derived, not tagged.** An optional free-text `area` (go, typescript, postgres) was the first idea. The panel judged it will drift (Go, golang, go language), leaves a merge chore on every Mac, and, if its values are listed in the prompt, adds cost per run and lets one client's domain words reach a prompt about another client's code. Instead, derive a group at display time from where a concept appears (file extensions and paths) and from its repository, and group the Concepts You Know page by it, with a count per group. If a model-assigned area is wanted anyway: display only, never sent in the prompt, scoped per repository like project patterns, with a small fixed list of aliases.
- **A level per area (do not build).** Hidden rule that changes what is skipped, a setting to fill in before you know which areas you need, and no gain over the per-language level for languages.
- **Matching of known concepts stays exact.** A wrong skip is worse than a repeat for all three personas, and it is invisible. Keep exact-name matching plus the existing "unless the change uses it in a new way", state the rule in the prompt, and never skip "related" concepts. Show how many known concepts an explanation left out ("Left out because you know them: 3", with Undo), next to "Matched to the change", so a wrong skip can be noticed.

At 500 concepts the kind chips stop helping: the page needs grouping and search, and a forget by repository (one client's finished patterns). The bigger cost is in the prompt: whether all known concepts are sent on every run (cost per run, and per seat) needs an answer before the ledger grows.

Not yet in `SPEC.md` or `docs/architecture.md`; the kinds and the grouping change behavior and the schema, so `SPEC.md` section 14 changes first.

### Editing the ledger, and scope (9 October 2026)

A second panel round on two proposals: a richer Concepts You Know page with light editing (change a concept's kind, rename it, add one by hand or paste a list, edit its description, group, sort, search, "left out N times", bulk Forget by repository), and three scopes for a concept (global, workspace, repository) used both on the page and when an explanation is generated. Hypotheses, not user data.

**Scope: two levels, not three.** All three personas thought of what they know as "things everyone knows" against "things this codebase does"; none thought in workspaces. A workspace is a grouping made for navigation, not a statement about confidentiality, and Brainiac cannot tell whether a user's workspaces follow their clients. A standard that reached every repository of a "web apps" workspace would carry one client's words into another client's prompt, which is a breach to explain, while a concept taught again costs a few cents. Wrong scope has to fail toward the narrower one. A repository in several workspaces would need a rule (the code-sharing rule, where any No wins, is about permission, and scope is about telling, so the same word would mean two things). So:

- Keep what `SPEC.md` has: global kinds (language, library, protocol or standard, tool or service) and the project pattern, which belongs to one repository. The default scope follows from the kind, by rule; never chosen by the model, and never by the user at the moment a concept is saved.
- For a company's standards shared by several repositories: an explicit, per-concept choice, never a scope that reaches by inference. It was first written as **Copy to another repository**; the maintainer preferred "applies to" and a later panel round agreed (Shared project patterns, below). Bulk Forget by repository covers the end of a client.
- Changing a concept to or from project pattern is a change of scope, so it is never an ordinary kind change: it names the repositories whose explanations will then be told about it, or stop being told. A kind menu that silently turns one client's pattern into a global concept is a leak.

**The page: what to build, in order.**
1. **Left out because you know them: N**, in the explanation next to "Matched to the change", with Undo beside the concept, not only in Settings. Everything below depends on a wrong skip being noticed, so it comes first. Entries added by hand are labelled "added by you" there.
2. **Group by repository, sort, and search over names and descriptions; bulk Forget by repository.** The cheapest part and the one the freelancer and the CTO most wanted. Descriptions are text derived from a client's code, so they stay in `brainiac.db` and are not indexed elsewhere. This replaces the grouping by where a concept appears (file paths) suggested above: two personas judged it expensive and prone to drift, and a repository grouping is what matches how people move between clients. Built in 0.6.1: sort (newest, name, kind), Group by Repository (a project pattern under its repository, any other concept under the repository it was learned in), search that also covers repository names, and a Forget for each group that says which of its concepts are known everywhere and will be taught again everywhere.
3. **The kind rename** (protocol or standard, tool or service), with the migration.
4. **Change kind and rename as a Merge**: the old kind and name stay as an alias that points at the edited concept, so a later explanation naming it the old way maps onto it instead of making a duplicate. Aliases show as rows inside the concept, as Merge's already do, never as an invisible pointer. Changing a kind stays within the same scope (a global kind to a global kind). What Forget does to a concept's aliases is open. Built in 0.6.1 as **Edit…** on a concept: the old identity is kept as a row merged into it, shown as "Also called"; a project pattern may become any kind after asking (it belongs to no repository afterwards), and a concept known in every repository cannot become a project pattern.
5. **Add one concept by hand**, marked "added by you" (not built in 0.6.1: the maintainer moved it after the exit-gate week). Disagreement: the learner wants it on day one (re-taught SQL joins feels like a tool that does not know them), the CTO distrusts a hand-added entry that is skipped silently, and the freelancer would do it once.

**Not built.** Workspace scope; a scope chosen by the model; description editing (it is display only, and invites the belief that it changes what is skipped); a pasted list (fifty entries added in a burst grow the prompt and are forgotten); rich text; editing where a concept was learned.

**An alternative to hand-adding.** A new reader's level is already set per language in Settings. Extend it with a short checklist of libraries and standards chosen from the languages seen in the first repository: bounded, and it cannot hold a client's names.

**The prompt is the real constraint.** Hand-adding and pasting make the ledger grow faster, and at 500 concepts the cost is in sending it on every run. If the list is capped or filtered, send only the concepts whose language or path matches the changed files, plus that repository's patterns, and show the count in the dialog's estimate. A hand-added concept is matched by exact name like any other, and "unless the change uses it in a new way" applies to it too; whether the user can see that the model decided so is open.

**How the ledger reaches the run: a file, not a tool and not the prompt.** An MCP server was considered and set aside: Brainiac's is a Unix socket on the Mac, so a container (and any remote host) cannot reach it without mounting the socket or tunnelling, which breaks the rule that a run's container gets nothing of the Mac; repository text is untrusted and could steer a tool call; scoping a tool to one repository would rest on a per-run token; and a pull model cannot say afterward why a concept was skipped or taught again. Instead Brainiac writes the ledger into the run's input, outside the repository (`/scratch`, not `/workspace`), and the prompt says its contents are data, not instructions. The run's image today is `node:22-bookworm-slim` with `git`, `ca-certificates`, and `ripgrep` (`rg`) added, so `grep`, `rg`, and Node are there and `jq` is not. Look names up with `known`, below; do not assume `jq`.

- **A tab-separated file, one concept a line** (built in 0.6.1): `known-concepts.tsv` with the folded name, the name, the kind, and the words an earlier explanation of this repository used. Words learned in another repository are left blank, because they can quote that repository's code and code sharing is one answer per repository. It is at `/opt/brainiac/input/known-concepts.tsv`, outside the repository and `/workspace`, written once at most 5,000 lines and 2 MiB, newest first. A name that would blow the ceiling is left out rather than failing the run. An alias is a line of its own with its target's name, kind, and words, and two kinds of one name are both lines. A tab-separated file replaced the plain text file and the `grep -Fixf` recipe: the lookup needs the words beside the name.
- **A lookup script in the image**, `known` (`agents/image/known.mjs`, Node, already in the image). Names arrive one per line on standard input, in a quoted heredoc, so a name can contain spaces or punctuation (`Arc<Mutex<_>>`, `Limit rule`) without the shell splitting or redirecting them. It folds each name as Brainiac does, one code point at a time, ignores a trailing "(kind)", and prints `known: <name> (<kind>) - <words>` or `new: <name>` for each, so the agent can judge "unless the change uses it in a new way" from the words used before. A name of two kinds prints two lines. Exact names only. `known_used` is the name alone, the text between `known: ` and ` (`. The prompt gives the file's path and count and says it is data and not to be read whole.
- **Not built:** the filter by the languages of the changed files (the file is capped by count only, as the agent searches it instead of reading it), the split by kind, and the reader's level in the file's header.
- **The checker confirms the count.** The agent lists the known concepts it left out (`known_used`) and Brainiac counts only the names the ledger holds by exact name, so "left out N" can never claim more than the ledger says (built in 0.6.1; `architecture.md`, Checks). It does not drop a known concept the agent kept in Concepts, because that is how "unless the change uses it in a new way" survives, so a concept the agent kept or left out without naming is not counted. Dropping exact matches as a backstop would need a way for the agent to mark a new use, and is not built.
- **Measure before relying on it.** The run's journal records the agent's commands: count how often each agent runs the lookup, how many known concepts it still writes, and what a ledger of a few hundred entries adds to time and cost, with Claude Code and with OpenCode, before the estimate or the cap is set.

### Shared project patterns (9 October 2026)

A third panel round (hypotheses, not user data) on one project pattern that **applies to several repositories**, in place of a copy per repository: the row says "Applies to: widgets, api"; Edit Concept gets a Repositories list with a searchable "Add repository…"; Group by Repository shows it under each; renaming or changing its kind acts on all of them at once. Costs a table of repositories per concept, a migration, and changes to the lookup and to conflicts.

- **Applies to, not copy.** All three thought of a convention as one thing. Copies drift: a rename or a Forget in one repository would not reach the others, which is what Merge and the aliases exist to prevent.
- **Later, not in 0.6.1.** None would use it often (the CTO about twice a month, the learner almost never with one repository, the freelancer once per client). The workaround is Got it again in the second repository, which teaches one paragraph again for a few cents. The exit-gate week counts how often a project pattern of the same name is learned in two or more repositories, and that number decides.
- **Name only to the other repositories.** A description is written from one repository's code. Sending it in a run about another repository would put that code's words under the other repository's code-sharing answer, and could make the agent judge "a new way" against the wrong implementation. The run's file already leaves descriptions learned elsewhere blank; a shared pattern would keep that. The learner would rather have the words for the "new way" judgment, which the spike (below) can show to matter or not.
- **If it is built, say what it does.** Undo in an explanation removes only that repository and says so ("Teach again in api"); Forget on a shared pattern lists the repositories it reaches; removing the last repository says it forgets the concept; bulk Forget by repository says how many shared patterns stay in which repositories; the repository picker shows each repository's workspace, so a misclick cannot put one client's name in another's runs.
- **Open:** when the target repository already has a pattern of that name (refused, merged, or kept as two); what the warning lists when a pattern shared by several repositories becomes known everywhere; what export, restore, and removing a tracked repository do with the extra repositories.

### Known-concepts spike (harness 9 October 2026)

`spikes/known-concepts/` runs the question under "Measure before relying on it" with headless agents on a clone, as the first round did (no container, so its start is still untimed). For each commit and agent it runs once with no ledger, then once with a ledger of a few hundred entries: filler names plus every second concept the first run wrote, so some of what the agent would explain is known. It records time, cost, how often the agent ran `known`, the known concepts it still wrote under Concepts, and the ones it named in `known_used`. 

**First results (9 October 2026).** Sonnet 5.5 in Claude Code and in OpenCode (an Anthropic key), commits `29bbc56`, `287bdc9`, and `7edf3c2`, one run per cell. Seconds and dollars are the totals of the three commits.

| Agent | Run | Time | Cost | Lookups (per commit) | Known concepts still written | Known concepts named in `known_used` |
| --- | --- | --- | --- | --- | --- | --- |
| Claude Code | no ledger | 301 s | $1.14 | 0 | n/a | n/a |
| Claude Code | 300-entry ledger | 289 s | $1.12 | 2, 7, 2 | 2 of 13 | 1 of 13 |
| OpenCode | no ledger | 285 s | $0.98 | 0 | n/a | n/a |
| OpenCode | 300-entry ledger | 278 s | $0.99 | 1, 2, 4 | 0 of 14 | 3 of 14 |

- **The ledger added no time or cost we could measure.** Each ledger run was within the spread of the same commit without one, in both directions (Claude Code's 7edf3c2 took 138 s against 128 s, OpenCode's 29bbc56 55 s against 62 s). The lookup is cheap; what a ledger of 5,000 costs is not measured.
- **Both agents run the lookup every time**, usually once with a batch of 20 to 40 names, and Claude Code up to seven times.
- **An exact name rarely matches.** Of the 27 known concepts (from a sibling run of the same commit), the agents left out four by name (three with OpenCode, one with Claude Code). For most of the rest they wrote the idea again under another name: "Closures" became "Closure", "Arc<T> shared ownership" became "Arc", "Role-on-a-machine pattern" became "Role on a machine". Claude Code also kept "Result and the ? operator" in Concepts twice under its exact known name; whether it was used in a new way was not checked. A ledger built from earlier explanations of other changes would have more variation than this sibling-run one.
- **The agent looks up before it has settled its names.** It sent short words (`closure`, `Result`, `iterator`) from its first read of the diff, then wrote its concepts under longer names it never looked up.
- **`known_used` can overclaim.** Ten of the fourteen names the agents listed were the ledger's filler (`React` for a Rust change, `tokio`, `async/await`, `SQLite`), which the agent had looked up; the change may not rely on them. The checker confirms a name is in the ledger, not that the change uses it, so "Left out because you know them: N" can count a name the explanation never needed. It stays a count of ledger names, never above the ledger.

What this suggested was tried the same day (`--variant late`: decide the concept names first, then look up exactly those names, with the same ledgers):

| Agent | Time | Cost | Lookups (per commit) | Known concepts named in `known_used` | Read the ledger whole |
| --- | --- | --- | --- | --- | --- |
| Claude Code | 384 s (was 289) | $1.03 (was $1.12) | 2, 2, 9 | 2 of 13 (was 1) | no |
| OpenCode | 355 s (was 278) | $1.17 (was $0.99) | 3, 3, 3 | 10 of 14 (was 3) | yes, in all three |

- **It helped OpenCode by defeating the file.** In every commit it ran `cut -f2,3 <file> | head -400` and read the whole ledger, in spite of "do not read it whole", then looked up names copied from it. That is why 10 of 14 known concepts were left out by name. With 300 entries that costs a few thousand tokens (the run cost 18% more and took 28% longer); with 5,000 it would not be cheap. The ledger's size, not the prompt, will decide whether agents read it whole.
- **It did not help Claude Code.** It looked up the names it was about to write, and the ledger held the same ideas under other names ("Result and ?" against "Result and the ? operator"), so 2 of 13 were left out by name. It no longer kept a known concept in Concepts (0, was 2). Its time and cost moved about as much as a repeat run would.
- **Reading the file is not the fix.** The lookup was meant to spare the agent the file; when the exact name misses, the agent's next step is to read it. So the answer is likely in what `known` returns: a near name as "similar to", or a few candidates by the words in a name, so one lookup shows the agent what the reader knows without reading the file. That reopens "related concept skipping", and is the next thing to try.
- One OpenCode run failed with "database is locked" when two OpenCode processes ran together; the harness should not run them in parallel.

The next try was a near-name lookup (`--variant similar`, the first prompt plus a sentence on `similar:` lines). When a name has no exact match, `known` still prints `new:`, then up to three `similar:` lines: known concepts that contain every word of the shorter name, after dropping small words and a plural "s", closest first. They are candidates for the agent to judge, never a match, so the checker still counts only exact names in `known_used`. The same ledgers and commits:

| Agent | Time | Cost | Lookups (per commit) | Known concepts named in `known_used` | Read the ledger whole |
| --- | --- | --- | --- | --- | --- |
| Claude Code | 250 s (first prompt 289) | $0.99 ($1.12) | 2, 4, 2 | 8 of 13 (2) | no |
| OpenCode | 338 s (278) | $1.07 ($0.99) | 2, 3, 2 | 11 of 14 (3) | no, one filtered search |

- **It worked without the agent reading the file.** Claude Code went from 2 of 13 to 8 and OpenCode from 3 of 14 to 11, with the same ledgers, at about the cost of no lookup at all (Claude Code's cost and time were lower than the first runs, OpenCode's higher by 8% and 22%, which is within what a repeat run varies). OpenCode ran one `cut | grep` over the file in one of three commits and read none whole.
- **What still missed:** a name with different words, not the same words in another form (an agent wrote "Slicing a string by byte index" for nothing known, which is right; "Versioned controller protocol" was found for "Controller protocol version" only through the longer name's words). Nothing was kept in Concepts under a known name, as before.
- **`known_used` still includes filler** (`tokio`, `async/await`, `SQLite`, `Docker`) in six of the seven runs that named any, so the count can still overstate, and the checker cannot tell which names the change relies on.
- **Built from it (9 October 2026):** `known` prints the `similar:` lines, the prompt describes them and tells the agent to name a similar concept that is the same idea in `known_used`, and `architecture.md` and the image tests cover them. Near names are word-subset matches only: the same idea in other words is still new. The exit gate counts the `known_used` names the change did not rely on.

## Speed (11 October 2026)

Explanations felt slow in use. Three explain runs that completed in the app (OpenCode on Opus at Teach me, two commits and a pull request, on this Mac) took 89, 95, and 112 seconds and cost $0.45 to $0.71. Their journals show where the time goes; the fastest, a commit of a few files:

| Phase | Time | What happened |
| --- | --- | --- |
| Copy the subject, start the container, open the session | about 3 s | Brainiac's own setup |
| Reading | about 28 s | about six `git diff`, `sed`, and `grep` calls, each a few seconds of model time |
| Writing `explanation.json` | about 37 s | one tool call while the model writes the whole file |
| Checking its own work | about 13 s | the agent searched its quotes again, re-read the file, and edited it three times |
| Closing message | about 8 s | a 2 KB summary in the conversation that nobody reads |

The pull request's run had the same shape: about 40 s reading and 51 s writing the file. So the container's start is not where the time goes (Open questions), and a warm container or a host-side agent would save about two seconds. The model's own work is the time: the long write, and the tail after it.

Options, cheapest first. None is built.

1. **Cut the tail** (a prompt change; about 15 to 20 s, a fifth of a run). Tell the agent that Brainiac checks every note and quote and has one follow-up turn for what fails, so it should not check its quotes again, and should end its turn with one word rather than a summary. The risk is more follow-up turns or dropped notes when a quote is off; it is measured on the same commits before it ships.
2. **A model for each depth with OpenCode too.** Claude Code profiles already take a model per depth (Settings → Explanations); an OpenCode profile always uses its own model, so every run above was Opus. The first spike found Sonnet about half the time and a third of the cost. Brief could use a smaller model still.
3. **Less to write** (about 10 to 20 s). The write is mostly output tokens: ask for the shortest quote that pins a claim, cap the notes for each depth, and leave out the fields a depth does not use.
4. **Less to read** (about 10 s). Put the change's stat and its patch, up to a size, in a file in the container (as the known concepts are) or in the prompt, which replaces the first few round trips.
5. **Show it as it is written** (no shorter, but the first part is seen sooner). The agent writes the summary and the reading order first, then the notes, and the panel shows each part as it lands. The largest change: Brainiac reads the file only once, after the run, and the checks would run on each part.
6. **A direct model call without an agent.** The largest gain for a small change; it is the 0.6.x follow-up above, with its doubts (fewer sources cited, so thinner notes).

Suggested first: 1 and 2, small and safe, which together may bring Teach me from about 90 s to 40 or 50 s. That is an estimate until the same commits are run again, comparing time, cost, follow-up turns, and notes dropped by the checks. Then 3.

## Open questions

- How long an explanation takes and costs on each agent, and the time limit to default to (the spike; Claude Code's first round is above, OpenCode is still to measure).
- Whether an explain run needs a smaller workspace than a coding run (it uses the profile's for now).
- Whether the panel and the canvas's Working state hold up with real progress from the journal.
- A shared answer for a team (which repositories may reach which provider), set once rather than on each developer's Mac. Brainiac has one user per Mac; this belongs with shared workspaces (`docs/design/shared-workspaces.md`).
- Whether the estimate from the last ten explanations is close enough to trust before Explain, once real runs in a container are timed.
- Whether the panel, with the summary first and Tour open, reads well on a large change, or should start folded. (9 October 2026: the Tour tab went; the numbered file list is the tour, and the panel names the selected file's step and role. `SPEC.md`, section 14.)
- The cap on the known-concepts file and what it adds to the dialog's estimate (the file, its filter, and the checker's backstop are in Editing the ledger, and scope; the cap and whether both agents use the lookup are for the spike).
- Whether the ledger is part of export and restore, and what deleting an explanation leaves of what it taught.
- Whether the explain run's container start is a large share of an explanation's time, once a run in a container is timed (the spike left it out). It decides whether a warm container or a host-side agent is worth anything. (11 October 2026: about 3 s of a 90 to 110 s run, so neither is worth building; Speed.)
- Whether a local model can write a valid schema at all, and how a repository's answer is recorded for something with no provider.
- What happens to aliases when a concept is forgotten, exported, or restored. Descriptions from another repository do not go into an explain run; the words included are from explanations of this repository.
- What the user sees when a concept's scope changes: which repositories' prompts it now reaches or leaves.
- Where "left out N times" is counted and what resets it.
- Whether a pattern shared by several repositories is needed at all, and whether its words may go to the others (Shared project patterns).
